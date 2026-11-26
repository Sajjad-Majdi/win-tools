//! `wfm default probe`: fire every common way Windows and apps open a folder and see which app got it.
//!
//! It opens real windows on the desktop for a moment and closes each one again: our windows through their own
//! control pipe (`quit`; the probe runs them on a private pipe name), Explorer windows through IShellWindows, and
//! only those that appeared during the probe (the Explorer windows and Rust File Explorer processes that were there
//! before are recorded first and never touched). It refuses to run while the user's own Rust File Explorer window is
//! open, because a folder sent there by Explorer would land in it.
//! RustFileExplorer.exe writes one line per start to %TEMP%\wfm-probe-log.txt while the probe runs
//! ([`crate::explorer::probe_note`]), so a place it handed to Explorer is told apart from one Explorer opened itself.
use crate::explorer::{ExplorerTab, tabs_with_browser};
use crate::util::*;
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

/// One way of opening, and what came of it.
#[derive(Clone, Debug)]
pub struct Row {
    pub way: String,
    /// "Rust File Explorer", "Explorer", "Explorer (handed back by Rust File Explorer)", "nothing", "error: ..."
    pub got: String,
    /// what Rust File Explorer noted, the place Explorer showed, what was closed
    pub note: String,
}

fn rfe_pids() -> HashSet<u32> {
    let mut out = HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            if from_wide(&e.szExeFile).eq_ignore_ascii_case("RustFileExplorer.exe") {
                out.insert(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// (window, tab) of every Explorer tab now open.
fn explorer_keys() -> HashSet<(isize, isize)> {
    tabs_with_browser().map(|v| v.into_iter().map(|(_, t)| (t.hwnd, t.tab)).collect()).unwrap_or_default()
}

fn shell_execute(file: &str, verb: Option<&str>, class: Option<&str>) -> Res<()> {
    let f = wide(file);
    let v = verb.map(wide);
    let c = class.map(wide);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | if c.is_some() { SEE_MASK_CLASSNAME } else { 0 },
        lpVerb: v.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
        lpFile: PCWSTR(f.as_ptr()),
        lpClass: c.as_ref().map(|c| PCWSTR(c.as_ptr())).unwrap_or(PCWSTR::null()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei) }.map_err(|e| win_err("ShellExecuteEx", &e))
}

fn open_and_select(file: &Path) -> Res<()> {
    let w = wide(file.as_os_str());
    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).map_err(|e| win_err("SHParseDisplayName", &e))?;
        let r = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
        r.map_err(|e| win_err("SHOpenFolderAndSelectItems", &e))
    }
}

fn spawn(exe: &str, raw: &str) -> Res<()> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(exe).raw_arg(raw).spawn().map(|_| ()).map_err(|e| format!("{exe}: {e}"))
}

fn log_lines(log: &Path, from: usize) -> Vec<String> {
    std::fs::read_to_string(log).unwrap_or_default().lines().skip(from).map(|l| l.splitn(3, '\t').nth(2).unwrap_or(l).replace('\t', " -> ")).collect()
}

fn log_len(log: &Path) -> usize {
    std::fs::read_to_string(log).map(|s| s.lines().count()).unwrap_or(0)
}

/// End a Rust File Explorer the probe started: `quit` on the pipe it serves, then (still running after 3 s) end it.
fn close_rfe(pids: &[u32], private_pipe: &str, default_pipe_was_up: bool) -> Vec<String> {
    let mut done = vec![];
    if pids.is_empty() {
        return done;
    }
    // SAFETY of the env switch: the probe is single-threaded here
    unsafe { std::env::set_var("WFM_PIPE", private_pipe) };
    if crate::pipe::exists() && crate::pipe::send("quit").is_ok() {
        done.push("quit (private pipe)".to_string());
    }
    if !default_pipe_was_up {
        unsafe { std::env::set_var("WFM_PIPE", crate::pipe::NAME) };
        if crate::pipe::exists() && crate::pipe::send("quit").is_ok() {
            done.push("quit (default pipe, opened by the probe)".to_string());
        }
        unsafe { std::env::set_var("WFM_PIPE", private_pipe) };
    }
    for &pid in pids {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) else { continue };
            if WaitForSingleObject(h, 3000).0 != 0 {
                let _ = TerminateProcess(h, 1);
                done.push(format!("ended pid {pid}"));
            }
            let _ = CloseHandle(h);
        }
    }
    done
}

/// Close the Explorer windows / tabs that appeared after `before` and show `place` (or any new one when the place
/// is virtual and Explorer names it differently). Returns the places closed and the new ones left open.
fn close_new_explorer(before: &HashSet<(isize, isize)>, place_keys: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut closed, mut left) = (vec![], vec![]);
    let Ok(all) = tabs_with_browser() else { return (closed, left) };
    for (wb, t) in all {
        if before.contains(&(t.hwnd, t.tab)) {
            continue;
        }
        let raw = t.raw.to_lowercase();
        let ours = place_keys.iter().any(|k| !k.is_empty() && (raw.contains(k) || (k == "::" && (raw.starts_with("::") || raw.is_empty()))));
        if ours {
            let _ = unsafe { wb.Quit() };
            closed.push(display(&t));
        } else {
            left.push(display(&t));
        }
    }
    (closed, left)
}

/// The window's own settings files in %LOCALAPPDATA%\WinTools as they are now (other apps' files are not read).
fn save_settings() -> Vec<(std::path::PathBuf, Option<Vec<u8>>)> {
    let dir = std::env::var_os("LOCALAPPDATA").map(|d| Path::new(&d).join("WinTools")).unwrap_or_default();
    ["fm.json", "look.json", "viewer.json"].iter().map(|n| dir.join(n)).map(|p| (p.clone(), std::fs::read(&p).ok())).collect()
}

/// Put back any of those files a window started by the probe changed.
fn restore_settings(saved: &[(std::path::PathBuf, Option<Vec<u8>>)]) {
    for (p, bytes) in saved {
        if let Some(b) = bytes
            && std::fs::read(p).ok().as_ref() != Some(b)
        {
            let _ = std::fs::write(p, b);
        }
    }
}

fn display(t: &ExplorerTab) -> String {
    if t.raw.is_empty() { "(no name)".into() } else { t.raw.clone() }
}

struct Case {
    way: &'static str,
    /// lower-case bits of the place Explorer should show ("::" = any virtual place)
    keys: Vec<String>,
    fire: Box<dyn Fn() -> Res<()>>,
}

/// Run every case; `progress` gets each row as it is done.
pub fn run(progress: &mut dyn FnMut(&Row)) -> Res<Vec<Row>> {
    let _c = com();
    let default_up = {
        unsafe { std::env::set_var("WFM_PIPE", crate::pipe::NAME) };
        crate::pipe::exists()
    };
    if default_up {
        return Err("your Rust File Explorer window is open: close it first (the probe must not send folders into it)".into());
    }
    let private = format!("wfm-probe-{}", std::process::id());
    unsafe { std::env::set_var("WFM_PIPE", &private) };
    let tmp = std::env::temp_dir();
    let base = tmp.join(format!("wfm-probe-{}", std::process::id()));
    let folder = base.join("Probe Folder");
    let file = folder.join("probe file.txt");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::fs::write(&file, b"probe").map_err(|e| e.to_string())?;
    let log = tmp.join("wfm-probe-log.txt");
    let _ = std::fs::remove_file(&log);
    std::fs::write(tmp.join("wfm-probe-active"), b"1").map_err(|e| e.to_string())?;
    let rfe_before = rfe_pids();
    let ex_before = explorer_keys();
    // our windows started by the probe itself get a private data folder; one Explorer starts uses the real one, so
    // the real settings files are kept as they are now and put back at the end
    let data = base.join("data");
    let _ = std::fs::create_dir_all(&data);
    let saved = save_settings();
    unsafe { std::env::set_var("WFM_DATA_DIR", &data) };

    let f = folder.display().to_string();
    let fl = file.display().to_string();
    let fk = "probe folder".to_string();
    let explorer = format!(r"{}\explorer.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
    let (f1, f2, f3, f4, f5, f6) = (f.clone(), f.clone(), f.clone(), f.clone(), f.clone(), file.clone());
    let (e1, e2, e3) = (explorer.clone(), explorer.clone(), fl.clone());
    let control = format!(r"{}\System32\control.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
    let cases: Vec<Case> = vec![
        Case { way: "ShellExecuteEx, no verb (folder)", keys: vec![fk.clone()], fire: Box::new(move || shell_execute(&f1, None, None)) },
        Case { way: "ShellExecuteEx \"open\"", keys: vec![fk.clone()], fire: Box::new(move || shell_execute(&f2, Some("open"), None)) },
        Case { way: "ShellExecuteEx \"explore\"", keys: vec![fk.clone()], fire: Box::new(move || shell_execute(&f3, Some("explore"), None)) },
        Case { way: "ShellExecuteEx lpClass \"Folder\"", keys: vec![fk.clone()], fire: Box::new(move || shell_execute(&f4, None, Some("Folder"))) },
        Case { way: "explorer.exe <folder>", keys: vec![fk.clone()], fire: Box::new(move || spawn(&e1, &format!("\"{f5}\""))) },
        Case { way: "explorer.exe /select,<file>", keys: vec![fk.clone()], fire: Box::new(move || spawn(&e2, &format!("/select,\"{e3}\""))) },
        Case { way: "SHOpenFolderAndSelectItems(<file>)", keys: vec![fk.clone()], fire: Box::new(move || open_and_select(&f6)) },
        Case { way: "ShellExecuteEx shell:Downloads", keys: vec!["downloads".into()], fire: Box::new(|| shell_execute("shell:Downloads", None, None)) },
        Case { way: "Control Panel (control.exe)", keys: vec!["::".into()], fire: Box::new(move || spawn(&control, "")) },
        Case { way: "Control Panel (shell:ControlPanelFolder)", keys: vec!["::".into()], fire: Box::new(|| shell_execute("shell:ControlPanelFolder", None, None)) },
        Case { way: "Recycle Bin (shell:RecycleBinFolder)", keys: vec!["::".into()], fire: Box::new(|| shell_execute("shell:RecycleBinFolder", None, None)) },
        Case { way: "Network (shell:NetworkPlacesFolder)", keys: vec!["::".into()], fire: Box::new(|| shell_execute("shell:NetworkPlacesFolder", None, None)) },
        Case { way: "This PC (shell:MyComputerFolder)", keys: vec!["::".into()], fire: Box::new(|| shell_execute("shell:MyComputerFolder", None, None)) },
    ];

    let mut rows = vec![];
    for c in cases {
        let ex_now = explorer_keys();
        let rfe_now = rfe_pids();
        let from = log_len(&log);
        let fired = (c.fire)();
        let start = Instant::now();
        let mut seen_at: Option<Instant> = None;
        // up to 8 s for something to show; then 1.5 s more so a hand-back (our exe -> Explorer) can finish
        while start.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(250));
            let new_ex = explorer_keys().difference(&ex_now).count() > 0;
            let new_rfe = rfe_pids().difference(&rfe_now).count() > 0 || crate::pipe::exists();
            if (new_ex || new_rfe || log_len(&log) > from) && seen_at.is_none() {
                seen_at = Some(Instant::now());
            }
            if seen_at.is_some_and(|t| t.elapsed() > Duration::from_millis(1500)) {
                break;
            }
        }
        let notes = log_lines(&log, from);
        let new_ex: Vec<String> = tabs_with_browser().map(|v| v.into_iter().filter(|(_, t)| !ex_now.contains(&(t.hwnd, t.tab))).map(|(_, t)| display(&t)).collect()).unwrap_or_default();
        let new_rfe: Vec<u32> = rfe_pids().difference(&rfe_now).copied().filter(|p| !rfe_before.contains(p)).collect();
        let rfe_window = !new_rfe.is_empty() || crate::pipe::exists();
        let handed = notes.iter().any(|n| n.contains("Explorer(") || n.contains("Explorer (") || n.contains("handed back"));
        let got = match &fired {
            Err(e) if new_ex.is_empty() && !rfe_window => format!("error: {e}"),
            _ if !new_ex.is_empty() && handed => "Explorer (handed back by Rust File Explorer)".to_string(),
            _ if !new_ex.is_empty() && rfe_window => "both (Explorer and Rust File Explorer)".to_string(),
            _ if !new_ex.is_empty() => "Explorer".to_string(),
            _ if rfe_window => "Rust File Explorer".to_string(),
            _ if notes.iter().any(|n| n.contains("loop guard")) => "nothing (loop guard stopped it)".to_string(),
            _ => "nothing".to_string(),
        };
        let mut note = vec![];
        if !notes.is_empty() {
            note.push(format!("RFE: {}", notes.join(" | ")));
        }
        if !new_ex.is_empty() {
            note.push(format!("Explorer showed: {}", new_ex.join(", ")));
        }
        // close what this case opened, before the next one
        let mut keys = c.keys.clone();
        keys.push(fk.clone());
        let (closed, left) = close_new_explorer(&ex_before, &keys);
        let stopped = close_rfe(&new_rfe, &private, false);
        if !closed.is_empty() {
            note.push(format!("closed Explorer: {}", closed.join(", ")));
        }
        if !left.is_empty() {
            note.push(format!("left open (not the probe's place): {}", left.join(", ")));
        }
        if !stopped.is_empty() {
            note.push(format!("closed RFE: {}", stopped.join(", ")));
        }
        let row = Row { way: c.way.to_string(), got, note: note.join("; ") };
        progress(&row);
        rows.push(row);
        // a quiet moment between cases (the loop guard window is 2 s)
        std::thread::sleep(Duration::from_millis(2200));
    }
    let _ = std::fs::remove_file(tmp.join("wfm-probe-active"));
    // last sweep: nothing the probe opened stays
    let _ = close_new_explorer(&ex_before, &["probe folder".into(), "::".into(), "downloads".into()]);
    let leftovers: Vec<u32> = rfe_pids().difference(&rfe_before).copied().collect();
    let _ = close_rfe(&leftovers, &private, false);
    let _ = std::fs::remove_dir_all(&base);
    restore_settings(&saved);
    Ok(rows)
}
