// Commands added for Task Manager v2: merged list, why, restart, free, anomalies, history.
// Included into wtm.rs with include!, so it shares its imports and helpers.

fn ram_total() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe {
        let _ = GlobalMemoryStatusEx(&mut m);
    }
    m.ullTotalPhys
}

fn imp(p: &Proc, ram: u64) -> wt::insight::Impact {
    wt::insight::impact(p.cpu, p.mem, p.io_rate, 0.0, ram)
}

fn cmdlines(s: &Sampler) -> std::collections::HashMap<u32, String> {
    s.procs.iter().filter_map(|p| procs::command_line(p.pid).map(|c| (p.pid, c))).collect()
}

fn svc_map() -> std::collections::HashMap<u32, Vec<String>> {
    let mut m: std::collections::HashMap<u32, Vec<String>> = Default::default();
    for x in services::list() {
        if x.pid != 0 {
            m.entry(x.pid).or_default().push(x.display);
        }
    }
    m
}

fn round1(v: f32) -> f64 {
    (v as f64 * 10.0).round() / 10.0
}

fn cmd_merged(args: &[String], s: &Sampler) -> R {
    use wt::insight::{Group, impact, merge};
    let ram = ram_total();
    let mut groups = merge(&s.procs, &cmdlines(s), &svc_map());
    let score = |g: &Group| impact(g.cpu, g.mem, g.io, 0.0, ram);
    groups.sort_by(|a, b| score(b).score.total_cmp(&score(a).score));
    let total_mem: u64 = groups.iter().map(|g| g.mem).sum();
    let n = opt(args, "--top").and_then(|n| n.parse::<usize>().ok()).unwrap_or(groups.len());
    let (shown, rest) = groups.split_at(n.min(groups.len()));
    let mut text = format!("{:<44} {:>6} {:>9} {:>4}  {:<10} {}\n", "NAME", "CPU%", "MEMORY", "N", "KIND", "RANKED BY");
    for g in shown {
        let i = score(g);
        text += &format!("{:<44} {:>6.1} {:>9} {:>4}  {:<10} {}\n", trunc(&g.label, 44), g.cpu, fmt_bytes(g.mem as f64), g.pids.len(), g.kind.as_str(), i.why);
        if g.members.len() > 1 {
            for (pid, lab, cpu, mem) in g.members.iter().take(4) {
                text += &format!("    {:<40} {:>6.1} {:>9}       pid {pid}\n", trunc(if lab.is_empty() { "-" } else { lab }, 40), cpu, fmt_bytes(*mem as f64));
            }
            if g.members.len() > 4 {
                let m: u64 = g.members[4..].iter().map(|x| x.3).sum();
                text += &format!("    {:<40} {:>6} {:>9}\n", format!("other {} processes", g.members.len() - 4), "", fmt_bytes(m as f64));
            }
        }
    }
    let rest_mem: u64 = rest.iter().map(|g| g.mem).sum();
    if !rest.is_empty() {
        text += &format!("{:<44} {:>6} {:>9} {:>4}\n", format!("other {} rows", rest.len()), "", fmt_bytes(rest_mem as f64), rest.iter().map(|g| g.pids.len()).sum::<usize>());
    }
    text += &format!("all processes together: {} private memory\n", fmt_bytes(total_mem as f64));
    let groups_json: Vec<Value> = shown
        .iter()
        .map(|g| {
            let i = score(g);
            json!({"label": g.label, "kind": g.kind.as_str(), "pids": g.pids, "cpu": round1(g.cpu), "memory": g.mem, "io_per_sec": g.io.round(),
                "impact": i.score, "ranked_by": i.driver, "why": i.why,
                "members": g.members.iter().map(|(pid, lab, cpu, mem)| json!({"pid": pid, "label": lab, "cpu": round1(*cpu), "memory": mem})).collect::<Vec<_>>()})
        })
        .collect();
    let v = json!({"ok": true, "total_memory": total_mem, "other_rows": {"count": rest.len(), "memory": rest_mem}, "groups": groups_json});
    Ok(Out { v, text })
}

fn cmd_why(target: Option<&String>) -> R {
    let target = target.ok_or_else(|| usage("why needs a PID or name"))?;
    let s = quick();
    let list = resolve(&s, target)?;
    let p = list[0];
    let signed = if p.meta.path.is_empty() { None } else { Some(explain::is_signed(&p.meta.path)) };
    let w = wt::insight::why(p, &s.procs, &services::list(), &startup::list_all(), signed);
    let join = |v: &Vec<String>| if v.is_empty() { "-".to_string() } else { v.join("; ") };
    let chain = if w.parents.is_empty() { "-".into() } else { w.parents.iter().map(|(pid, n)| format!("{n} ({pid})")).collect::<Vec<_>>().join(" <- ") };
    let text = format!(
        "{} (PID {})\n  started by  {}\n  startup     {}\n  task        {}\n  service     {}\n  flags       {}\n",
        p.name, p.pid, chain, join(&w.startup), join(&w.tasks), join(&w.services), join(&w.flags)
    );
    let v = json!({"ok": true, "pid": p.pid, "name": p.name, "parents": w.parents.iter().map(|(pid, n)| json!({"pid": pid, "name": n})).collect::<Vec<_>>(),
        "startup": w.startup, "tasks": w.tasks, "services": w.services, "flags": w.flags});
    Ok(Out { v, text })
}

fn cmd_restart(target: Option<&String>) -> R {
    let pid: u32 = target.and_then(|p| p.parse().ok()).ok_or_else(|| usage("restart needs a PID"))?;
    let new = wt::insight::restart(pid).map_err(|e| (1, e))?;
    Ok(Out { v: json!({"ok": true, "old_pid": pid, "new_pid": new}), text: format!("restarted: PID {pid} -> {new}\n") })
}

fn sampler_growing() -> Vec<u32> {
    let Ok(r) = wt::ipc::send_to("history", "anomalies") else { return vec![] };
    let v: Value = serde_json::from_str(&r).unwrap_or(json!({}));
    v["anomalies"].as_array().cloned().unwrap_or_default().iter().filter(|a| a["kind"] == "memory_growing").filter_map(|a| a["pid"].as_u64().map(|p| p as u32)).collect()
}

fn cmd_free(args: &[String]) -> R {
    use wt::insight::FreeVerdict;
    let s = sample(args);
    let groups = wt::insight::merge(&s.procs, &cmdlines(&s), &svc_map());
    let items = wt::insight::free_candidates(&groups, &s.procs, &sampler_growing(), &startup::list_all());
    let mut text = format!("{:<40} {:>9}  {:<15} {}\n", "NAME", "MEMORY", "VERDICT", "IF ENDED");
    let mut freeable = 0u64;
    let verdict = |v: FreeVerdict| match v {
        FreeVerdict::Safe => "safe",
        FreeVerdict::SafeButStops => "safe_but_stops",
        FreeVerdict::Keep => "keep",
    };
    for i in &items {
        if i.verdict != FreeVerdict::Keep {
            freeable += i.mem;
        }
        let label = match i.verdict {
            FreeVerdict::Safe => "safe",
            FreeVerdict::SafeButStops => "safe, but",
            FreeVerdict::Keep => "better to keep",
        };
        let extra = format!(
            "{}{}",
            if i.restart_hint { "; still growing: restart it to get the memory back" } else { "" },
            i.startup_entry.as_ref().map(|e| format!("; starts with Windows ({e})")).unwrap_or_default()
        );
        text += &format!("{:<40} {:>9}  {:<15} {}{}\n", trunc(&i.label, 40), fmt_bytes(i.mem as f64), label, i.effect, extra);
    }
    text += &format!("up to {} can be freed\n", fmt_bytes(freeable as f64));
    let items_json: Vec<Value> = items
        .iter()
        .map(|i| json!({"label": i.label, "pids": i.pids, "memory": i.mem, "verdict": verdict(i.verdict), "effect": i.effect, "restart_to_reclaim": i.restart_hint, "startup_entry": i.startup_entry}))
        .collect();
    Ok(Out { v: json!({"ok": true, "freeable": freeable, "items": items_json}), text })
}

fn cmd_anomalies(args: &[String]) -> R {
    let s = sample(args);
    let now = wt::history::anomalies_now(&s.procs);
    let mut list: Vec<Value> = now.iter().filter(|a| a.kind == "not_responding").map(|a| a.json()).collect();
    let source = match wt::ipc::send_to("history", "anomalies") {
        Ok(r) => {
            let v: Value = serde_json::from_str(&r).unwrap_or(json!({}));
            list.extend(v["anomalies"].as_array().cloned().unwrap_or_default());
            format!("sampler ({} min recorded)", v["minutes_recorded"])
        }
        Err(_) => {
            list.extend(now.iter().filter(|a| a.kind != "not_responding").map(|a| a.json()));
            "instant checks only (run `wtm sampler` for minutes-long history)".to_string()
        }
    };
    let mut text = String::new();
    for a in &list {
        text += &format!("{:>7}  {:<28} {}\n", a["pid"], trunc(a["name"].as_str().unwrap_or(""), 28), a["text"].as_str().unwrap_or(""));
    }
    if list.is_empty() {
        text += "nothing unusual\n";
    }
    text += &format!("source: {source}\n");
    Ok(Out { v: json!({"ok": true, "source": source, "anomalies": list}), text })
}

fn cmd_history(target: Option<&String>) -> R {
    let pid = target.ok_or_else(|| usage("history needs a PID (or 'cost')"))?;
    if pid == "cost" {
        let r = wt::ipc::send_to("history", "cost").map_err(|_| (1, "the sampler is not running (start: wtm sampler)".to_string()))?;
        let v: Value = serde_json::from_str(&r).unwrap_or(json!({"ok": false}));
        let text = format!(
            "sampler: {} samples, last sample {:.1} ms, total CPU {:.2} s, private {}, working set {}\n",
            v["samples"], v["last_sample_ms"].as_f64().unwrap_or(0.0), v["cpu_seconds_total"].as_f64().unwrap_or(0.0),
            fmt_bytes(v["private_bytes"].as_f64().unwrap_or(0.0)), fmt_bytes(v["working_set"].as_f64().unwrap_or(0.0))
        );
        return Ok(Out { v, text });
    }
    let r =wt::ipc::send_to("history", &format!("series {pid}")).map_err(|_| (1, "the sampler is not running (start: wtm sampler)".to_string()))?;
    let v: Value = serde_json::from_str(&r).unwrap_or(json!({"ok": false}));
    let pts = v["points"].as_array().cloned().unwrap_or_default();
    let mut text = format!("{} samples, every {} s\n", pts.len(), v["step_s"]);
    for p in pts.iter().rev().take(12).rev() {
        text += &format!("  t={:>5}s  cpu {:>5}%  mem {}\n", p[0], p[1], fmt_bytes(p[2].as_f64().unwrap_or(0.0)));
    }
    Ok(Out { v, text })
}

fn cmd_ai(action: Option<&String>, value: Option<&String>) -> R {
    use wt::ai;
    match (action.map(|s| s.as_str()), value.map(|s| s.as_str())) {
        (None | Some("status"), _) => {
            let v = ai::status();
            let text = format!(
                "AI features: {} (switch {}, key {}), Google Gemini {}, answers in {}\n  proxy   {}\n  get a free key: {}\n  sent    {}\n  meaning search: {} (install: {})\n",
                if v["enabled"].as_bool().unwrap_or(false) { "ON" } else { "off" },
                if v["switch"].as_bool().unwrap_or(false) { "on" } else { "off" },
                if v["key_saved"].as_bool().unwrap_or(false) { "saved (encrypted)" } else { "not set" },
                v["model"].as_str().unwrap_or(""), v["lang"].as_str().unwrap_or(""), v["proxy"].as_str().unwrap_or(""),
                v["get_key_url"].as_str().unwrap_or(""), v["sent"].as_str().unwrap_or(""),
                if v["semsearch"]["enabled"].as_bool().unwrap_or(false) { "on" } else { "off" }, v["semsearch"]["installed"].as_str().unwrap_or("none")
            );
            Ok(Out { v, text })
        }
        (Some(a @ ("on" | "off")), _) => {
            ai::set_enabled(a == "on").map_err(|e| (1, e))?;
            Ok(Out { v: json!({"ok": true, "switch": a == "on"}), text: format!("AI features {a}\n") })
        }
        (Some("key"), Some("set")) => {
            // the key is read from stdin so it never shows in the command line or shell history
            let mut k = String::new();
            std::io::stdin().read_line(&mut k).map_err(|e| (1, e.to_string()))?;
            ai::set_key(&k).map_err(|e| (1, e))?;
            Ok(Out { v: json!({"ok": true, "key_saved": true}), text: "API key saved (encrypted for this Windows user)\n".into() })
        }
        (Some("key"), Some("test")) => {
            ai::test_key(None).map_err(|e| (1, e))?;
            Ok(Out { v: json!({"ok": true, "key_works": true}), text: "the key works\n".into() })
        }
        (Some("key"), Some("clear")) => {
            ai::clear_key().map_err(|e| (1, e))?;
            Ok(Out { v: json!({"ok": true, "key_saved": false}), text: "API key removed; AI features off\n".into() })
        }
        (Some("lang"), Some(l @ ("en" | "fa"))) => {
            ai::set_lang(l).map_err(|e| (1, e))?;
            Ok(Out { v: json!({"ok": true, "lang": l}), text: format!("AI answer language: {l}\n") })
        }
        (Some("semsearch"), v) => {
            match v {
                Some(a @ ("on" | "off")) => {
                    if a == "on" && wt::models::semsearch_detect().is_none() && !wt::models::ready(wt::models::Section::Semsearch) {
                        return Err((1, "meaning search is not installed and its model is not downloaded".into()));
                    }
                    wt::models::set_switch(wt::models::Section::Semsearch, a == "on").map_err(|e| (1, e))?;
                }
                None | Some("status") => {}
                Some(o) => return Err(usage(&format!("ai semsearch on|off|status, not '{o}'"))),
            }
            let det = wt::models::semsearch_detect().map(|p| p.display().to_string());
            let on = wt::models::semsearch_enabled();
            Ok(Out { v: json!({"ok": true, "enabled": on, "installed": det}), text: format!("meaning search: {} (install: {})\n", if on { "on" } else { "off" }, det.as_deref().unwrap_or("none")) })
        }
        _ => Err(usage("ai [status] | ai on|off | ai key set|test|clear (set reads the key from stdin) | ai lang en|fa | ai semsearch on|off")),
    }
}
