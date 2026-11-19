//! Judgements on top of the raw sample: an explainable impact score, merged rows with named children,
//! "free up memory" candidates, "why is this running?", restart, and hung-app detection.
use crate::procs::{self, Kind, Proc};
use std::collections::HashMap;

// ---------------------------------------------------------------------------------------------- impact
#[derive(Clone, Debug)]
pub struct Impact {
    pub score: f32,
    /// "cpu" | "memory" | "disk" | "gpu"
    pub driver: &'static str,
    /// e.g. "memory 347 MB = 2.2% of RAM"
    pub why: String,
}

/// Each resource as a share of the whole PC (0..100), weighted; the biggest one ranks the row and is named.
/// Disk: 50 MB/s counts as 100%. Ties are broken by a small part of the other shares.
pub fn impact(cpu: f32, mem: u64, io: f64, gpu: f32, ram_total: u64) -> Impact {
    let mem_pct = mem as f64 / ram_total.max(1) as f64 * 100.0;
    let disk_pct = (io / (50.0 * 1048576.0) * 100.0).min(100.0);
    let parts = [("cpu", cpu as f64 * 1.0), ("memory", mem_pct * 0.6), ("disk", disk_pct * 0.5), ("gpu", gpu as f64 * 0.3)];
    let (driver, top) = parts.iter().copied().fold(("memory", -1.0), |a, b| if b.1 > a.1 { b } else { a });
    let rest: f64 = parts.iter().map(|p| p.1).sum::<f64>() - top;
    let why = match driver {
        "cpu" => format!("CPU {cpu:.1}% of the PC"),
        "disk" => format!("disk {}/s", procs::fmt_bytes(io)),
        "gpu" => format!("GPU {gpu:.0}%"),
        _ => format!("memory {} = {mem_pct:.1}% of RAM", procs::fmt_bytes(mem as f64)),
    };
    Impact { score: (top + rest * 0.25) as f32, driver, why }
}

// ---------------------------------------------------------------------------------------------- merged rows
#[derive(Clone, Debug)]
pub struct Group {
    pub key: String,
    pub label: String,
    pub kind: Kind,
    pub pids: Vec<u32>,
    pub cpu: f32,
    pub mem: u64,
    pub io: f64,
    /// per member: (pid, child label from its script / --type / window title, cpu, mem)
    pub members: Vec<(u32, String, f32, u64)>,
}

/// What a child process is doing, from its command line: the script it runs, chrome --type, or its window title.
pub fn child_label(p: &Proc, cmdline: Option<&str>) -> String {
    if !p.title.is_empty() {
        return p.title.clone();
    }
    let Some(c) = cmdline else { return String::new() };
    if let Some(i) = c.find("--type=") {
        return c[i + 7..].chars().take_while(|ch| !ch.is_whitespace() && *ch != '"').collect();
    }
    // first argument that looks like a script or file (python x.py, node relay.js, powershell -File y.ps1)
    let args = split_cmdline(c);
    // prefer an argument that is a file or path (xray run -c config.json -> config.json)
    if let Some(a) = args.iter().skip(1).find(|a| !a.starts_with('-') && (a.contains('\\') || a.contains('/') || a.rsplit_once('.').map(|(_, e)| (1..=4).contains(&e.len())).unwrap_or(false))) {
        return tail(a);
    }
    for (i, a) in args.iter().enumerate().skip(1) {
        let al = a.to_lowercase();
        if (al == "-file" || al == "-m" || al == "-c") && i + 1 < args.len() {
            let v = &args[i + 1];
            return if al == "-c" { v.chars().take(50).collect() } else { tail(v) };
        }
        if a.starts_with('-') {
            continue;
        }
        return tail(a);
    }
    String::new()
}

fn tail(a: &str) -> String {
    let parts: Vec<&str> = a.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    // keep the last two parts so "relay\server.js" stays meaningful
    let n = parts.len();
    let s = if n >= 2 { format!("{}\\{}", parts[n - 2], parts[n - 1]) } else { parts.last().unwrap_or(&"").to_string() };
    s.chars().take(60).collect()
}

/// Windows command-line splitting (quotes; enough for display).
pub fn split_cmdline(c: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut q = false;
    for ch in c.chars() {
        match ch {
            '"' => q = !q,
            ' ' | '\t' if !q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Merge processes by exe name and product (description), so "Claude" (the app) and "Claude Code" stay apart.
pub fn merge(procs: &[Proc], cmdlines: &HashMap<u32, String>, services: &HashMap<u32, Vec<String>>) -> Vec<Group> {
    let mut map: HashMap<String, Group> = HashMap::new();
    for p in procs.iter().filter(|p| p.pid != 0) {
        let svc_host = p.name.eq_ignore_ascii_case("svchost.exe");
        let label = if svc_host {
            services.get(&p.pid).map(|s| format!("Service Host: {}", s[0])).unwrap_or_else(|| "Service Host".into())
        } else if !p.meta.description.is_empty() {
            p.meta.description.clone()
        } else {
            p.name.clone()
        };
        // service hosts stay one row each (each runs different services)
        let key = if svc_host { format!("svchost:{}", p.pid) } else { format!("{}|{}", p.name.to_lowercase(), label.to_lowercase()) };
        let g = map.entry(key.clone()).or_insert_with(|| Group { key, label, kind: p.kind, pids: vec![], cpu: 0.0, mem: 0, io: 0.0, members: vec![] });
        if p.kind == Kind::App {
            g.kind = Kind::App;
        }
        g.pids.push(p.pid);
        g.cpu += p.cpu;
        g.mem += p.mem;
        g.io += p.io_rate;
        g.members.push((p.pid, child_label(p, cmdlines.get(&p.pid).map(|s| s.as_str())), p.cpu, p.mem));
    }
    let mut v: Vec<Group> = map.into_values().collect();
    for g in &mut v {
        g.members.sort_by(|a, b| b.2.total_cmp(&a.2).then(b.3.cmp(&a.3)));
    }
    v
}

// ---------------------------------------------------------------------------------------------- free up memory
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FreeVerdict {
    Safe,
    /// safe, but something stops (see `effect`)
    SafeButStops,
    Keep,
}

#[derive(Clone, Debug)]
pub struct FreeItem {
    pub label: String,
    pub pids: Vec<u32>,
    pub mem: u64,
    pub verdict: FreeVerdict,
    pub effect: String,
    /// growing app: restarting it gets the memory back and keeps the app
    pub restart_hint: bool,
    pub startup_entry: Option<String>,
}

/// name (lower case) -> what stops when it ends
/// What stops when a process with this exe name ends (None = nothing specific known).
pub fn stop_effect(name: &str) -> Option<String> {
    let n = name.to_lowercase();
    STOPS.iter().find(|(k, _)| *k == n).map(|(_, e)| format!("Ending it is safe, but {e}."))
}

const STOPS: &[(&str, &str)] = &[
    ("xray.exe", "the VPN / proxy connection drops"),
    ("v2rayn.exe", "the VPN / proxy connection drops"),
    ("windscribe.exe", "the VPN drops"),
    ("idman.exe", "running downloads pause"),
    ("onedrive.exe", "file sync pauses"),
    ("telegram.exe", "new messages stop arriving on this PC"),
    ("python.exe", "the script it runs stops"),
    ("pythonw.exe", "the script it runs stops"),
    ("node.exe", "the server or tool it runs stops"),
    ("code.exe", "unsaved editor work may be lost"),
    ("chrome.exe", "open tabs close"),
    ("msedge.exe", "open tabs close"),
    ("firefox.exe", "open tabs close"),
    ("bb.exe", "Browser Bridge stops"),
    ("goya.exe", "voice typing stops"),
    ("rustc.exe", "the build it is running fails"),
    ("cargo.exe", "the build it is running fails"),
    ("cl.exe", "the build it is running fails"),
    ("link.exe", "the build it is running fails"),
    ("ld.exe", "the build it is running fails"),
    ("gcc.exe", "the build it is running fails"),
    ("msbuild.exe", "the build it is running fails"),
    ("git.exe", "the git command it runs stops half-way"),
    ("bash.exe", "the shell and the commands in it stop"),
    ("powershell.exe", "the script in it stops"),
    ("pwsh.exe", "the script in it stops"),
    ("cmd.exe", "the commands in it stop"),
];

/// Never suggested: Windows parts, our own tools, the search index the file manager uses, WebView2 hosts
/// (they belong to other apps), antivirus, and graphics / audio drivers' helpers.
const NEVER: &[&str] = &[
    "everything.exe", "msedgewebview2.exe", "claude.exe", "wintask.exe", "wtm.exe", "wfm.exe", "explorer.exe", "dwm.exe",
    "msmpeng.exe", "nissrv.exe", "searchhost.exe", "ctfmon.exe", "textinputhost.exe", "sihost.exe",
    "audiodg.exe", "fontdrvhost.exe", "lsass.exe", "csrss.exe", "winlogon.exe", "services.exe", "svchost.exe",
];

pub fn free_candidates(groups: &[Group], procs: &[Proc], growing: &[u32], startup: &[crate::startup::Entry]) -> Vec<FreeItem> {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut out = vec![];
    for g in groups {
        if g.mem < 30 * 1048576 {
            continue;
        }
        let Some(first) = g.pids.first().and_then(|p| by_pid.get(p)) else { continue };
        let name = first.name.to_lowercase();
        let path = first.meta.path.to_lowercase();
        let system = g.kind == Kind::Windows || path.is_empty() || path.starts_with(r"c:\windows\");
        let entry = startup.iter().find(|e| !path.is_empty() && e.command.to_lowercase().contains(&path)).map(|e| e.name.clone());
        let restart_hint = g.pids.iter().any(|p| growing.contains(p));
        let (verdict, effect) = if system || NEVER.contains(&name.as_str()) {
            (FreeVerdict::Keep, match name.as_str() {
                "everything.exe" => "the file manager's instant search uses it".into(),
                "msedgewebview2.exe" => "it belongs to another app (its window would break)".into(),
                "claude.exe" => "your Claude agents run inside it".into(),
                _ => "Windows or a tool you use needs it".to_string(),
            })
        } else if let Some((_, e)) = STOPS.iter().find(|(n, _)| *n == name) {
            (FreeVerdict::SafeButStops, e.to_string())
        } else if g.kind == Kind::App {
            (FreeVerdict::SafeButStops, "the app closes; unsaved work may be lost".into())
        } else {
            (FreeVerdict::Safe, "nothing you use stops; it starts again when needed".into())
        };
        out.push(FreeItem { label: g.label.clone(), pids: g.pids.clone(), mem: g.mem, verdict, effect, restart_hint, startup_entry: entry });
    }
    out.sort_by(|a, b| (a.verdict as u8).cmp(&(b.verdict as u8)).then(b.mem.cmp(&a.mem)));
    out
}

// ---------------------------------------------------------------------------------------------- why is this running?
#[derive(Clone, Debug, Default)]
pub struct Why {
    pub parents: Vec<(u32, String)>,
    pub startup: Vec<String>,
    pub services: Vec<String>,
    pub tasks: Vec<String>,
    pub flags: Vec<String>,
}

pub fn why(p: &Proc, procs: &[Proc], services: &[crate::services::Service], startup: &[crate::startup::Entry], signed: Option<bool>) -> Why {
    let mut w = Why::default();
    // parent chain (stop at reused PIDs)
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|q| (q.pid, q)).collect();
    let mut cur = p;
    for _ in 0..8 {
        match by_pid.get(&cur.ppid) {
            Some(par) if par.pid != cur.pid && par.create_time <= cur.create_time && par.pid != 0 => {
                w.parents.push((par.pid, par.name.clone()));
                cur = par;
            }
            _ => break,
        }
    }
    let path = p.meta.path.to_lowercase();
    if !path.is_empty() {
        for e in startup {
            if e.command.to_lowercase().contains(&path) || (e.source == "Scheduled task" && e.command.to_lowercase().contains(&p.name.to_lowercase())) {
                let s = format!("{} ({}{})", e.name, e.source, if e.enabled { "" } else { ", disabled" });
                if e.source == "Scheduled task" { w.tasks.push(s) } else { w.startup.push(s) }
            }
        }
    }
    for s in services.iter().filter(|s| s.pid == p.pid && s.pid != 0) {
        w.services.push(format!("{} ({})", s.display, s.name));
    }
    if signed == Some(false) {
        w.flags.push("not digitally signed".into());
    }
    if path.contains(r"\appdata\local\temp\") || path.contains(r"\windows\temp\") {
        w.flags.push("runs from a Temp folder".into());
    }
    if path.contains(r"\downloads\") {
        w.flags.push("runs from Downloads".into());
    }
    w
}

// ---------------------------------------------------------------------------------------------- restart
/// End the process and start it again with the same command line (and the exe's folder as working dir).
pub fn restart(pid: u32) -> Result<u32, String> {
    use windows::Win32::System::Threading::*;
    let cmd = procs::command_line(pid).ok_or("cannot read its command line (run as administrator?)")?;
    let path = procs::image_path(pid).ok_or("cannot read its path")?;
    if path.to_lowercase().starts_with(r"c:\windows\") {
        return Err("Windows processes are not restarted this way".into());
    }
    let dir = current_dir(pid).unwrap_or_else(|| std::path::Path::new(&path).parent().map(|d| d.display().to_string()).unwrap_or_default());
    procs::terminate(pid)?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut wcmd: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
    let wdir = crate::sys::wide(&dir);
    let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut pi = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(None, Some(windows::core::PWSTR(wcmd.as_mut_ptr())), None, None, false, PROCESS_CREATION_FLAGS(0), None, windows::core::PCWSTR(wdir.as_ptr()), &si, &mut pi).map_err(|e| e.message())?;
        let _ = windows::Win32::Foundation::CloseHandle(pi.hThread);
        let _ = windows::Win32::Foundation::CloseHandle(pi.hProcess);
    }
    Ok(pi.dwProcessId)
}

/// The process's current directory, read from its PEB (x64 layout: PEB+0x20 -> parameters, +0x38 = CurrentDirectory).
pub fn current_dir(pid: u32) -> Option<String> {
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Threading::*;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let read = |addr: usize, buf: &mut [u8]| -> bool { ReadProcessMemory(h, addr as *const _, buf.as_mut_ptr() as *mut _, buf.len(), None).is_ok() };
        let mut pbi = [0usize; 6]; // PROCESS_BASIC_INFORMATION; [1] = PebBaseAddress
        let mut n = 0u32;
        let st = crate::sys::NtQueryInformationProcess(h.0, 0, pbi.as_mut_ptr() as *mut _, size_of_val(&pbi) as u32, &mut n);
        let mut out = None;
        if st >= 0 {
            let mut p8 = [0u8; 8];
            if read(pbi[1] + 0x20, &mut p8) {
                let params = usize::from_le_bytes(p8);
                let mut us = [0u8; 16];
                if read(params + 0x38, &mut us) {
                    let len = u16::from_le_bytes([us[0], us[1]]) as usize;
                    let buf = usize::from_le_bytes(us[8..16].try_into().unwrap());
                    let mut w = vec![0u8; len];
                    if len > 0 && len < 4096 && read(buf, &mut w) {
                        let u: Vec<u16> = w.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                        out = Some(String::from_utf16_lossy(&u));
                    }
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(h);
        out
    }
}

// ---------------------------------------------------------------------------------------------- hung apps
/// PIDs whose top-level window Windows reports as hung ("Not responding").
pub fn hung_pids() -> Vec<u32> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> windows::core::BOOL {
        unsafe {
            let v = &mut *(l.0 as *mut Vec<u32>);
            if IsWindowVisible(h).as_bool() && IsHungAppWindow(h).as_bool() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut pid));
                if !v.contains(&pid) {
                    v.push(pid);
                }
            }
            true.into()
        }
    }
    let mut v: Vec<u32> = vec![];
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// CPU % averaged over the whole life of the process (works on the very first frame, no history needed).
pub fn lifetime_cpu(p: &Proc, now_ft: i64, ncpu: usize) -> f32 {
    let alive = (now_ft - p.create_time).max(1) as f64;
    (p.cpu_time as f64 / (alive * ncpu as f64) * 100.0).clamp(0.0, 100.0) as f32
}

pub fn now_filetime() -> i64 {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_names_the_driver_and_ranks_memory_over_tiny_cpu() {
        let gb16 = 16u64 << 30;
        let everything = impact(0.0, 347 << 20, 0.0, 0.0, gb16);
        let dwm = impact(0.2, 28 << 20, 1500.0, 3.0, gb16);
        assert_eq!(everything.driver, "memory");
        assert!(everything.score > dwm.score, "347 MB idle must outrank 28 MB + 0.2% CPU");
        assert!(impact(12.4, 149 << 20, 0.0, 0.0, gb16).why.starts_with("CPU 12.4%"));
    }

    #[test]
    fn cmdline_split_and_child_labels() {
        assert_eq!(split_cmdline(r#""C:\a b\x.exe" run -c "d:\cfg\config.json""#), vec![r"C:\a b\x.exe", "run", "-c", r"d:\cfg\config.json"]);
        assert_eq!(tail(r"C:\Users\S\tools\smart-proxy\smart_proxy.py"), r"smart-proxy\smart_proxy.py");
        assert_eq!(tail("relay.js"), "relay.js");
    }
}
