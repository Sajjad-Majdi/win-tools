//! wfm: Rust File Explorer on the command line. Every file-manager action is here, so an agent or script never needs
//! the GUI. Output is a readable table by default and one-line JSON with --json. Exit code 0 = ok, 1 = error,
//! 2 = usage.
use fmcore::list::SortKey;
use fmcore::ops::{self, Conflict, Control, Progress};
use fmcore::util::*;
use fmcore::{cleanup, dialog, info, journal, list, menu, namespace, pins, recycle, search, shell, thumb, watch, zip};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HELP: &str = "\
wfm - Rust File Explorer CLI (for agents and scripts)

USAGE: wfm <command> [args] [--json]

Browse
  ls <dir> [--all] [--sort name|size|date|created|type] [--desc] [--dirs-first] [--top N]
  info <path> [--no-index] [--limit MS]   size, dates, attributes, version (exe/dll), shortcut target,
                                          folder size (Everything index, else a walk of at most MS, default 3000)
  search <words...>                       same as find (Everything first)
  find <words...> [--in DIR] [--ext pdf,docx] [--size >10mb] [--modified 7d] [--folders|--files]
       [--top N] [--sort date-modified-descending] [--regex] [--match-path]
       [--engine auto|everything|walk] [--limit MS]
                                          Everything when it runs, else a walk of --in (or this folder)
                                          that stops after MS (default 5000); the output names the engine
  ls \\\\server                             the shared folders of a server (\\\\server\\share lists as usual)
  drives [--timeout MS]                   letter, label, type, free / total, mapped network drives
                                          (a drive that does not answer in MS, default 2000, is marked)
  recent [--top N]                        recently opened files and folders
  pins | pin <path> [--name N] | unpin <path or name>
  bin [--top N]                           the Recycle Bin: id, original path, deleted, size
  bin restore <id or original path>       put an item back (undo: wfm undo)
  bin empty --yes                         empty the Recycle Bin for good (needs --yes)
  cleanup --scan                          read-only cleanup groups and sizes (uses Everything)
  devices                                 phones and cameras (MTP, no drive letter)
  shell-ls <parsing name or path>         browse a phone folder (or any shell folder)
  shell-copy <parsing name...> <dest>     copy from (or to) a phone through the shell

Change (every change can be undone with `wfm undo`)
  copy <src...> <dest> [--conflict skip|replace|keep-both]     progress lines with --json
  move <src...> <dest> [--conflict skip|replace|keep-both]
  rename <path> <new name>
  batch-rename <dir> --pattern REGEX --to TEMPLATE [--dry-run] [--every] [--folders] [--hidden] [--keep-ext]
       TEMPLATE: $1..$9 groups, $0 whole match, {n} or {n:3} counter, {name} {ext} {date} {parent}
  delete <path...> [--permanent]          to the Recycle Bin unless --permanent
  mkdir <path>      new-file <path>
  zip <src...> <out.zip>      unzip <archive> <dir>
  undo [--list]                           reverse the last change (or list the journal)

Open
  open <path> [--verb VERB]               default app (VERB: edit, print, runas, ...)
  open <file> --neighbors [--desc]        picture/video/song: the app also gets the folder's others (arrow keys)
  open --select <path>                    like `explorer /select,`: prints the folder and the item to select
  openwith <path> [--all]                 the apps that can open it (the Windows default is marked)
  openwith <path> <app>                   open it with one of them (also: --app NAME; old name: open-with)
  openwith default <.ext>                 the Windows default app for a type
  reveal <path>                           show it selected in a folder window
  menu <path...> [--extended]             the right-click menu, with app extensions (7-Zip, Terminal, ...)
  menu <path...> --invoke VERB            run one entry (by verb or by its text)
  eject <drive>                           safely remove a USB stick, card or DVD

Live
  watch <dir> [--recursive] [--timeout SEC] [--count N]   JSON lines of changes until Ctrl+C
  thumb <path> --out <png> [--size 256] [--icon|--thumbnail-only]
  picture <path> [--out <png>] [--max N]  decode a picture as the viewer does (WIC, EXIF orientation applied):
                                          size, orientation, frames, time; --out writes the upright pixels

Agents: Save / Open dialogs
  dialog list [--desktop NAME] [--verbose]
  dialog save --path <full path> [--hwnd H] [--overwrite] [--desktop NAME]
       types the full path into the open Save dialog and presses Save

Window (RustFileExplorer.exe must be running; every command answers with one line of JSON)
  gui state                               tabs, panes, path, view, selection, overlays, timings
  gui open <path> | gui search <words> | gui select <name...> | gui select-all | gui clear
  gui tab new [path] | tab close [N] | tab N        gui view details|list|grid     gui split on|off
  gui pane 0|1 | gui preview | gui menu | gui whatis | gui rename | gui settings | gui palette [text]
  gui transfers | gui copy | gui cut | gui paste | gui delete | gui undo | gui newfolder <name>
  gui key <keys e.g. ctrl+shift+n> | gui keys <text> | gui type <text> | gui look [key value]
  gui click X Y [ctrl|shift] | gui rclick X Y | gui dblclick X Y [shift] | gui hover X Y | gui wheel X Y N
  gui theme light|dark|system | gui size W H | gui shot <file.png> | gui quit
  gui shot-real <file.png>                capture the real window as Windows composes it (PrintWindow);
                                          start it with --offscreen so nothing shows on the screen
  gui capture <file.png>                  the window as DWM composes it WITH its backdrop and alpha
                                          (Windows.Graphics.Capture); prints the alpha of sample pixels
  gui drag X0 Y0 X1 Y1 [ctrl|shift]       a mouse drag sent to the window only (no real cursor); dbl starts
                                          it with a double-click (word selection in a text box)
  gui look [key value]                    show or change a Settings > Personalize value
  gui view-image <path>                   open the picture viewer (the list's pictures, or the folder's by name)
  gui view-state | view-key K | view-post K | view-click X Y | view-dblclick X Y | view-hover X Y
  gui view-wheel X Y N [ctrl] | view-drag X0 Y0 X1 Y1 | view-hits [f] | view-size W H | view-shot FILE [--hover]
  gui view-close                          (the viewer's state is also in gui state, key viewer)
  default status                          which parts open in Rust File Explorer: folders, drives, Win+E,
                                          This PC (set/restore only from Settings, or:
                                          default set --yes / default restore --yes; --root TESTKEY for tests)
  default probe --yes                     opens a test folder every common way (ShellExecute verbs, class
                                          Folder, explorer.exe, SHOpenFolderAndSelectItems, Control Panel ...)
                                          and reports which app got it; closes every window it opened
  props <path> [--set NAME --value TEXT]  file properties as Explorer's Details pane shows them (* = can be
                                          changed); --set changes one, e.g. --set System.Music.Artist
                                          --value 'A; B' (empty TEXT removes it)
  explorer                                the open Windows Explorer tabs: location, selection, active
  explorer check <folder> [--parent]      would RustFileExplorer.exe hand this folder back to the Explorer
                                          window in front? (--parent: also require Explorer as parent)
  ev <words...> [--top N]                 Everything IPC query (no es.exe), with timing

Notes
  --json gives one line of JSON on every command (copy/move/watch: one JSON object per line).
  Times in JSON are Unix seconds (UTC). Settings and the undo journal: %LOCALAPPDATA%\\WinTools.
";

/// Flags that take a value.
const WITH_VALUE: &[&str] = &[
    "--sort", "--top", "--in", "--ext", "--size", "--modified", "--conflict", "--pattern", "--to", "--name", "--verb", "--app", "--timeout", "--count",
    "--out", "--path", "--hwnd", "--desktop", "--limit", "--engine", "--invoke", "--select", "--set", "--value",
];

struct Out {
    v: Value,
    text: String,
}

type R = Result<Out, (i32, String)>;

fn usage(msg: &str) -> (i32, String) {
    (2, format!("{msg} (see: wfm help)"))
}

fn fail(e: String) -> (i32, String) {
    (1, e)
}

struct Args {
    all: Vec<String>,
    pos: Vec<String>,
    json: bool,
}

impl Args {
    fn parse() -> Args {
        let all: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
        let mut pos = vec![];
        let mut skip = false;
        for a in &all {
            if skip {
                skip = false;
                continue;
            }
            if WITH_VALUE.contains(&a.as_str()) {
                skip = true;
                continue;
            }
            if a.starts_with("--") && a.len() > 2 {
                continue;
            }
            pos.push(a.clone());
        }
        let json = all.iter().any(|a| a == "--json");
        Args { all, pos, json }
    }
    fn opt(&self, name: &str) -> Option<&str> {
        self.all.iter().position(|a| a == name).and_then(|i| self.all.get(i + 1)).map(|s| s.as_str())
    }
    fn flag(&self, name: &str) -> bool {
        self.all.iter().any(|a| a == name)
    }
    fn num(&self, name: &str) -> Result<Option<u64>, (i32, String)> {
        match self.opt(name) {
            None => Ok(None),
            Some(v) => v.parse().map(Some).map_err(|_| usage(&format!("{name} needs a number"))),
        }
    }
    fn arg(&self, i: usize, what: &str) -> Result<&str, (i32, String)> {
        self.pos.get(i).map(|s| s.as_str()).ok_or_else(|| usage(what))
    }
}

fn main() {
    let a = Args::parse();
    let res = run(&a);
    let mut so = std::io::stdout().lock();
    match res {
        Ok(Out { v, text }) => {
            // A command may hand over ready-made JSON text as a JSON string value (ls does, for speed).
            let _ = match (&v, a.json) {
                (Value::String(raw), true) => writeln!(so, "{raw}"),
                (_, true) => writeln!(so, "{}", serde_json::to_string(&v).unwrap()),
                _ => so.write_all(text.as_bytes()),
            };
            let _ = so.flush();
        }
        Err((code, msg)) => {
            if a.json {
                let _ = writeln!(so, "{}", json!({"ok": false, "error": msg}));
                let _ = so.flush();
            } else {
                eprintln!("error: {msg}");
            }
            std::process::exit(code);
        }
    }
}

fn run(a: &Args) -> R {
    let cmd = a.pos.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "gui" => cmd_gui(a),
        "default" => cmd_default(a),
        "explorer" => cmd_explorer(a),
        "props" => cmd_props(a),
        "ev" => cmd_ev(a),
        "ev-raw" => cmd_ev_raw(a),
        "help" | "-h" | "/?" => Ok(Out { v: json!({"ok": true, "help": HELP}), text: HELP.into() }),
        "ls" | "list" | "dir" => cmd_ls(a),
        "info" | "stat" => cmd_info(a),
        "find" | "search" => cmd_find(a),
        "drives" => cmd_drives(a),
        "recent" => cmd_recent(a),
        "pins" => cmd_pins(),
        "pin" => cmd_pin(a),
        "unpin" => cmd_unpin(a),
        "copy" | "cp" => cmd_transfer(a, false),
        "move" | "mv" => cmd_transfer(a, true),
        "rename" | "ren" => cmd_rename(a),
        "batch-rename" => cmd_batch(a),
        "delete" | "del" | "rm" => cmd_delete(a),
        "mkdir" | "md" => cmd_mkdir(a),
        "new-file" | "touch" => cmd_new_file(a),
        "zip" => cmd_zip(a),
        "unzip" => cmd_unzip(a),
        "undo" => cmd_undo(a),
        "open" => cmd_open(a),
        "open-with" | "openwith" => cmd_open_with(a),
        "reveal" => cmd_reveal(a),
        "watch" => cmd_watch(a),
        "thumb" => cmd_thumb(a),
        "picture" => cmd_picture(a),
        "dialog" => cmd_dialog(a),
        "bin" | "recycle-bin" => cmd_bin(a),
        "cleanup" => cmd_cleanup(a),
        "devices" => cmd_devices(),
        "shell-ls" => cmd_shell_ls(a),
        "shell-copy" => cmd_shell_copy(a),
        "menu" => cmd_menu(a),
        "menu-data" => cmd_menu_data(a),
        "icon" => cmd_icon(a),
        "eject" => cmd_eject(a),
        other => Err(usage(&format!("unknown command '{other}'"))),
    }
}

fn cmd_cleanup(a: &Args) -> R {
    if !a.flag("--scan") || a.pos.len() != 1 {
        return Err(usage("use: wfm cleanup --scan"));
    }
    let start = Instant::now();
    let groups = cleanup::scan(&Default::default(), &|_| {}).map_err(fail)?;
    let ms = start.elapsed().as_millis();
    let mut text = format!("Cleanup scan ({ms} ms)\n");
    let rows: Vec<_> = groups.iter().map(|g| {
        let count = if g.id == cleanup::GroupId::RecycleBin { None } else { Some(g.items.len()) };
        let suffix = if g.limited { " [partial]" } else { "" };
        let items = count.map(|n| format!("  {n} items")).unwrap_or_default();
        text += &format!("{:<18} {:>10}{items}{suffix}\n", g.title, fmt_bytes(g.total_size));
        json!({"id": g.id.as_str(), "title": g.title, "reason": g.reason, "size": g.total_size, "count": count, "limited": g.limited})
    }).collect();
    Ok(Out { v: json!({"ok": true, "ms": ms, "groups": rows}), text })
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

/// "\\server" (a server, no share)?
fn bare_server(p: &str) -> Option<&str> {
    let s = p.trim_end_matches('\\');
    let rest = s.strip_prefix(r"\\")?;
    if rest.is_empty() || rest.contains('\\') || rest.starts_with('?') || rest.starts_with('.') { None } else { Some(s) }
}

fn cmd_ls(a: &Args) -> R {
    let raw = a.pos.get(1).map(|s| s.as_str()).unwrap_or(".");
    if let Some(server) = bare_server(raw) {
        let shares = shell::server_shares(server, a.flag("--all")).map_err(fail)?;
        let mut t: String = shares.iter().map(|s| format!("{server}\\{s}\n")).collect();
        if shares.is_empty() {
            t = format!("{server} shares no folders (hidden admin shares like C$ show with --all)\n");
        }
        return Ok(Out { v: json!({"ok": true, "server": server, "shares": shares.iter().map(|s| json!({"name": s, "path": format!("{server}\\{s}")})).collect::<Vec<_>>()}), text: t });
    }
    let dir = abs(raw);
    let t0 = Instant::now();
    let mut v = list::list_dir(&dir, a.flag("--all")).map_err(fail)?;
    let key = match a.opt("--sort") {
        Some(s) => SortKey::parse(s).ok_or_else(|| usage("--sort takes name, size, date, created or type"))?,
        None => SortKey::Name,
    };
    list::sort_entries(&mut v, key, a.flag("--desc"), a.flag("--dirs-first"));
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let (bytes, files, dirs) = list::totals(&v);
    let total = v.len();
    if let Some(n) = a.num("--top")? {
        v.truncate(n as usize);
    }
    if a.json {
        // Written straight into one string (no JSON tree): about half the memory for big folders.
        let q = |s: &str| serde_json::to_string(s).unwrap();
        let mut o = String::with_capacity(v.len() * 130 + 256);
        o += &format!("{{\"ok\":true,\"dir\":{},\"count\":{total},\"files\":{files},\"folders\":{dirs},\"bytes\":{bytes},\"ms\":{},\"items\":[", q(&dir.to_string_lossy()), (ms * 100.0).round() / 100.0);
        for (i, e) in v.iter().enumerate() {
            if i > 0 {
                o.push(',');
            }
            o += &format!(
                "{{\"name\":{},\"dir\":{},\"type\":{},\"size\":{},\"modified\":{},\"created\":{},\"attributes\":\"{}\"}}",
                q(&e.name), e.is_dir(), q(&e.kind()), e.size, ft_unix(e.modified), ft_unix(e.created), attr_string(e.attrs)
            );
        }
        o += "]}";
        return Ok(Out { v: Value::String(o), text: String::new() });
    }
    let mut t = String::with_capacity(v.len() * 80);
    t += &format!("{:<16}  {:>10}  {:<5} {}\n", "MODIFIED", "SIZE", "ATTR", "NAME");
    for e in &v {
        let size = if e.is_dir() { "<DIR>".to_string() } else { fmt_bytes(e.size) };
        t += &format!("{:<16}  {:>10}  {:<5} {}{}\n", fmt_time(e.modified), size, attr_string(e.attrs & !ATTR_DIRECTORY), e.name, if e.is_dir() { "\\" } else { "" });
    }
    t += &format!("{} folders, {} files, {} in {} ({:.1} ms)\n", dirs, files, fmt_bytes(bytes), dir.display(), ms);
    Ok(Out { v: Value::Null, text: t })
}

fn cmd_info(a: &Args) -> R {
    let p = a.arg(1, "info needs a path")?;
    let limit = Duration::from_millis(a.num("--limit")?.unwrap_or(3000));
    let i = info::info(Path::new(p), Some(limit), !a.flag("--no-index")).map_err(fail)?;
    let ver = i.version.as_ref().map(|v| {
        json!({"file_version": v.file_version, "product_version": v.product_version, "description": v.description, "company": v.company, "product": v.product, "original_name": v.original_name, "copyright": v.copyright})
    });
    let sc = i.shortcut.as_ref().map(|s| json!({"target": s.target, "arguments": s.arguments, "work_dir": s.work_dir, "description": s.description}));
    let fo = i.folder.as_ref().map(|f| json!({"bytes": f.bytes, "files": f.files, "folders": f.folders, "source": f.source, "complete": f.complete, "ms": f.ms}));
    let v = json!({
        "ok": true, "path": i.path, "name": i.name, "dir": i.is_dir, "size": i.size, "type": i.type_name,
        "created": ft_unix(i.created), "modified": ft_unix(i.modified), "accessed": ft_unix(i.accessed),
        "attributes": attr_string(i.attrs), "attr_bits": i.attrs, "version": ver, "shortcut": sc, "link_target": i.link_target, "folder": fo,
    });
    let mut t = format!(
        "{}\n  type       {}\n  size       {} ({} bytes)\n  created    {}\n  modified   {}\n  accessed   {}\n  attributes {}\n",
        i.path, i.type_name, fmt_bytes(i.size), i.size, fmt_time(i.created), fmt_time(i.modified), fmt_time(i.accessed), attr_string(i.attrs)
    );
    if let Some(f) = &i.folder {
        t += &format!("  contents   {} files{}, from {}{} ({} ms)\n", f.files, if f.source == "walk" { format!(", {} folders", f.folders) } else { String::new() }, f.source, if f.complete { "" } else { " (time limit hit: at least this much)" }, f.ms);
    }
    if let Some(v) = &i.version {
        t += &format!("  version    {} (product {})\n  describes  {}\n  company    {}\n  product    {}\n  original   {}\n", v.file_version, v.product_version, v.description, v.company, v.product, v.original_name);
    }
    if let Some(s) = &i.shortcut {
        t += &format!("  target     {}\n", s.target);
        if !s.arguments.is_empty() {
            t += &format!("  arguments  {}\n", s.arguments);
        }
    }
    if let Some(l) = &i.link_target {
        t += &format!("  links to   {l}\n");
    }
    Ok(Out { v, text: t })
}

fn cmd_find(a: &Args) -> R {
    let words: Vec<String> = a.pos[1..].to_vec();
    if words.is_empty() && a.opt("--ext").is_none() && a.opt("--size").is_none() && a.opt("--modified").is_none() {
        return Err(usage("find needs words or a filter"));
    }
    let q = search::Query {
        words,
        in_dir: a.opt("--in").map(abs),
        exts: a.opt("--ext").map(|e| e.split([',', ';']).filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default(),
        size: a.opt("--size").map(String::from),
        modified: a.opt("--modified").map(String::from),
        folders: if a.flag("--folders") { Some(true) } else if a.flag("--files") { Some(false) } else { None },
        top: a.num("--top")?.unwrap_or(100) as usize,
        sort: a.opt("--sort").map(String::from),
        regex: a.flag("--regex"),
        match_path: a.flag("--match-path"),
        engine: match a.opt("--engine") {
            None | Some("auto") => search::Engine::Auto,
            Some("everything") => search::Engine::Everything,
            Some("walk") => search::Engine::Walk,
            Some(_) => return Err(usage("--engine takes auto, everything or walk")),
        },
        walk_limit_ms: a.num("--limit")?.unwrap_or(5000),
    };
    let t0 = Instant::now();
    let found = search::search(&q).map_err(fail)?;
    let hits = &found.hits;
    let ms = t0.elapsed().as_millis() as u64;
    let v = json!({"ok": true, "engine": found.engine, "complete": found.complete, "note": found.note, "count": hits.len(), "ms": ms, "results": hits.iter().map(|h| json!({
        "path": h.path, "name": h.name, "dir": h.is_dir, "size": h.size, "modified": ft_unix(h.modified), "created": ft_unix(h.created), "attributes": attr_string(h.attrs),
    })).collect::<Vec<_>>()});
    let mut t = String::new();
    for h in hits {
        let size = if h.is_dir { "<DIR>".into() } else { h.size.map(fmt_bytes).unwrap_or_default() };
        t += &format!("{:<16}  {:>10}  {}\n", fmt_time(h.modified), size, h.path);
    }
    let engine = if found.engine == "everything" { "Everything".to_string() } else { format!("folder walk of {}", q.in_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| abs(".").display().to_string())) };
    t += &format!("{} results ({} ms, engine: {engine}{})\n", hits.len(), ms, if found.complete { "" } else { ", stopped early: more may exist" });
    Ok(Out { v, text: t })
}

/// props <path> [--set NAME --value TEXT]: properties through the Windows property system.
fn cmd_props(a: &Args) -> R {
    let p = PathBuf::from(a.arg(1, "props needs a path")?);
    if let Some(k) = a.opt("--set") {
        fmcore::props::set(&p, k, a.opt("--value").unwrap_or("")).map_err(fail)?;
    }
    let v = fmcore::props::read(&p).map_err(fail)?;
    let mut t = String::new();
    for x in &v {
        t += &format!("{} {:<14} {}  ({})\n", if x.writable { "*" } else { " " }, x.label, x.value, x.key);
    }
    Ok(Out { v: json!({"ok": true, "props": v.iter().map(|x| json!({"key": x.key, "label": x.label, "value": x.value, "raw": x.raw, "writable": x.writable})).collect::<Vec<_>>()}), text: t })
}

/// explorer | explorer check <folder> [--parent]: Windows Explorer tabs (read only) and the hand-back decision.
fn cmd_explorer(a: &Args) -> R {
    if a.pos.get(1).map(|s| s.as_str()) == Some("find") {
        let p = a.arg(2, "explorer find needs a folder")?;
        let lines = fmcore::explorer::find_shell_window(p);
        return Ok(Out { v: json!({"ok": true, "lines": lines}), text: lines.join("
") + "
" });
    }
    if a.pos.get(1).map(|s| s.as_str()) == Some("check") {
        let p = a.arg(2, "explorer check needs a folder")?;
        let v = fmcore::explorer::check(p, a.flag("--parent"));
        return Ok(Out { v: json!({"ok": true, "verdict": format!("{v:?}")}), text: format!("{v:?}\n") });
    }
    let tabs = fmcore::explorer::list().map_err(fail)?;
    let mut t = String::new();
    for x in &tabs {
        t += &format!("{:#x} {} {}  [{} selected]\n", x.hwnd, if x.active { "*" } else { " " }, if x.location.is_empty() { "(virtual)" } else { &x.location }, x.selected.len());
    }
    t += &format!("{} Explorer tab(s)\n", tabs.len());
    let v = json!({"ok": true, "tabs": tabs.iter().map(|x| json!({"hwnd": x.hwnd, "tab": x.tab, "active": x.active, "location": x.location, "selected": x.selected})).collect::<Vec<_>>()});
    Ok(Out { v, text: t })
}

fn cmd_drives(a: &Args) -> R {
    let d = shell::drives_within(Duration::from_millis(a.num("--timeout")?.unwrap_or(2000)));
    let mut t = format!("{:<5} {:<24} {:<10} {:<6} {:>10} {:>10}  {}\n", "DRIVE", "LABEL", "TYPE", "FS", "FREE", "TOTAL", "SHARE");
    for x in &d {
        let (free, total) = if x.ready { (fmt_bytes(x.free), fmt_bytes(x.total)) } else if x.timed_out { ("no answer".into(), "-".into()) } else { ("-".into(), "-".into()) };
        t += &format!("{:<5} {:<24} {:<10} {:<6} {:>10} {:>10}  {}\n", x.root, trunc(&x.label, 24), x.kind, x.fs, free, total, x.remote);
    }
    let v = json!({"ok": true, "drives": d.iter().map(|x| json!({"root": x.root, "label": x.label, "type": x.kind, "fs": x.fs, "free": x.free, "total": x.total, "ready": x.ready, "remote": x.remote, "timed_out": x.timed_out})).collect::<Vec<_>>()});
    Ok(Out { v, text: t })
}

fn cmd_bin(a: &Args) -> R {
    match a.pos.get(1).map(|s| s.as_str()) {
        None | Some("list") => {
            let top = a.num("--top")?.unwrap_or(u64::MAX) as usize;
            let all = recycle::list_all();
            let total = all.len();
            let mut t = String::new();
            let mut arr = vec![];
            for r in all.iter().take(top) {
                let size = recycle::record_size(&r.record);
                let id = recycle::item_id(r);
                t += &format!("{:<18} {:<16} {:>10}  {}\n", id, fmt_time(r.deleted), fmt_bytes(size), r.original.display());
                arr.push(json!({"id": id, "original": r.original.to_string_lossy(), "deleted": ft_unix(r.deleted), "size": size, "dir": r.data.is_dir()}));
            }
            t += &format!("{total} items in the Recycle Bin\n");
            Ok(Out { v: json!({"ok": true, "count": total, "items": arr}), text: t })
        }
        Some("restore") => {
            let key = a.arg(2, "bin restore needs an id or the original path")?;
            let r = recycle::find_item(key).map_err(fail)?;
            recycle::restore(&r).map_err(fail)?;
            let mut op = journal::Op::new("restore");
            op.created.push(r.original.clone());
            journal::record_quiet(op);
            Ok(Out { v: json!({"ok": true, "restored": r.original.to_string_lossy()}), text: format!("restored {}\n", r.original.display()) })
        }
        Some("empty") => {
            if !a.flag("--yes") {
                return Err(usage("bin empty deletes everything in the Recycle Bin for good; add --yes to confirm"));
            }
            let n = recycle::list_all().len();
            recycle::empty_all().map_err(fail)?;
            Ok(Out { v: json!({"ok": true, "emptied": n}), text: format!("emptied the Recycle Bin ({n} items)\n") })
        }
        Some(o) => Err(usage(&format!("unknown bin action '{o}'"))),
    }
}

fn shell_entries_out(l: &[namespace::ShellEntry], empty: &str) -> Out {
    let mut t: String = l.iter().map(|e| format!("{}{:<36} {}\n", if e.is_folder { "[dir] " } else { "      " }, trunc(&e.name, 36), e.parsing)).collect();
    if l.is_empty() {
        t = format!("{empty}\n");
    }
    Out { v: json!({"ok": true, "count": l.len(), "items": l.iter().map(|e| json!({"name": e.name, "parsing": e.parsing, "folder": e.is_folder, "filesystem": e.is_filesystem})).collect::<Vec<_>>()}), text: t }
}

fn cmd_devices() -> R {
    let l = namespace::portable_devices().map_err(fail)?;
    Ok(shell_entries_out(&l, "no phones or cameras connected"))
}

fn cmd_shell_ls(a: &Args) -> R {
    let p = a.arg(1, "shell-ls needs a parsing name or path")?;
    let l = namespace::children(p).map_err(fail)?;
    Ok(shell_entries_out(&l, "empty"))
}

fn cmd_shell_copy(a: &Args) -> R {
    if a.pos.len() < 3 {
        return Err(usage("shell-copy needs <items...> <dest>"));
    }
    let items: Vec<String> = a.pos[1..a.pos.len() - 1].to_vec();
    let dest = a.pos.last().unwrap();
    namespace::copy(&items, dest).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "copied": items, "to": dest}), text: format!("copied {} item(s) to {dest}\n", items.len()) })
}

fn cmd_menu(a: &Args) -> R {
    if a.pos.len() < 2 {
        return Err(usage("menu needs paths"));
    }
    let paths: Vec<PathBuf> = a.pos[1..].iter().map(PathBuf::from).collect();
    let ext = a.flag("--extended");
    if let Some(v) = a.opt("--invoke") {
        let it = menu::invoke(&paths, v, ext).map_err(fail)?;
        return Ok(Out { v: json!({"ok": true, "invoked": {"verb": it.verb, "text": it.text}}), text: format!("ran '{}'\n", it.text) });
    }
    let l = menu::list(&paths, ext).map_err(fail)?;
    let mut t = format!("{:<22} {}\n", "VERB", "TEXT");
    for i in &l {
        let text = if i.parent.is_empty() { i.text.clone() } else { format!("{} > {}", i.parent, i.text) };
        t += &format!("{:<22} {}{}\n", if i.verb.is_empty() { "-" } else { &i.verb }, text, if i.enabled { "" } else { "  (disabled)" });
    }
    Ok(Out { v: json!({"ok": true, "items": l.iter().map(|i| json!({"verb": i.verb, "text": i.text, "parent": i.parent, "enabled": i.enabled, "icon": i.icon.as_ref().map(|c| format!("{}x{}", c.width, c.height))})).collect::<Vec<_>>()}), text: t })
}

/// Icon pixels as "W,H,hex" (premultiplied BGRA) for the window's out-of-process menu.
fn icon_hex(i: &menu::Icon) -> String {
    let mut s = String::with_capacity(i.bgra.len() * 2 + 16);
    s += &format!("{},{},", i.width, i.height);
    for b in &i.bgra {
        s += &format!("{b:02x}");
    }
    s
}

/// Internal, for RustFileExplorer: the right-click menu's shell entries and (with --apps EXT) the apps that open
/// the type, icons included, as JSON. It runs in this short-lived process so the shell extensions it loads
/// (7-Zip, Git, VS Code, Defender ...) never stay in the window's memory.
fn cmd_menu_data(a: &Args) -> R {
    // the value of --apps is not a path
    let apps_ext = a.opt("--apps").unwrap_or("").to_string();
    let paths: Vec<PathBuf> = a.pos[1..].iter().filter(|p| **p != apps_ext).map(PathBuf::from).collect();
    let items = if paths.is_empty() { vec![] } else { menu::list(&paths, false).unwrap_or_default() };
    let apps = match a.opt("--apps") {
        Some(ext) if !ext.is_empty() => shell::open_with_list(ext, false).unwrap_or_default().into_iter().take(12).collect::<Vec<_>>(),
        _ => vec![],
    };
    let v = json!({
        "ok": true,
        "items": items.iter().map(|i| json!({"verb": i.verb, "text": i.text, "parent": i.parent, "enabled": i.enabled, "icon": i.icon.as_ref().map(icon_hex)})).collect::<Vec<_>>(),
        "apps": apps.iter().map(|h| json!({"name": h.name, "ui_name": h.ui_name, "recommended": h.recommended, "icon_loc": h.icon, "default": h.default, "icon": menu::handler_icon(h).as_ref().map(icon_hex)})).collect::<Vec<_>>(),
    });
    Ok(Out { text: v.to_string(), v })
}

/// icon <path> --out <file.png> [--size N] [--shell]: the icon the file manager shows (or SHGetFileInfo's with --shell)
fn cmd_icon(a: &Args) -> R {
    let p = a.arg(1, "icon needs a path")?;
    let out = a.opt("--out").ok_or_else(|| usage("icon needs --out <file.png>"))?;
    let px = a.num("--size")?.unwrap_or(32).clamp(8, 256) as u32;
    let (icon, arrow) = if a.flag("--shell") {
        (menu::shgetfileinfo_icon(Path::new(p), px).ok_or_else(|| fail("no icon".into()))?, true)
    } else {
        (menu::list_icon(Path::new(p), px).ok_or_else(|| fail("no icon".into()))?, false)
    };
    // premultiplied -> straight alpha for the PNG
    let mut px_ = icon.bgra.clone();
    for q in px_.chunks_exact_mut(4) {
        let al = q[3] as u32;
        if al > 0 && al < 255 {
            for c in &mut q[..3] {
                *c = ((*c as u32 * 255 + al / 2) / al).min(255) as u8;
            }
        }
    }
    thumb::save_png(&thumb::Image { width: icon.width, height: icon.height, bgra: px_ }, Path::new(out)).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "file": out, "size": [icon.width, icon.height], "arrow_drawn_by_shell": arrow}), text: format!("saved {out} ({}x{})
", icon.width, icon.height) })
}

fn cmd_eject(a: &Args) -> R {
    let d = a.arg(1, "eject needs a drive letter")?;
    shell::eject(d).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "ejected": d}), text: format!("{d} can be removed now\n") })
}

fn cmd_recent(a: &Args) -> R {
    let top = a.num("--top")?.unwrap_or(20) as usize;
    let r = pins::recent(top).map_err(fail)?;
    let mut t = String::new();
    for x in &r {
        t += &format!("{:<16}  {}{}{}\n", fmt_time(x.used), x.target, if x.is_dir { "\\" } else { "" }, if x.exists { "" } else { "  (gone)" });
    }
    if r.is_empty() {
        t = "no recent items (Windows keeps none; \"Show recently opened items\" may be off)\n".into();
    }
    let v = json!({"ok": true, "recent": r.iter().map(|x| json!({"path": x.target, "name": x.name, "dir": x.is_dir, "exists": x.exists, "used": ft_unix(x.used)})).collect::<Vec<_>>()});
    Ok(Out { v, text: t })
}

fn pins_out(p: Vec<pins::Pin>) -> Out {
    let mut t = String::new();
    for x in &p {
        t += &format!("{:<24} {}{}\n", trunc(&x.name, 24), x.path, if x.exists { "" } else { "  (gone)" });
    }
    if p.is_empty() {
        t = "no pins\n".into();
    }
    Out { v: json!({"ok": true, "pins": p.iter().map(|x| json!({"path": x.path, "name": x.name, "exists": x.exists})).collect::<Vec<_>>()}), text: t }
}

fn cmd_pins() -> R {
    Ok(pins_out(pins::pins()))
}

fn cmd_pin(a: &Args) -> R {
    let p = a.arg(1, "pin needs a path")?;
    Ok(pins_out(pins::pin(Path::new(p), a.opt("--name")).map_err(fail)?))
}

fn cmd_unpin(a: &Args) -> R {
    let p = a.arg(1, "unpin needs a path or pin name")?;
    Ok(pins_out(pins::unpin(p).map_err(fail)?))
}

fn progress_json(p: &Progress) -> Value {
    json!({"event": "progress", "bytes_done": p.bytes_done, "bytes_total": p.bytes_total, "files_done": p.files_done, "files_total": p.files_total,
        "current": p.current.to_string_lossy(), "speed": p.speed.round(), "eta_s": p.eta_s, "finished": p.finished})
}

fn cmd_transfer(a: &Args, is_move: bool) -> R {
    if a.pos.len() < 3 {
        return Err(usage(if is_move { "move needs <src...> <dest>" } else { "copy needs <src...> <dest>" }));
    }
    let srcs: Vec<PathBuf> = a.pos[1..a.pos.len() - 1].iter().map(PathBuf::from).collect();
    let dest = PathBuf::from(a.pos.last().unwrap());
    let conflict = match a.opt("--conflict") {
        Some(c) => Conflict::parse(c).ok_or_else(|| usage("--conflict takes skip, replace or keep-both"))?,
        None => Conflict::Fail,
    };
    let ctl = Control::default();
    let json_out = a.json;
    let tty = std::io::stderr().is_terminal();
    let mut on = |p: &Progress| {
        if json_out {
            let mut so = std::io::stdout().lock();
            let _ = writeln!(so, "{}", progress_json(p));
            let _ = so.flush();
        } else if tty {
            let pct = if p.bytes_total > 0 { p.bytes_done as f64 * 100.0 / p.bytes_total as f64 } else { 100.0 };
            eprint!("\r{pct:5.1}%  {} / {}  {}/s  {} of {} files   ", fmt_bytes(p.bytes_done), fmt_bytes(p.bytes_total), fmt_bytes(p.speed as u64), p.files_done, p.files_total);
            if p.finished {
                eprintln!();
            }
        }
    };
    let rep = if is_move { ops::move_items(&srcs, &dest, conflict, &ctl, &mut on) } else { ops::copy(&srcs, &dest, conflict, &ctl, &mut on) }.map_err(fail)?;
    let verb = if is_move { "moved" } else { "copied" };
    let mut t = String::new();
    for (s, d) in &rep.done {
        t += &format!("{verb} {} -> {}\n", s.display(), d.display());
    }
    for s in &rep.skipped {
        t += &format!("skipped {} (target exists)\n", s.display());
    }
    for e in &rep.errors {
        t += &format!("FAILED {e}\n");
    }
    t += &format!("{} {}, {}{}\n", rep.files, if is_move { "items" } else { "files" }, fmt_bytes(rep.bytes), if rep.journal_id > 0 { "  (undo: wfm undo)" } else { "" });
    let v = json!({"ok": rep.errors.is_empty() && !rep.cancelled, "event": "done", "done": rep.done.iter().map(|(s, d)| json!({"from": s.to_string_lossy(), "to": d.to_string_lossy()})).collect::<Vec<_>>(),
        "skipped": rep.skipped.iter().map(|s| s.to_string_lossy()).collect::<Vec<_>>(), "errors": rep.errors, "files": rep.files, "bytes": rep.bytes, "cancelled": rep.cancelled, "journal_id": rep.journal_id});
    if rep.done.is_empty() && !rep.errors.is_empty() {
        return Err(fail(rep.errors.join("; ")));
    }
    Ok(Out { v, text: t })
}

fn cmd_rename(a: &Args) -> R {
    let p = a.arg(1, "rename needs <path> <new name>")?;
    let n = a.arg(2, "rename needs <path> <new name>")?;
    let to = ops::rename(Path::new(p), n).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "from": abs(p).to_string_lossy(), "to": to.to_string_lossy()}), text: format!("renamed to {}\n", to.display()) })
}

fn cmd_batch(a: &Args) -> R {
    let dir = a.arg(1, "batch-rename needs a folder")?;
    let pat = a.opt("--pattern").ok_or_else(|| usage("batch-rename needs --pattern"))?;
    let to = a.opt("--to").ok_or_else(|| usage("batch-rename needs --to"))?;
    let o = ops::BatchOpts { all: a.flag("--every"), folders: a.flag("--folders"), hidden: a.flag("--hidden"), stem_only: a.flag("--keep-ext") };
    let plan = ops::batch_plan(Path::new(dir), pat, to, &o).map_err(fail)?;
    let dry = a.flag("--dry-run");
    let mut t = String::new();
    for (f, n) in &plan.items {
        t += &format!("{} -> {}\n", f.file_name().unwrap().to_string_lossy(), n.file_name().unwrap().to_string_lossy());
    }
    for p in &plan.problems {
        t += &format!("PROBLEM {p}\n");
    }
    let items: Vec<Value> = plan.items.iter().map(|(f, n)| json!({"from": f.file_name().unwrap().to_string_lossy(), "to": n.file_name().unwrap().to_string_lossy()})).collect();
    if dry {
        t += &format!("{} would change, {} unchanged (dry run)\n", plan.items.len(), plan.unchanged);
        return Ok(Out { v: json!({"ok": plan.problems.is_empty(), "dry_run": true, "items": items, "unchanged": plan.unchanged, "problems": plan.problems}), text: t });
    }
    let id = ops::batch_apply(&plan).map_err(fail)?;
    t += &format!("{} renamed, {} unchanged\n", plan.items.len(), plan.unchanged);
    Ok(Out { v: json!({"ok": true, "dry_run": false, "items": items, "unchanged": plan.unchanged, "journal_id": id}), text: t })
}

fn cmd_delete(a: &Args) -> R {
    if a.pos.len() < 2 {
        return Err(usage("delete needs paths"));
    }
    let paths: Vec<PathBuf> = a.pos[1..].iter().map(PathBuf::from).collect();
    let permanent = a.flag("--permanent");
    let done = ops::delete(&paths, permanent).map_err(fail)?;
    let how = if permanent { "deleted for good" } else { "moved to the Recycle Bin" };
    let t: String = done.iter().map(|p| format!("{how}: {}\n", p.display())).collect();
    Ok(Out { v: json!({"ok": true, "permanent": permanent, "deleted": done.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()}), text: t })
}

fn cmd_mkdir(a: &Args) -> R {
    let p = ops::mkdir(Path::new(a.arg(1, "mkdir needs a path")?)).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "path": p.to_string_lossy()}), text: format!("created {}\n", p.display()) })
}

fn cmd_new_file(a: &Args) -> R {
    let p = ops::new_file(Path::new(a.arg(1, "new-file needs a path")?)).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "path": p.to_string_lossy()}), text: format!("created {}\n", p.display()) })
}

fn cmd_zip(a: &Args) -> R {
    if a.pos.len() < 3 {
        return Err(usage("zip needs <src...> <out.zip>"));
    }
    let srcs: Vec<PathBuf> = a.pos[1..a.pos.len() - 1].iter().map(PathBuf::from).collect();
    let out = zip::zip(&srcs, Path::new(a.pos.last().unwrap())).map_err(fail)?;
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    Ok(Out { v: json!({"ok": true, "path": out.to_string_lossy(), "size": size}), text: format!("created {} ({})\n", out.display(), fmt_bytes(size)) })
}

fn cmd_unzip(a: &Args) -> R {
    let z = a.arg(1, "unzip needs <archive> <dir>")?;
    let d = a.arg(2, "unzip needs <archive> <dir>")?;
    let out = zip::unzip(Path::new(z), Path::new(d)).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "dir": out.to_string_lossy()}), text: format!("unpacked into {}\n", out.display()) })
}

fn cmd_undo(a: &Args) -> R {
    if a.flag("--list") {
        let ops = journal::load();
        let mut t = String::new();
        for o in ops.iter().rev() {
            t += &format!("#{:<4} {}\n", o.id, o.summary());
        }
        if ops.is_empty() {
            t = "the journal is empty\n".into();
        }
        return Ok(Out { v: json!({"ok": true, "ops": ops.iter().rev().map(|o| o.to_json()).collect::<Vec<_>>()}), text: t });
    }
    let op = journal::undo_last().map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "undone": op.to_json()}), text: format!("undone: {}\n", op.summary()) })
}

fn cmd_open(a: &Args) -> R {
    if let Some(sel) = a.opt("--select") {
        let sel = sel.trim_start_matches(',');
        let (dir, name) = shell::select_target(Path::new(sel)).map_err(fail)?;
        return Ok(Out { v: json!({"ok": true, "folder": dir.to_string_lossy(), "select": name}), text: format!("folder {}\nselect {}\n", dir.display(), name) });
    }
    let p = a.arg(1, "open needs a path")?;
    let verb = a.opt("--verb").unwrap_or("open");
    if a.flag("--neighbors") {
        // pictures/videos/songs: the app gets the folder's other files of that kind, by name (arrow keys in Photos)
        shell::open_with_neighbors_wait(Path::new(p), "System.ItemNameDisplay", a.flag("--desc"), true).map_err(fail)?;
        // the app reads the list from this process later: stay alive a while (--keep seconds, default 120)
        let keep = a.opt("--keep").and_then(|s| s.parse::<u64>().ok()).unwrap_or(120);
        std::thread::sleep(std::time::Duration::from_secs(keep));
    } else {
        shell::open_verb(Path::new(p), verb, "").map_err(fail)?;
    }
    Ok(Out { v: json!({"ok": true, "path": abs(p).to_string_lossy(), "verb": verb}), text: format!("opened {}\n", abs(p).display()) })
}

/// openwith <file> [--all]            the apps that open it (Windows default marked, icon location)
/// openwith <file> <app> | --app <app>  open it with one of them
/// openwith default <.ext>              the Windows default app for a type (it is changed only in Windows' own dialog)
fn cmd_open_with(a: &Args) -> R {
    let p = a.arg(1, "openwith needs a path (or: openwith default <.ext>)")?;
    if p == "default" {
        let ext = a.arg(2, "openwith default needs an extension, e.g. .mp4")?;
        let key = shell::ext_key(ext);
        let (win, _) = shell::default_app(&key);
        return Ok(Out { v: json!({"ok": true, "ext": key, "windows_default": win}), text: format!("{key}: {}
", if win.is_empty() { "no default app".to_string() } else { win }) });
    }
    if let Some(app) = a.opt("--app").or_else(|| a.pos.get(2).map(|s| s.as_str())) {
        let h = shell::open_with(Path::new(p), app).map_err(fail)?;
        return Ok(Out { v: json!({"ok": true, "app": h.ui_name, "name": h.name}), text: format!("opened with {}\n", h.ui_name) });
    }
    let ext = ext_of(p);
    let list = shell::open_with_list(&ext, a.flag("--all")).map_err(fail)?;
    let mut t = String::new();
    for h in &list {
        t += &format!("{:<36} {:<9} {}
", trunc(&h.ui_name, 36), if h.default { "default" } else { "" }, h.name);
    }
    Ok(Out {
        v: json!({"ok": true, "ext": ext, "apps": list.iter().map(|h| json!({"ui_name": h.ui_name, "name": h.name, "recommended": h.recommended, "windows_default": h.default, "icon": h.icon})).collect::<Vec<_>>()}),
        text: t,
    })
}

fn cmd_reveal(a: &Args) -> R {
    let p = a.arg(1, "reveal needs a path")?;
    shell::reveal(Path::new(p)).map_err(fail)?;
    Ok(Out { v: json!({"ok": true}), text: "shown\n".into() })
}

fn cmd_watch(a: &Args) -> R {
    let dir = abs(a.arg(1, "watch needs a folder")?);
    let timeout = a.num("--timeout")?.map(Duration::from_secs);
    let max = a.num("--count")?.unwrap_or(u64::MAX);
    let mut n = 0u64;
    {
        let mut so = std::io::stdout().lock();
        let _ = writeln!(so, "{}", json!({"event": "watching", "dir": dir.to_string_lossy()}));
        let _ = so.flush();
    }
    watch::watch_blocking(&dir, a.flag("--recursive"), timeout, |c| {
        let mut so = std::io::stdout().lock();
        let _ = writeln!(so, "{}", json!({"event": c.kind.as_str(), "path": c.path.to_string_lossy(), "old_path": c.old_path.as_ref().map(|p| p.to_string_lossy())}));
        let _ = so.flush();
        n += 1;
        n < max
    })
    .map_err(fail)?;
    // The JSON lines are the output; the final line marks the end.
    Ok(Out { v: json!({"event": "stopped", "changes": n}), text: format!("{}\n", json!({"event": "stopped", "changes": n})) })
}

fn cmd_thumb(a: &Args) -> R {
    let p = a.arg(1, "thumb needs a path")?;
    let out = a.opt("--out").ok_or_else(|| usage("thumb needs --out <file.png>"))?;
    let size = a.num("--size")?.unwrap_or(256) as u32;
    let kind = if a.flag("--icon") {
        thumb::ThumbKind::IconOnly
    } else if a.flag("--thumbnail-only") {
        thumb::ThumbKind::ThumbnailOnly
    } else {
        thumb::ThumbKind::Auto
    };
    let out = abs(out);
    let (w, h) = thumb::thumbnail_png(Path::new(p), &out, size, kind).map_err(fail)?;
    Ok(Out { v: json!({"ok": true, "out": out.to_string_lossy(), "width": w, "height": h}), text: format!("wrote {} ({w}x{h})\n", out.display()) })
}

fn cmd_picture(a: &Args) -> R {
    let p = a.arg(1, "picture needs a path")?;
    let max = a.num("--max")?.unwrap_or(16384) as u32;
    let t0 = Instant::now();
    let d = fmcore::picture::decode(Path::new(p), max, 80_000_000, false).map_err(fail)?;
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut v = json!({"ok": true, "path": abs(p).to_string_lossy(), "width": d.width, "height": d.height, "full_width": d.full_w, "full_height": d.full_h, "orientation": d.orientation, "frames": d.frames, "ms": (ms * 10.0).round() / 10.0});
    if let Some(out) = a.opt("--out") {
        let out = abs(out);
        thumb::save_png(&thumb::Image { width: d.width, height: d.height, bgra: d.bgra }, &out).map_err(fail)?;
        v["out"] = json!(out.to_string_lossy());
    }
    let text = format!("{}x{} (original {}x{}), orientation {}, {} frame(s), {:.0} ms\n", d.width, d.height, d.full_w, d.full_h, d.orientation, d.frames, ms);
    Ok(Out { v, text })
}

fn dialog_json(d: &dialog::FileDialog) -> Value {
    json!({"hwnd": d.hwnd, "title": d.title, "pid": d.pid, "process": d.process, "kind": d.kind, "file_name": d.file_name})
}

fn cmd_dialog(a: &Args) -> R {
    let sub = a.arg(1, "dialog needs list or save")?;
    if let Some(d) = a.opt("--desktop") {
        dialog::use_desktop(d).map_err(fail)?;
    }
    match sub {
        "list" => {
            let l = dialog::list();
            let mut t = String::new();
            let mut arr = vec![];
            for d in &l {
                t += &format!("{:<10} {:<5} {:<22} {} [{}] name: {}\n", d.hwnd, d.kind, trunc(&d.title, 22), d.pid, d.process, d.file_name);
                let mut j = dialog_json(d);
                if a.flag("--verbose") {
                    j["controls"] = json!(dialog::describe(d.hwnd).iter().map(|(c, id, tx)| json!({"class": c, "id": id, "text": tx})).collect::<Vec<_>>());
                }
                arr.push(j);
            }
            if l.is_empty() {
                t = "no open Save/Open dialog\n".into();
            }
            Ok(Out { v: json!({"ok": true, "dialogs": arr}), text: t })
        }
        "save" => {
            let path = a.opt("--path").ok_or_else(|| usage("dialog save needs --path <full path>"))?;
            if !Path::new(path).is_absolute() {
                return Err(usage("--path must be a full path (like C:\\folder\\name.txt)"));
            }
            let hwnd = match a.opt("--hwnd") {
                Some(h) => Some(h.parse::<isize>().map_err(|_| usage("--hwnd needs a number from `wfm dialog list`"))?),
                None => None,
            };
            let r = dialog::save(path, hwnd, a.flag("--overwrite"), Duration::from_secs(5)).map_err(fail)?;
            let ok = r.closed;
            let mut t = if r.closed { format!("saved: {} accepted {}\n", r.dialog.process, path) } else { "the dialog is still open\n".to_string() };
            if let Some(c) = &r.confirm {
                t += &format!("the app asked: {c}{}\n", if r.confirmed { " (answered Yes)" } else { " (use --overwrite to answer Yes)" });
            }
            let v = json!({"ok": ok, "closed": r.closed, "confirm": r.confirm, "confirmed": r.confirmed, "dialog": dialog_json(&r.dialog), "path": path});
            if !ok {
                return Err((1, t.trim().replace('\n', "; ")));
            }
            Ok(Out { v, text: t })
        }
        o => Err(usage(&format!("unknown dialog action '{o}'"))),
    }
}

fn cmd_gui(a: &Args) -> R {
    // everything after "gui" is passed on as is (flags included)
    let i = a.all.iter().position(|x| x == "gui").unwrap_or(0);
    let line: Vec<&str> = a.all[i + 1..].iter().filter(|x| x.as_str() != "--json").map(|s| s.as_str()).collect();
    if line.is_empty() {
        return Err(usage("gui needs a command"));
    }
    if line[0] == "capture" {
        return gui_capture(line.get(1..).map(|r| r.join(" ")).unwrap_or_default().trim());
    }
    if line[0] == "shot-real" {
        return gui_shot_real(line.get(1..).map(|r| r.join(" ")).unwrap_or_default().trim());
    }
    let reply = fmcore::pipe::send(&line.join(" ")).map_err(fail)?;
    let v: Value = serde_json::from_str(&reply).unwrap_or_else(|_| json!({"ok": false, "error": reply}));
    if v["ok"] == json!(false) {
        return Err(fail(v["error"].as_str().unwrap_or("failed").to_string()));
    }
    let text = format!("{}
", serde_json::to_string_pretty(&v).unwrap());
    Ok(Out { v, text })
}

/// The window exactly as DWM composes it, backdrop (Mica / Acrylic) and per-pixel alpha included, through
/// Windows.Graphics.Capture (PrintWindow cannot show the backdrop). Saves a PNG with the captured alpha and prints
/// the colour and alpha at sample points of each panel, so transparency can be proven without a screen.
fn gui_capture(file: &str) -> R {
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::{HMODULE, HWND};
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::core::Interface;
    if file.is_empty() {
        return Err(usage("gui capture <file.png>"));
    }
    let _ = fmcore::pipe::send("paint");
    let reply = fmcore::pipe::send("state").map_err(fail)?;
    let st: Value = serde_json::from_str(&reply).map_err(|_| fail("bad state reply".into()))?;
    let hw = st["hwnd"].as_i64().ok_or_else(|| fail("the window gave no hwnd".into()))?;
    let hwnd = HWND(hw as isize as *mut _);
    let e = |m: &str, x: windows::core::Error| fail(format!("{m}: {}", x.message()));
    unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        let mut dev: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx)).map_err(|x| e("D3D11CreateDevice", x))?;
        let (dev, ctx) = (dev.unwrap(), ctx.unwrap());
        let dxgi: IDXGIDevice = dev.cast().map_err(|x| e("IDXGIDevice", x))?;
        let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi).and_then(|i| i.cast()).map_err(|x| e("WinRT device", x))?;
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(|x| e("capture interop", x))?;
        let item: GraphicsCaptureItem = interop.CreateForWindow(hwnd).map_err(|x| e("CreateForWindow", x))?;
        let size = item.Size().map_err(|x| e("item size", x))?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size).map_err(|x| e("frame pool", x))?;
        let session = pool.CreateCaptureSession(&item).map_err(|x| e("session", x))?;
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture().map_err(|x| e("StartCapture", x))?;
        let t0 = std::time::Instant::now();
        let frame = loop {
            if let Ok(f) = pool.TryGetNextFrame() {
                break f;
            }
            if t0.elapsed() > std::time::Duration::from_secs(3) {
                let _ = session.Close();
                return Err(fail("no frame from Windows.Graphics.Capture in 3 s".into()));
            }
            let _ = fmcore::pipe::send("paint");
            std::thread::sleep(std::time::Duration::from_millis(30));
        };
        let surf = frame.Surface().map_err(|x| e("surface", x))?;
        let access: IDirect3DDxgiInterfaceAccess = surf.cast().map_err(|x| e("dxgi access", x))?;
        let tex: ID3D11Texture2D = access.GetInterface().map_err(|x| e("texture", x))?;
        let mut d = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut d);
        let sd = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0, ..d };
        let mut staging: Option<ID3D11Texture2D> = None;
        dev.CreateTexture2D(&sd, None, Some(&mut staging)).map_err(|x| e("staging", x))?;
        let staging = staging.unwrap();
        ctx.CopyResource(&staging, &tex);
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).map_err(|x| e("map", x))?;
        let (w, h) = (d.Width as usize, d.Height as usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            let src = std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), w * 4);
            px[y * w * 4..(y + 1) * w * 4].copy_from_slice(src);
        }
        ctx.Unmap(&staging, 0);
        let _ = frame.Close();
        let _ = session.Close();
        let _ = pool.Close();
        // the window's client area starts at the frame's top-left (custom frame, no caption); sample each panel
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let at = |x: f32, y: f32| {
            let (xi, yi) = (((x * dpi) as usize).min(w - 1), ((y * dpi) as usize).min(h - 1));
            let i = (yi * w + xi) * 4;
            json!({"x": x, "y": y, "rgba": format!("#{:02X}{:02X}{:02X}", px[i + 2], px[i + 1], px[i]), "alpha": px[i + 3]})
        };
        let wl = w as f32 / dpi;
        let samples = json!({
            "titlebar": at(wl - 200.0, 4.0), "toolbar": at(wl - 260.0, 44.0), "sidebar": at(8.0, h as f32 / dpi - 60.0),
            "content": at(wl - 40.0, h as f32 / dpi - 80.0),
        });
        thumb::save_png(&thumb::Image { width: w as u32, height: h as u32, bgra: px }, Path::new(file)).map_err(fail)?;
        let text = format!("saved {file} ({w}x{h})\n{}\n", serde_json::to_string_pretty(&samples).unwrap());
        Ok(Out { v: json!({"ok": true, "file": file, "size": [w, h], "samples": samples}), text })
    }
}

/// The window as Windows composes it (DWM), not our own offscreen paint: PrintWindow with PW_RENDERFULLCONTENT.
/// Run from this process so the window's thread is free to answer.
fn gui_shot_real(file: &str) -> R {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    if file.is_empty() {
        return Err(usage("gui shot-real <file.png>"));
    }
    let _ = fmcore::pipe::send("paint");
    let reply = fmcore::pipe::send("state").map_err(fail)?;
    let st: Value = serde_json::from_str(&reply).map_err(|_| fail("bad state reply".into()))?;
    let hw = st["hwnd"].as_i64().ok_or_else(|| fail("the window gave no hwnd".into()))?;
    let hwnd = HWND(hw as isize as *mut _);
    unsafe {
        let mut rc = RECT::default();
        GetWindowRect(hwnd, &mut rc).map_err(|e| fail(e.message()))?;
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        let dc = CreateCompatibleDC(None);
        let bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).map_err(|e| fail(e.message()))?;
        let old = SelectObject(dc, bmp.into());
        // PW_RENDERFULLCONTENT = 2
        let ok = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)).as_bool();
        let _ = GdiFlush();
        let mut px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        for p in px.chunks_exact_mut(4) {
            p[3] = 255;
        }
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        if !ok {
            return Err(fail("PrintWindow failed".into()));
        }
        thumb::save_png(&thumb::Image { width: w as u32, height: h as u32, bgra: px }, Path::new(file)).map_err(fail)?;
        Ok(Out { v: json!({"ok": true, "file": file, "size": [w, h], "window": [rc.left, rc.top]}), text: format!("saved {file} ({w}x{h})
") })
    }
}

fn cmd_default(a: &Args) -> R {
    let what = a.pos.get(1).map(|s| s.as_str()).unwrap_or("status");
    // --root KEY (tests only): write under HKCU\KEY instead of HKCU\Software\Classes; it must name a test key
    let root = match a.opt("--root") {
        Some(r) if r.to_lowercase().contains("test") => r.trim_start_matches(r"HKCU\").to_string(),
        Some(_) => return Err(usage("--root is for tests only: its key name must contain \"Test\"")),
        None => fmcore::defaultfm::CLASSES.to_string(),
    };
    let st = |root: &str| {
        let s = fmcore::defaultfm::status_in(root);
        let mut t = String::new();
        for (k, on) in s.parts() {
            t += &format!("{:<12} {}\n", k, if on { "Rust File Explorer" } else { "Explorer" });
        }
        (json!({"default": s.folders, "all": s.all(), "parts": s.parts().iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(), "root": root}), t)
    };
    match what {
        "status" => {
            let (mut v, t) = st(&root);
            v["ok"] = json!(true);
            Ok(Out { v, text: t })
        }
        "set" | "restore" => {
            if !a.flag("--yes") {
                return Err(usage(&format!("default {what} changes how Windows opens folders; add --yes")));
            }
            if what == "set" {
                let exe = match a.opt("--exe") {
                    Some(e) => std::path::PathBuf::from(e),
                    None => std::env::current_exe().map_err(|e| fail(e.to_string()))?.with_file_name("RustFileExplorer.exe"),
                };
                // the real root also starts the Win+E helper; a test root only writes its keys
                if root == fmcore::defaultfm::CLASSES { fmcore::defaultfm::set(&exe) } else { fmcore::defaultfm::set_in(&root, &exe) }.map_err(fail)?;
            } else {
                if root == fmcore::defaultfm::CLASSES { fmcore::defaultfm::restore() } else { fmcore::defaultfm::restore_in(&root) }.map_err(fail)?;
            }
            let (mut v, t) = st(&root);
            v["ok"] = json!(true);
            Ok(Out { v, text: format!("done\n{t}") })
        }
        // fire every common way of opening a folder and report which app got it (opens and closes real windows)
        "probe" => {
            if !a.flag("--yes") {
                return Err(usage("default probe opens real windows on the desktop for a moment and closes them; add --yes"));
            }
            let json_out = a.flag("--json");
            let rows = fmcore::probe::run(&mut |r| {
                if !json_out {
                    println!("{:<42} {:<46} {}", r.way, r.got, r.note);
                }
            })
            .map_err(fail)?;
            let v = json!({"ok": true, "rows": rows.iter().map(|r| json!({"way": r.way, "got": r.got, "note": r.note})).collect::<Vec<_>>()});
            Ok(Out { v, text: format!("{} ways probed
", rows.len()) })
        }
        _ => Err(usage("default status|set|restore|probe [--yes] [--root TESTKEY] [--exe PATH]")),
    }
}

fn cmd_ev(a: &Args) -> R {
    let words = a.pos[1..].join(" ");
    let top = a.num("--top")?.unwrap_or(20) as u32;
    let r = fmcore::evipc::query(&words, top, fmcore::evipc::REQ_NAME | fmcore::evipc::REQ_PATH | fmcore::evipc::REQ_SIZE | fmcore::evipc::REQ_MODIFIED | fmcore::evipc::REQ_ATTRS, fmcore::evipc::SORT_DATE_MODIFIED_DESC, false, Duration::from_secs(3)).map_err(fail)?;
    let mut t = format!("{} results, {:.1} ms
", r.total, r.ms);
    for i in &r.items {
        t += &format!("{:>10}  {}
", i.size.map(fmt_bytes).unwrap_or_default(), i.full());
    }
    Ok(Out { v: json!({"ok": true, "total": r.total, "ms": r.ms, "items": r.items.iter().map(|i| json!({"path": i.full(), "dir": i.is_dir, "size": i.size, "modified": ft_unix(i.modified)})).collect::<Vec<_>>()}), text: t })
}

/// Internal relay for wfiles on another desktop: raw Everything reply bytes to stdout (args: hex query, max, req, sort, path).
fn cmd_ev_raw(a: &Args) -> R {
    let hex = a.pos.get(1).map(|s| s.as_str()).unwrap_or("");
    let units: Vec<u16> = (0..hex.len() / 4).filter_map(|i| u16::from_str_radix(&hex[i * 4..i * 4 + 4], 16).ok()).collect();
    let q = String::from_utf16_lossy(&units);
    let n = |i: usize| a.pos.get(i).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    let bytes = fmcore::evipc::query_raw(&q, n(2), n(3), n(4), n(5) == 1, Duration::from_secs(4)).map_err(fail)?;
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(&bytes);
    let _ = so.flush();
    std::process::exit(0);
}
