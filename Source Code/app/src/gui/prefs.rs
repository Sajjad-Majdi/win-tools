//! Window settings (%LOCALAPPDATA%\WinTools\taskman.json) and the "default Task Manager" switch.
//! The switch writes HKLM\...\Image File Execution Options\taskmgr.exe  Debugger = "<path to wintask.exe>".
//! It needs admin rights, so the window starts an elevated copy of itself (`wintask.exe --set-default` or
//! `--restore-default`) through ShellExecute "runas", and only when the user clicks the button.
use crate::sys::wide;
use serde_json::{Value, json};
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

pub const IFEO: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe";

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// 0 = follow Windows, 1 = light, 2 = dark
    pub theme: u8,
    pub interval_ms: u64,
    pub merge: bool,
    pub ask_end: bool,
    pub show_pid: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { theme: 0, interval_ms: 1000, merge: true, ask_end: true, show_pid: false }
    }
}

fn path() -> std::path::PathBuf {
    crate::ai::config_dir().join("taskman.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        let v: Value = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
        let d = Prefs::default();
        Prefs {
            theme: v["theme"].as_u64().map(|x| x.min(2) as u8).unwrap_or(d.theme),
            interval_ms: v["interval_ms"].as_u64().filter(|x| [500, 1000, 2000, 5000].contains(x)).unwrap_or(d.interval_ms),
            merge: v["merge"].as_bool().unwrap_or(d.merge),
            ask_end: v["ask_end"].as_bool().unwrap_or(d.ask_end),
            show_pid: v["show_pid"].as_bool().unwrap_or(d.show_pid),
        }
    }

    pub fn save(&self) {
        let v = json!({"theme": self.theme, "interval_ms": self.interval_ms, "merge": self.merge, "ask_end": self.ask_end, "show_pid": self.show_pid});
        let _ = std::fs::create_dir_all(crate::ai::config_dir());
        let _ = std::fs::write(path(), serde_json::to_string_pretty(&v).unwrap());
    }
}

/// Is Win Tools set as the Task Manager (IFEO Debugger points at wintask.exe)?
pub fn is_default() -> bool {
    crate::reg::read_string(crate::reg::HKLM, IFEO, "Debugger").map(|d| d.to_lowercase().contains("wintask")).unwrap_or(false)
}

/// Start the elevated helper. Windows shows the UAC prompt; returns false when it could not start (or was declined).
pub fn launch_helper(hwnd: windows::Win32::Foundation::HWND, set: bool) -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = wide(&exe.display().to_string());
    let args = wide(if set { "--set-default" } else { "--restore-default" });
    let verb = wide("runas");
    let r = unsafe { ShellExecuteW(Some(hwnd), PCWSTR(verb.as_ptr()), PCWSTR(exe.as_ptr()), PCWSTR(args.as_ptr()), PCWSTR::null(), SW_HIDE) };
    r.0 as isize > 32
}

/// The elevated helper itself (`wintask.exe --set-default | --restore-default`). Returns the exit code.
pub fn run_helper(set: bool) -> i32 {
    let key = wide(IFEO);
    let name = wide("Debugger");
    unsafe {
        if set {
            let Ok(exe) = std::env::current_exe() else { return 2 };
            let val = wide(&format!("\"{}\"", exe.display()));
            let mut hk = HKEY::default();
            if RegCreateKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut hk, None).is_err() {
                return 3;
            }
            let bytes = std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2);
            let r = RegSetValueExW(hk, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(hk);
            if r.is_err() { 4 } else { 0 }
        } else {
            let r = RegDeleteKeyValueW(HKEY_LOCAL_MACHINE, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()));
            // already gone counts as done
            if r.is_ok() || r.0 == 2 { 0 } else { 5 }
        }
    }
}
