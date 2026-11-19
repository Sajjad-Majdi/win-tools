//! wtm: Rust Task Manager on the command line. Every action of the window is here, so an agent or script never
//! needs the GUI. Output is a readable table by default and JSON with --json. Exit code 0 = ok, 1 = error, 2 = usage.
use serde_json::{Value, json};
use std::time::Duration;
use wt::explain::{self, Verdict};
use wt::procs::{self, Proc, Sampler, fmt_bytes};
use wt::{perf, services, startup};

const HELP: &str = "\
wtm - Rust Task Manager CLI (for agents and scripts)

USAGE: wtm <command> [args] [--json]

Processes
  list [--sort impact|cpu|mem|io|name|pid] [--top N] [--kind app|background|windows] [--interval MS]
  list --merge [--top N]             same-name processes as one row (children named by their script / role)
  find <words...> [--top N]          search name, window title, description, company, PID, path, command line
  info <pid|name>                    details: path, command line, parent, children, memory, services inside
  tree [pid]                         process tree (all, or below one PID)
  kill <pid|name> [--tree]           end the process (a name ends every process with that exact name)
  close <pid|name>                   ask the app to close its windows (like clicking X; it may ask to save)
  priority <pid> <idle|below|normal|above|high|realtime>
  explain <pid|name> [--ai]          what is it, who made it, is it safe to end (--ai asks Gemini for unknown ones)
  why <pid|name>                     why is it running: parents, startup entry, scheduled task, service, flags
  restart <pid>                      end it and start it again with the same command line
  free                               what can be ended to free memory, and what stops if you do
  anomalies                          not responding, high CPU for minutes, memory still growing
  sampler                            run the optional history sampler (every 10 s, 30 min kept)
  history <pid|cost>                 CPU/memory history of one process, or the sampler's own cost
  ai [status]                        AI features (Google Gemini with your own key; off by default)
  ai on|off | ai lang en|fa | ai key set|test|clear   (key set reads the key from stdin; stored encrypted)
  ai semsearch on|off                meaning search (uses an existing semsearch install)

System
  perf [--gpu] [--interval MS]       CPU (total + cores), memory, disks, network, GPU
  services [words...] [--running]    list services (filter by words)
  service <start|stop|restart|info> <name>
  startup                            startup apps, scheduled tasks at logon and third-party auto services
  startup <enable|disable> <name>
  default                            is Rust Task Manager the default Task Manager? (read only; the window has the button)

Window (the open Rust Task Manager window)
  gui <command>                      drive the open window: show, search WORDS, select PID, expand, tab NAME,
                                     pane NAME, flyout NAME, sort, theme light|dark, state, shot FILE.bmp, quit, ...

Notes
  --json gives machine-readable output on every command. list/find answer at once: CPU % is the sampler's last
  value when `wtm sampler` runs, else the average over each process's life; add --interval MS to measure live.
  perf waits --interval ms (default 500). Without admin rights, other users' and protected processes show no path, and
  many services cannot be started or stopped.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json_out = args.iter().any(|a| a == "--json");
    let res = run(&args);
    match res {
        Ok(Out { v, text }) => {
            if json_out {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                print!("{text}");
            }
        }
        Err((code, msg)) => {
            if json_out {
                println!("{}", json!({"ok": false, "error": msg}));
            } else {
                eprintln!("error: {msg}");
            }
            std::process::exit(code);
        }
    }
}

struct Out {
    v: Value,
    text: String,
}

type R = Result<Out, (i32, String)>;

fn usage(msg: &str) -> (i32, String) {
    (2, format!("{msg} (see: wtm help)"))
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// positional args: drop --flags and their values
fn positional(args: &[String]) -> Vec<String> {
    let with_value = ["--sort", "--top", "--kind", "--interval"];
    let mut out = vec![];
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if with_value.contains(&a.as_str()) {
            skip = true;
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        out.push(a.clone());
    }
    out
}

/// One instant snapshot. CPU % = the sampler's last 10-s value when `wtm sampler` runs, else the average over each
/// process's life. `--interval MS` measures live instead (two samples MS apart).
fn sample(args: &[String]) -> Sampler {
    let mut s = Sampler::new();
    s.refresh(true);
    if let Some(ms) = opt(args, "--interval").and_then(|v| v.parse::<u64>().ok()).filter(|ms| *ms > 0) {
        std::thread::sleep(Duration::from_millis(ms));
        s.refresh(true);
    } else if let Ok(r) = wt::ipc::send_to("history", "latest")
        && let Ok(v) = serde_json::from_str::<Value>(&r)
        && let Some(m) = v["cpu"].as_object()
    {
        for p in &mut s.procs {
            if let Some(c) = m.get(&p.pid.to_string()).and_then(|c| c.as_f64()) {
                p.cpu = c as f32;
            }
        }
        if let Some(t) = v["total"].as_f64() {
            s.total_cpu = t as f32;
        }
    }
    s.fill_desc(usize::MAX);
    s
}

fn quick() -> Sampler {
    let mut s = Sampler::new();
    s.refresh(true);
    s.fill_desc(usize::MAX);
    s
}

fn proc_json(p: &Proc) -> Value {
    json!({
        "pid": p.pid, "ppid": p.ppid, "name": p.name, "kind": p.kind.as_str(),
        "cpu": (p.cpu as f64 * 10.0).round() / 10.0, "memory": p.mem, "working_set": p.ws, "commit": p.commit,
        "io_per_sec": p.io_rate.round(), "threads": p.threads, "handles": p.handles, "session": p.session,
        "title": p.title, "path": p.meta.path, "description": p.meta.description, "company": p.meta.company,
        "started_unix": filetime_unix(p.create_time),
    })
}

fn filetime_unix(ft: i64) -> i64 {
    if ft <= 0 { 0 } else { ft / 10_000_000 - 11_644_473_600 }
}

fn table(rows: &[&Proc]) -> String {
    let mut t = format!("{:>7}  {:<28} {:>6} {:>9} {:>9}  {:<10} {}\n", "PID", "NAME", "CPU%", "MEMORY", "IO/s", "KIND", "TITLE / DESCRIPTION");
    for p in rows {
        let extra = if !p.title.is_empty() { p.title.as_str() } else { p.meta.description.as_str() };
        t += &format!("{:>7}  {:<28} {:>6.1} {:>9} {:>9}  {:<10} {}\n", p.pid, trunc(&p.name, 28), p.cpu, fmt_bytes(p.mem as f64), fmt_bytes(p.io_rate), p.kind.as_str(), trunc(extra, 60));
    }
    t
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

/// pid or exact (case-insensitive) name; ".exe" may be left out
fn resolve<'a>(s: &'a Sampler, target: &str) -> Result<Vec<&'a Proc>, (i32, String)> {
    if let Ok(pid) = target.parse::<u32>() {
        return s.procs.iter().find(|p| p.pid == pid).map(|p| vec![p]).ok_or((1, format!("no process with PID {pid}")));
    }
    let t = target.to_lowercase();
    let t2 = if t.ends_with(".exe") { t.clone() } else { format!("{t}.exe") };
    let v: Vec<&Proc> = s.procs.iter().filter(|p| {
        let n = p.name.to_lowercase();
        n == t || n == t2
    }).collect();
    if v.is_empty() { Err((1, format!("no process named '{target}' (try: wtm find {target})"))) } else { Ok(v) }
}

fn run(args: &[String]) -> R {
    let pos = positional(args);
    let cmd = pos.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "help" | "-h" | "--help" => Ok(Out { v: json!({"help": HELP}), text: HELP.into() }),
        "list" | "ls" | "ps" => cmd_list(args),
        "find" | "search" => cmd_find(args, &pos[1..]),
        "info" => cmd_info(args, pos.get(1)),
        "tree" => cmd_tree(pos.get(1)),
        "kill" | "end" => cmd_kill(args, pos.get(1)),
        "close" => cmd_close(pos.get(1)),
        "priority" => cmd_priority(pos.get(1), pos.get(2)),
        "explain" | "what" => cmd_explain(args, pos.get(1)),
        "perf" => cmd_perf(args),
        "services" => cmd_services(args, &pos[1..]),
        "service" => cmd_service(pos.get(1), pos.get(2)),
        "startup" => cmd_startup(pos.get(1), pos.get(2)),
        "default" => cmd_default(),
        "gui" => cmd_gui(&pos[1..]),
        "why" => cmd_why(pos.get(1)),
        "restart" => cmd_restart(pos.get(1)),
        "free" => cmd_free(args),
        "anomalies" => cmd_anomalies(args),
        "sampler" => wt::history::run_sampler(),
        "history" => cmd_history(pos.get(1)),
        "ai" => cmd_ai(pos.get(1), pos.get(2)),
        other => Err(usage(&format!("unknown command '{other}'"))),
    }
}

fn sort_procs(v: &mut [&Proc], by: &str) {
    match by {
        "mem" | "memory" => v.sort_by(|a, b| b.mem.cmp(&a.mem)),
        "io" | "disk" => v.sort_by(|a, b| b.io_rate.total_cmp(&a.io_rate)),
        "name" => v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "pid" => v.sort_by_key(|p| p.pid),
        "impact" => {
            let ram = ram_total();
            v.sort_by(|a, b| imp(b, ram).score.total_cmp(&imp(a, ram).score))
        }
        _ => v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem))),
    }
}

fn cmd_list(args: &[String]) -> R {
    let s = sample(args);
    if flag(args, "--merge") {
        return cmd_merged(args, &s);
    }
    let mut v: Vec<&Proc> = s.procs.iter().filter(|p| p.pid != 0).collect();
    if let Some(k) = opt(args, "--kind") {
        v.retain(|p| p.kind.as_str() == k);
    }
    sort_procs(&mut v, opt(args, "--sort").unwrap_or("cpu"));
    let total = v.len();
    if let Some(n) = opt(args, "--top").and_then(|n| n.parse::<usize>().ok()) {
        v.truncate(n);
    }
    let mut text = table(&v);
    text += &format!("{total} processes, CPU {:.0}%\n", s.total_cpu);
    Ok(Out { v: json!({"ok": true, "cpu_total": (s.total_cpu as f64 * 10.0).round() / 10.0, "count": total, "processes": v.iter().map(|p| proc_json(p)).collect::<Vec<_>>()}), text })
}

fn cmd_find(args: &[String], words: &[String]) -> R {
    if words.is_empty() {
        return Err(usage("find needs words"));
    }
    let s = sample(args);
    let toks = procs::tokens(&words.join(" "));
    let mut hits: Vec<(u32, &Proc, Option<String>)> = vec![];
    let me = std::process::id();
    for p in s.procs.iter().filter(|p| p.pid != 0 && p.pid != me) {
        if let Some(r) = procs::match_rank(p, &toks, None) {
            hits.push((r, p, None));
        } else {
            let cl = procs::command_line(p.pid);
            if let Some(r) = procs::match_rank(p, &toks, cl.as_deref()) {
                hits.push((r, p, cl));
            }
        }
    }
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.mem.cmp(&a.1.mem)));
    if let Some(n) = opt(args, "--top").and_then(|n| n.parse::<usize>().ok()) {
        hits.truncate(n);
    }
    let rows: Vec<&Proc> = hits.iter().map(|h| h.1).collect();
    let text = if rows.is_empty() { "no match\n".into() } else { table(&rows) };
    let v = json!({"ok": true, "query": words.join(" "), "count": hits.len(), "processes": hits.iter().map(|(_, p, cl)| {
        let mut j = proc_json(p);
        if let Some(c) = cl { j["command_line"] = json!(c); }
        j
    }).collect::<Vec<_>>()});
    Ok(Out { v, text })
}

fn cmd_info(args: &[String], target: Option<&String>) -> R {
    let target = target.ok_or_else(|| usage("info needs a PID or name"))?;
    let s = sample(args);
    let list = resolve(&s, target)?;
    let svcs = services::list();
    let mut text = String::new();
    let mut arr = vec![];
    for p in list {
        let cl = procs::command_line(p.pid).unwrap_or_default();
        let parent = s.procs.iter().find(|q| q.pid == p.ppid && q.create_time <= p.create_time);
        let children: Vec<&Proc> = s.procs.iter().filter(|q| q.ppid == p.pid && q.create_time >= p.create_time && q.pid != p.pid).collect();
        let in_svc: Vec<&services::Service> = svcs.iter().filter(|x| x.pid == p.pid && x.pid != 0).collect();
        let mut j = proc_json(p);
        j["command_line"] = json!(cl);
        j["parent"] = json!(parent.map(|q| json!({"pid": q.pid, "name": q.name})));
        j["children"] = json!(children.iter().map(|q| json!({"pid": q.pid, "name": q.name})).collect::<Vec<_>>());
        j["services"] = json!(in_svc.iter().map(|x| json!({"name": x.name, "display": x.display})).collect::<Vec<_>>());
        text += &format!(
            "{} (PID {})\n  kind        {}\n  CPU         {:.1}%\n  memory      {} private, {} working set, {} commit\n  disk I/O    {}/s\n  threads     {}   handles {}   session {}\n  title       {}\n  description {}\n  company     {}\n  path        {}\n  command     {}\n  parent      {}\n  children    {}\n  services    {}\n\n",
            p.name, p.pid, p.kind.as_str(), p.cpu, fmt_bytes(p.mem as f64), fmt_bytes(p.ws as f64), fmt_bytes(p.commit as f64), fmt_bytes(p.io_rate),
            p.threads, p.handles, p.session, p.title, p.meta.description, p.meta.company, p.meta.path, cl,
            parent.map(|q| format!("{} ({})", q.name, q.pid)).unwrap_or("-".into()),
            if children.is_empty() { "-".into() } else { children.iter().map(|q| format!("{} ({})", q.name, q.pid)).collect::<Vec<_>>().join(", ") },
            if in_svc.is_empty() { "-".into() } else { in_svc.iter().map(|x| x.name.clone()).collect::<Vec<_>>().join(", ") },
        );
        arr.push(j);
    }
    Ok(Out { v: json!({"ok": true, "processes": arr}), text })
}

fn cmd_tree(root: Option<&String>) -> R {
    let s = quick();
    let by_pid: std::collections::HashMap<u32, &Proc> = s.procs.iter().map(|p| (p.pid, p)).collect();
    let is_root = |p: &Proc| match by_pid.get(&p.ppid) {
        Some(par) => par.pid == p.pid || par.create_time > p.create_time,
        None => true,
    };
    fn walk(s: &Sampler, p: &Proc, depth: usize, text: &mut String) -> Value {
        *text += &format!("{}{} ({})  {}\n", "  ".repeat(depth), p.name, p.pid, fmt_bytes(p.mem as f64));
        let kids: Vec<Value> = s.procs.iter().filter(|q| q.ppid == p.pid && q.pid != p.pid && q.pid != 0 && q.create_time >= p.create_time).map(|q| walk(s, q, depth + 1, text)).collect();
        json!({"pid": p.pid, "name": p.name, "memory": p.mem, "children": kids})
    }
    let mut text = String::new();
    let roots: Vec<&Proc> = match root {
        Some(r) => resolve(&s, r)?,
        None => s.procs.iter().filter(|p| p.pid != 0 && is_root(p)).collect(),
    };
    let v: Vec<Value> = roots.iter().map(|p| walk(&s, p, 0, &mut text)).collect();
    Ok(Out { v: json!({"ok": true, "tree": v}), text })
}

fn cmd_kill(args: &[String], target: Option<&String>) -> R {
    let target = target.ok_or_else(|| usage("kill needs a PID or name"))?;
    let s = quick();
    let list = resolve(&s, target)?;
    let tree = flag(args, "--tree");
    let mut ended = vec![];
    let mut failed = vec![];
    for p in list {
        let mut pids = vec![p.pid];
        if tree {
            pids.extend(procs::descendants(&s.procs, p.pid));
        }
        for pid in pids {
            let name = s.procs.iter().find(|q| q.pid == pid).map(|q| q.name.clone()).unwrap_or_default();
            match procs::terminate(pid) {
                Ok(()) => ended.push(json!({"pid": pid, "name": name})),
                Err(e) => failed.push(json!({"pid": pid, "name": name, "error": e})),
            }
        }
    }
    let mut text = String::new();
    for e in &ended {
        text += &format!("ended {} ({})\n", e["name"].as_str().unwrap_or(""), e["pid"]);
    }
    for f in &failed {
        text += &format!("FAILED {} ({}): {}\n", f["name"].as_str().unwrap_or(""), f["pid"], f["error"].as_str().unwrap_or(""));
    }
    if ended.is_empty() && !failed.is_empty() {
        return Err((1, text.trim_end().replace('\n', "; ")));
    }
    Ok(Out { v: json!({"ok": failed.is_empty(), "ended": ended, "failed": failed}), text })
}

fn cmd_close(target: Option<&String>) -> R {
    let target = target.ok_or_else(|| usage("close needs a PID or name"))?;
    let s = quick();
    let list = resolve(&s, target)?;
    let mut n = 0;
    for p in &list {
        n += procs::close_windows(p.pid);
    }
    if n == 0 {
        return Err((1, "no visible window to close (use kill to end it)".into()));
    }
    Ok(Out { v: json!({"ok": true, "windows_asked": n}), text: format!("asked {n} window(s) to close\n") })
}

fn cmd_priority(pid: Option<&String>, level: Option<&String>) -> R {
    let pid: u32 = pid.and_then(|p| p.parse().ok()).ok_or_else(|| usage("priority needs a PID"))?;
    let level = level.ok_or_else(|| usage("priority needs a level"))?;
    procs::set_priority(pid, level).map_err(|e| (1, e))?;
    Ok(Out { v: json!({"ok": true, "pid": pid, "priority": level}), text: format!("PID {pid} priority set to {level}\n") })
}

fn cmd_explain(args: &[String], target: Option<&String>) -> R {
    let target = target.ok_or_else(|| usage("explain needs a PID or name"))?;
    let s = quick();
    let list = resolve(&s, target)?;
    let p = list[0];
    let signed = if p.meta.path.is_empty() { None } else { Some(explain::is_signed(&p.meta.path)) };
    let mut a = explain::explain(p, signed);
    let mut ai_error = None;
    if flag(args, "--ai") && a.source != "builtin" {
        let cl = procs::command_line(p.pid).unwrap_or_default();
        match wt::ai::ask(p, &cl, &a) {
            Ok(x) => a = x,
            Err(e) => ai_error = Some(e),
        }
    }
    let mut text = format!("{} (PID {}{})\n  verdict  {}\n  what     {}\n  maker    {}\n", p.name, p.pid, if list.len() > 1 { format!(", 1 of {}", list.len()) } else { String::new() }, a.verdict.label_en(), a.what_en, if a.maker.is_empty() { "-" } else { &a.maker });
    if let Some(sg) = signed {
        text += &format!("  signed   {}\n", if sg { "yes" } else { "NO" });
    }
    for n in &a.notes {
        text += &format!("  note     {n}\n");
    }
    if a.verdict == Verdict::Unknown && a.source != "ai" {
        text += "  tip      add --ai to ask the AI about it\n";
    }
    if let Some(e) = &ai_error {
        text += &format!("  ai       failed: {e}\n");
    }
    let v = json!({"ok": true, "pid": p.pid, "name": p.name, "verdict": a.verdict.as_str(), "what_en": a.what_en, "what_fa": a.what_fa, "maker": a.maker, "signed": signed, "notes": a.notes, "source": a.source, "ai_error": ai_error, "path": p.meta.path});
    Ok(Out { v, text })
}

fn cmd_perf(args: &[String]) -> R {
    let gpu = flag(args, "--gpu");
    let ms: u64 = opt(args, "--interval").and_then(|v| v.parse().ok()).unwrap_or(500);
    let mut pf = perf::Perf::new();
    pf.refresh(gpu);
    std::thread::sleep(Duration::from_millis(ms));
    pf.refresh(gpu);
    let s = &pf.snap;
    let used = s.mem_total - s.mem_avail;
    let mut text = format!(
        "CPU      {:.0}%  {}\n  cores  {}\nMemory   {} / {} ({:.0}%), committed {} / {}, cached {}\nSystem   {} processes, {} threads, {} handles, up {}h {}m\n",
        s.cpu, s.cpu_name, s.cores.iter().map(|c| format!("{c:.0}")).collect::<Vec<_>>().join(" "),
        fmt_bytes(used as f64), fmt_bytes(s.mem_total as f64), used as f64 / s.mem_total.max(1) as f64 * 100.0,
        fmt_bytes(s.commit_total as f64), fmt_bytes(s.commit_limit as f64), fmt_bytes(s.cached as f64),
        s.processes, s.threads, s.handles, s.uptime_s / 3600, s.uptime_s / 60 % 60,
    );
    for d in &s.disks {
        text += &format!("{:<8} {:.0}% active, read {}/s, write {}/s\n", d.name, d.active, fmt_bytes(d.read_rate), fmt_bytes(d.write_rate));
    }
    for n in &s.nets {
        text += &format!("{:<8} {} recv {}/s, send {}/s ({})\n", "Network", n.name, fmt_bytes(n.recv_rate), fmt_bytes(n.send_rate), n.desc);
    }
    if let Some(g) = &s.gpu {
        text += &format!("GPU      {:.0}% ({}), dedicated memory {}, shared {}\n", g.util, g.engines.iter().take(3).map(|e| format!("{} {:.0}%", e.0, e.1)).collect::<Vec<_>>().join(", "), fmt_bytes(g.dedicated_used as f64), fmt_bytes(g.shared_used as f64));
    }
    let v = json!({
        "ok": true, "cpu": s.cpu, "cpu_name": s.cpu_name, "cores": s.cores,
        "memory": {"total": s.mem_total, "available": s.mem_avail, "used": used, "commit": s.commit_total, "commit_limit": s.commit_limit, "cached": s.cached},
        "processes": s.processes, "threads": s.threads, "handles": s.handles, "uptime_s": s.uptime_s,
        "disks": s.disks.iter().map(|d| json!({"name": d.name, "active": d.active, "read_per_sec": d.read_rate.round(), "write_per_sec": d.write_rate.round()})).collect::<Vec<_>>(),
        "network": s.nets.iter().map(|n| json!({"name": n.name, "description": n.desc, "recv_per_sec": n.recv_rate.round(), "send_per_sec": n.send_rate.round(), "link_bps": n.link_bps})).collect::<Vec<_>>(),
        "gpu": s.gpu.as_ref().map(|g| json!({"util": g.util, "engines": g.engines, "dedicated_used": g.dedicated_used, "shared_used": g.shared_used})),
    });
    Ok(Out { v, text })
}

fn cmd_services(args: &[String], words: &[String]) -> R {
    let toks = procs::tokens(&words.join(" "));
    let running = flag(args, "--running");
    let list: Vec<services::Service> = services::list().into_iter().filter(|s| {
        let hay = format!("{} {}", s.name, s.display).to_lowercase();
        (!running || s.state == "running") && toks.iter().all(|t| hay.contains(t.as_str()))
    }).collect();
    let mut text = format!("{:<34} {:<9} {:>7}  {}\n", "NAME", "STATE", "PID", "DISPLAY NAME");
    for s in &list {
        text += &format!("{:<34} {:<9} {:>7}  {}\n", trunc(&s.name, 34), s.state, if s.pid == 0 { "-".into() } else { s.pid.to_string() }, s.display);
    }
    text += &format!("{} services\n", list.len());
    Ok(Out { v: json!({"ok": true, "count": list.len(), "services": list.iter().map(|s| json!({"name": s.name, "display": s.display, "state": s.state, "pid": s.pid})).collect::<Vec<_>>()}), text })
}

fn cmd_service(action: Option<&String>, name: Option<&String>) -> R {
    let action = action.ok_or_else(|| usage("service needs start|stop|restart|info"))?;
    let name = name.ok_or_else(|| usage("service needs a name"))?;
    if action == "info" {
        let s = services::list().into_iter().find(|s| s.name.eq_ignore_ascii_case(name)).ok_or((1, format!("no service '{name}'")))?;
        let (start, bin, desc) = services::details(&s.name).unwrap_or_default();
        let text = format!("{} ({})\n  state   {}\n  pid     {}\n  start   {}\n  binary  {}\n  about   {}\n", s.display, s.name, s.state, s.pid, start, bin, desc);
        return Ok(Out { v: json!({"ok": true, "name": s.name, "display": s.display, "state": s.state, "pid": s.pid, "start_type": start, "binary": bin, "description": desc}), text });
    }
    services::control(name, action).map_err(|e| (1, e))?;
    Ok(Out { v: json!({"ok": true, "service": name, "action": action}), text: format!("{action}: {name} done\n") })
}

fn cmd_startup(action: Option<&String>, name: Option<&String>) -> R {
    match action.map(|s| s.as_str()) {
        None | Some("list") => {
            let l = startup::list_all();
            let mut text = format!("{:<34} {:<8} {:<17} {}\n", "NAME", "ENABLED", "SOURCE", "COMMAND");
            for e in &l {
                text += &format!("{:<34} {:<8} {:<17} {}\n", trunc(&e.name, 34), if e.enabled { "yes" } else { "no" }, e.source, trunc(&e.command, 70));
            }
            Ok(Out { v: json!({"ok": true, "entries": l.iter().map(|e| json!({"name": e.name, "enabled": e.enabled, "source": e.source, "command": e.command})).collect::<Vec<_>>()}), text })
        }
        Some(a @ ("enable" | "disable")) => {
            let name = name.ok_or_else(|| usage("startup enable/disable needs a name"))?;
            let e = startup::set_enabled(name, a == "enable").map_err(|e| (1, e))?;
            Ok(Out { v: json!({"ok": true, "name": e.name, "enabled": e.enabled, "source": e.source}), text: format!("{} {}d\n", e.name, a) })
        }
        Some(o) => Err(usage(&format!("unknown startup action '{o}'"))),
    }
}

fn cmd_default() -> R {
    let ifeo = wt::reg::read_string(wt::reg::HKLM, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe", "Debugger");
    let ours = ifeo.as_deref().map(|d| { let d = d.to_lowercase(); d.contains("rusttaskmanager") || d.contains("wintask") }).unwrap_or(false);
    let text = format!("default Task Manager: {}\n", if ours { "Rust Task Manager" } else { "Windows" });
    Ok(Out { v: json!({"ok": true, "win_tools_is_default": ours, "debugger": ifeo}), text })
}

fn cmd_gui(rest: &[String]) -> R {
    if rest.is_empty() {
        return Err(usage("gui needs show|search|select|tab|quit"));
    }
    let line = rest.join(" ");
    match wt::ipc::send(&line) {
        Ok(reply) => Ok(Out { v: serde_json::from_str(&reply).unwrap_or(json!({"ok": true, "reply": reply})), text: format!("{reply}\n") }),
        Err(e) => Err((1, e)),
    }
}

include!("../wtm_more.rs");
