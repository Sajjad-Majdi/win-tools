//! Startup apps: Run keys (HKCU, HKLM, HKLM 32-bit) and the two Startup folders, with the enabled state Explorer
//! keeps in `...\Explorer\StartupApproved\{Run,Run32,StartupFolder}` (first byte even = enabled, odd = disabled).
use crate::reg::{self, HKCU, HKLM};
use windows::Win32::System::Registry::HKEY;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub command: String,
    /// "HKCU Run", "HKLM Run", "HKLM Run32", "User folder", "All users folder"
    pub source: &'static str,
    pub enabled: bool,
}

struct Src {
    label: &'static str,
    root: Option<HKEY>,
    approved_root: HKEY,
    approved_key: &'static str,
}

fn sources() -> [Src; 5] {
    [
        Src { label: "HKCU Run", root: Some(HKCU), approved_root: HKCU, approved_key: "Run" },
        Src { label: "HKLM Run", root: Some(HKLM), approved_root: HKLM, approved_key: "Run" },
        Src { label: "HKLM Run32", root: Some(HKLM), approved_root: HKLM, approved_key: "Run32" },
        Src { label: "User folder", root: None, approved_root: HKCU, approved_key: "StartupFolder" },
        Src { label: "All users folder", root: None, approved_root: HKLM, approved_key: "StartupFolder" },
    ]
}

fn folder(label: &str) -> Option<std::path::PathBuf> {
    let base = if label == "User folder" { std::env::var("APPDATA").ok()? } else { std::env::var("ProgramData").ok()? };
    Some(std::path::Path::new(&base).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
}

fn enabled(src: &Src, name: &str) -> bool {
    match reg::read_binary(src.approved_root, &format!(r"{APPROVED}\{}", src.approved_key), name) {
        Some(b) if !b.is_empty() => b[0] & 1 == 0,
        _ => true,
    }
}

pub fn list() -> Vec<Entry> {
    let mut out = vec![];
    for s in sources() {
        if let Some(root) = s.root {
            let key = if s.label == "HKLM Run32" { RUN32 } else { RUN };
            for (name, cmd) in reg::values(root, key) {
                let en = enabled(&s, &name);
                out.push(Entry { name, command: cmd, source: s.label, enabled: en });
            }
        } else if let Some(dir) = folder(s.label)
            && let Ok(rd) = std::fs::read_dir(&dir)
        {
            for f in rd.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                if name.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                let en = enabled(&s, &name);
                out.push(Entry { name, command: f.path().display().to_string(), source: s.label, enabled: en });
            }
        }
    }
    out
}

/// Everything that starts with Windows: Run keys and Startup folders, plus scheduled tasks with a logon/boot
/// trigger (outside \Microsoft) and auto-start services whose program is not part of Windows.
pub fn list_all() -> Vec<Entry> {
    let mut v = list();
    v.extend(scheduled_tasks());
    v.extend(auto_services());
    v
}

/// Scheduled tasks that run at logon or boot, not under \Microsoft\ (Task Scheduler COM).
pub fn scheduled_tasks() -> Vec<Entry> {
    use windows::Win32::System::Com::*;
    use windows::Win32::System::TaskScheduler::*;
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::BSTR;
    let mut out = vec![];
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(svc) = CoCreateInstance::<_, ITaskService>(&TaskScheduler, None, CLSCTX_INPROC_SERVER) else { return out };
        let e = VARIANT::default();
        if svc.Connect(&e, &e, &e, &e).is_err() {
            return out;
        }
        let Ok(root) = svc.GetFolder(&BSTR::from("\\")) else { return out };
        let mut stack = vec![root];
        while let Some(f) = stack.pop() {
            let path = f.Path().map(|p| p.to_string()).unwrap_or_default();
            if path.starts_with("\\Microsoft") {
                continue;
            }
            if let Ok(subs) = f.GetFolders(0) {
                for i in 1..=subs.Count().unwrap_or(0) {
                    if let Ok(sf) = subs.get_Item(&VARIANT::from(i)) {
                        stack.push(sf);
                    }
                }
            }
            let Ok(tasks) = f.GetTasks(TASK_ENUM_HIDDEN.0) else { continue };
            for i in 1..=tasks.Count().unwrap_or(0) {
                let Ok(t) = tasks.get_Item(&VARIANT::from(i)) else { continue };
                let Ok(def) = t.Definition() else { continue };
                let mut at_logon = false;
                if let Ok(trs) = def.Triggers() {
                    let mut n = 0;
                    let _ = trs.Count(&mut n);
                    for k in 1..=n {
                        if let Ok(tr) = trs.get_Item(k) {
                            let mut ty = TASK_TRIGGER_TYPE2::default();
                            let _ = tr.Type(&mut ty);
                            if ty == TASK_TRIGGER_LOGON || ty == TASK_TRIGGER_BOOT {
                                at_logon = true;
                            }
                        }
                    }
                }
                if !at_logon {
                    continue;
                }
                let mut cmd = String::new();
                if let Ok(acts) = def.Actions() {
                    let mut n = 0;
                    let _ = acts.Count(&mut n);
                    for k in 1..=n {
                        if let Ok(a) = acts.get_Item(k)
                            && let Ok(x) = windows::core::Interface::cast::<IExecAction>(&a)
                        {
                            let (mut p, mut ar) = (BSTR::new(), BSTR::new());
                            let _ = x.Path(&mut p);
                            let _ = x.Arguments(&mut ar);
                            cmd = format!("\"{}\" {}", p.to_string().trim_matches('"'), ar).trim().to_string();
                            break;
                        }
                    }
                }
                let name = t.Path().map(|p| p.to_string()).unwrap_or_default();
                let enabled = t.Enabled().map(|b| b.as_bool()).unwrap_or(false);
                out.push(Entry { name, command: cmd, source: "Scheduled task", enabled });
            }
        }
    }
    out
}

/// Auto-start services whose program is outside C:\Windows (third-party background services).
pub fn auto_services() -> Vec<Entry> {
    let mut out = vec![];
    for s in crate::services::list() {
        let Some((start, bin, _)) = crate::services::details(&s.name) else { continue };
        if start != "auto" {
            continue;
        }
        let b = bin.to_lowercase();
        if b.contains(r"c:\windows\") || b.contains(r"%systemroot%") || b.starts_with(r"\systemroot") || b.contains(r"\windows defender\") {
            continue;
        }
        out.push(Entry { name: s.display.clone(), command: bin, source: "Service", enabled: true });
    }
    out
}

fn set_task_enabled(path: &str, on: bool) -> Result<(), String> {
    use windows::Win32::System::Com::*;
    use windows::Win32::System::TaskScheduler::*;
    use windows::Win32::System::Variant::VARIANT;
    use windows::core::BSTR;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let svc: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message())?;
        let e = VARIANT::default();
        svc.Connect(&e, &e, &e, &e).map_err(|e| e.message())?;
        let (folder, name) = path.rsplit_once('\\').unwrap_or(("", path));
        let f = svc.GetFolder(&BSTR::from(if folder.is_empty() { "\\" } else { folder })).map_err(|e| e.message())?;
        let t = f.GetTask(&BSTR::from(name)).map_err(|e| e.message())?;
        t.SetEnabled(windows::Win32::Foundation::VARIANT_BOOL(if on { -1 } else { 0 })).map_err(|e| {
            if e.code().0 as u32 == 0x80070005 { "access denied (this task needs administrator rights)".to_string() } else { e.message() }
        })
    }
}

/// Enable or disable an entry the way Explorer / Task Manager do (writes only the StartupApproved flag).
/// Scheduled tasks are enabled/disabled in Task Scheduler. Services are not changed here (that needs admin).
pub fn set_enabled(name: &str, on: bool) -> Result<Entry, String> {
    set_enabled_in(name, None, on)
}

/// Like `set_enabled`, for the entry in one place (`source`, e.g. "Registry (user)") when a name is in several.
pub fn set_enabled_in(name: &str, source: Option<&str>, on: bool) -> Result<Entry, String> {
    let all = list_all();
    let matches: Vec<&Entry> = all
        .iter()
        .filter(|e| source.map(|s| e.source == s).unwrap_or(true))
        .filter(|e| e.name.eq_ignore_ascii_case(name) || (e.source == "Scheduled task" && e.name.rsplit('\\').next().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)))
        .collect();
    let e = match matches.as_slice() {
        [] => return Err(format!("no startup entry named '{name}'")),
        [e] => *e,
        _ => return Err(format!("'{name}' is in several places; names must be unique")),
    };
    if e.source == "Scheduled task" {
        set_task_enabled(&e.name, on)?;
        return Ok(Entry { enabled: on, ..e.clone() });
    }
    if e.source == "Service" {
        return Err("services are changed with admin rights: sc config <name> start= demand".into());
    }
    let src = sources().into_iter().find(|s| s.label == e.source).unwrap();
    let mut data = vec![if on { 2u8 } else { 3u8 }, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    if !on {
        // bytes 4..12 = FILETIME when it was disabled
        let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
        let v = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        data[4..12].copy_from_slice(&v.to_le_bytes());
    }
    reg::write_binary(src.approved_root, &format!(r"{APPROVED}\{}", src.approved_key), &e.name, &data).map_err(|err| {
        if src.approved_root == HKLM { format!("{err} (machine-wide entry: run as administrator)") } else { err }
    })?;
    Ok(Entry { enabled: on, ..e.clone() })
}
