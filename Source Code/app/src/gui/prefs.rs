//! Window settings (%LOCALAPPDATA%\WinTools\taskman.json) and the "default Task Manager" switch.
//! The switch writes HKLM\...\Image File Execution Options\taskmgr.exe  Debugger = "<path to RustTaskManager.exe>".
//! It needs admin rights, so the window starts an elevated copy of itself (`RustTaskManager.exe --set-default` or
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
    /// 0 Mica, 1 Mica Alt, 2 Acrylic, 3 solid
    pub backdrop: u8,
    /// surface transparency: 0 off, 1 low, 2 medium, 3 high
    pub transp: u8,
    /// accent base colour 0xRRGGBB; None = the Windows accent
    pub accent: Option<u32>,
    /// 0 compact, 1 comfortable
    pub density: u8,
    /// text size: 0 small, 1 medium, 2 large
    pub font: u8,
    /// 0 rounded, 1 square
    pub corners: u8,
    /// optional list columns: Type, Disk, Network, GPU (PID is `show_pid`)
    pub col_type: bool,
    pub col_disk: bool,
    pub col_net: bool,
    pub col_gpu: bool,
    /// the resource summary card above the list, and the attention chips
    pub tiles: bool,
    pub chips: bool,
    /// sort when the window opens (SortKey::parse names)
    pub sort: String,
    /// side pane width: 0 narrow, 1 medium, 2 wide
    pub pane: u8,
    /// list in sections (Apps, Background processes, Windows processes)
    pub group: bool,
    /// sections showing all rows (else the top 5 + "Show all")
    pub sec_open: [bool; 3],
    /// Startup page sections (Apps, Scheduled tasks, Services): all rows shown (else the top 5), and folded shut
    pub st_all: [bool; 3],
    pub st_fold: [bool; 3],
    /// sidebar expanded (icon + label) or collapsed (icons)
    pub nav_open: bool,
    /// the accent follows the desktop (wallpaper / DWM colour) instead of `accent`; one choice, never the default
    pub desktop: bool,
    /// how strongly the whole window takes the accent hue: 0 subtle, 1 medium, 2 strong
    pub strength: u8,
}

/// Three flags from a JSON array (missing = false).
fn bools(v: &Value) -> [bool; 3] {
    let mut o = [false; 3];
    if let Some(a) = v.as_array() {
        for (i, x) in a.iter().take(3).enumerate() {
            o[i] = x.as_bool().unwrap_or(false);
        }
    }
    o
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: 0,
            interval_ms: 1000,
            merge: true,
            ask_end: true,
            show_pid: false,
            backdrop: 0,
            transp: 1,
            accent: None,
            density: 1,
            font: 1,
            corners: 0,
            col_type: true,
            col_disk: true,
            col_net: true,
            col_gpu: true,
            tiles: true,
            chips: true,
            sort: "impact".into(),
            pane: 1,
            group: true,
            sec_open: [false; 3],
            st_all: [false; 3],
            st_fold: [false; 3],
            nav_open: true,
            desktop: false,
            strength: 1,
        }
    }
}

fn path() -> std::path::PathBuf {
    crate::ai::config_dir().join("taskman.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        let v: Value = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
        Prefs::from_json(&v)
    }

    /// Missing or bad values fall back to the defaults one by one.
    pub fn from_json(v: &Value) -> Prefs {
        let d = Prefs::default();
        let small = |k: &str, max: u64, def: u8| v[k].as_u64().filter(|x| *x <= max).map(|x| x as u8).unwrap_or(def);
        let flag = |k: &str, def: bool| v[k].as_bool().unwrap_or(def);
        Prefs {
            theme: v["theme"].as_u64().map(|x| x.min(2) as u8).unwrap_or(d.theme),
            interval_ms: v["interval_ms"].as_u64().filter(|x| [500, 1000, 2000, 5000].contains(x)).unwrap_or(d.interval_ms),
            merge: flag("merge", d.merge),
            ask_end: flag("ask_end", d.ask_end),
            show_pid: flag("show_pid", d.show_pid),
            backdrop: small("backdrop", 3, d.backdrop),
            transp: small("transparency", 3, d.transp),
            // 0.2.x had a separate "Window colour" with a custom hue: an old custom hue becomes the accent
            accent: v["accent"].as_str().and_then(parse_hex).or_else(|| {
                (v["window_color"].as_u64() == Some(3)).then(|| {
                    let h = v["window_hue"].as_u64().unwrap_or(210) as f32;
                    let c = super::theme::from_hsl(h, 0.78, 0.45);
                    ((c.0 * 255.0).round() as u32) << 16 | ((c.1 * 255.0).round() as u32) << 8 | (c.2 * 255.0).round() as u32
                })
            }),
            density: small("density", 1, d.density),
            font: small("text_size", 2, d.font),
            corners: small("corners", 1, d.corners),
            col_type: flag("col_type", d.col_type),
            col_disk: flag("col_disk", d.col_disk),
            col_net: flag("col_network", d.col_net),
            col_gpu: flag("col_gpu", d.col_gpu),
            tiles: flag("summary", d.tiles),
            chips: flag("chips", d.chips),
            sort: v["sort"].as_str().filter(|s| super::model::SortKey::parse(s).is_some()).map(|s| s.to_lowercase()).unwrap_or(d.sort),
            pane: small("pane_width", 2, d.pane),
            group: flag("group", d.group),
            nav_open: flag("nav_open", d.nav_open),
            desktop: flag("accent_desktop", d.desktop),
            // the old "Neutral" window colour becomes Subtle
            strength: v["colour_strength"].as_u64().filter(|x| *x <= 2).map(|x| x as u8).unwrap_or(if v["window_color"].as_u64() == Some(2) { 0 } else { d.strength }),
            st_all: bools(&v["startup_all"]),
            st_fold: bools(&v["startup_collapsed"]),
            sec_open: {
                let mut o = d.sec_open;
                if let Some(a) = v["sections_open"].as_array() {
                    for (i, x) in a.iter().take(3).enumerate() {
                        o[i] = x.as_bool().unwrap_or(false);
                    }
                }
                o
            },
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "theme": self.theme, "interval_ms": self.interval_ms, "merge": self.merge, "ask_end": self.ask_end,
            "show_pid": self.show_pid, "backdrop": self.backdrop, "transparency": self.transp,
            "accent": self.accent.map(|a| format!("#{a:06X}")), "density": self.density, "text_size": self.font,
            "corners": self.corners, "col_type": self.col_type, "col_disk": self.col_disk, "col_network": self.col_net,
            "col_gpu": self.col_gpu, "summary": self.tiles, "chips": self.chips, "sort": self.sort, "pane_width": self.pane,
            "group": self.group, "sections_open": self.sec_open, "startup_all": self.st_all, "startup_collapsed": self.st_fold, "nav_open": self.nav_open,
            "accent_desktop": self.desktop, "colour_strength": self.strength,
        })
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(crate::ai::config_dir());
        let _ = std::fs::write(path(), serde_json::to_string_pretty(&self.to_json()).unwrap());
    }
}

/// "#12AB34" / "12ab34" -> 0x12AB34
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_round_trip_and_defaults() {
        let d = Prefs::default();
        assert_eq!(Prefs::from_json(&json!({})), d);
        let mut p = d.clone();
        p.backdrop = 2;
        p.transp = 3;
        p.accent = Some(0x12AB34);
        p.density = 0;
        p.font = 2;
        p.corners = 1;
        p.col_net = false;
        p.tiles = false;
        p.sort = "memory".into();
        p.pane = 2;
        p.group = false;
        p.nav_open = false;
        p.desktop = true;
        p.strength = 2;
        p.sec_open = [true, false, true];
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        // bad values fall back one by one
        let bad = Prefs::from_json(&json!({"backdrop": 9, "accent": "zz", "sort": "colour", "density": 0}));
        assert_eq!((bad.backdrop, bad.accent, bad.sort.as_str(), bad.density), (0, None, "impact", 0));
        assert_eq!(parse_hex("#0078d4"), Some(0x0078D4));
        // 0.2.x values migrate: a custom window hue becomes the accent, Neutral becomes Subtle
        let old = Prefs::from_json(&json!({"window_color": 3, "window_hue": 120}));
        assert!(old.accent.is_some() && !old.desktop && old.strength == 1);
        assert_eq!(Prefs::from_json(&json!({"window_color": 2})).strength, 0);
        assert_eq!(Prefs::from_json(&json!({"window_color": 0})).desktop, false);
    }
}

/// Is Rust Task Manager set as the Task Manager (IFEO Debugger points at RustTaskManager.exe, or wintask.exe before 0.2.0)?
pub fn is_default() -> bool {
    crate::reg::read_string(crate::reg::HKLM, IFEO, "Debugger").map(|d| { let d = d.to_lowercase(); d.contains("rusttaskmanager") || d.contains("wintask") }).unwrap_or(false)
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

/// Run this exe elevated with `args` (one UAC prompt) and wait for it. Ok(exit code); Err("cancelled") when the
/// user said No to the prompt; Err(text) when it could not start.
pub fn run_elevated_wait(hwnd: windows::Win32::Foundation::HWND, args: &str) -> Result<u32, String> {
    use windows::Win32::Foundation::{CloseHandle, GetLastError};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = wide(&exe.display().to_string());
    let a = wide(args);
    let verb = wide("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(a.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if ShellExecuteExW(&mut info).is_err() {
            // 1223 = ERROR_CANCELLED: the user answered No in the UAC prompt
            return Err(if GetLastError().0 == 1223 { "cancelled".into() } else { format!("could not start the administrator helper ({})", GetLastError().0) });
        }
        if info.hProcess.is_invalid() {
            return Err("the administrator helper did not start".into());
        }
        let _ = WaitForSingleObject(info.hProcess, 30_000);
        let mut code = 0u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        Ok(code)
    }
}

/// The elevated service helper (`RustTaskManager.exe --service-start <name> auto|auto-delayed|manual`): 0 = done,
/// 3 = access denied, 4 = other failure, 2 = bad arguments.
pub fn run_service_helper(args: &[String]) -> i32 {
    let Some(i) = args.iter().position(|a| a == "--service-start") else { return 2 };
    let (Some(name), Some(mode)) = (args.get(i + 1), args.get(i + 2)) else { return 2 };
    let (auto, delayed) = match mode.as_str() {
        "auto" => (true, false),
        "auto-delayed" => (true, true),
        "manual" => (false, false),
        _ => return 2,
    };
    match crate::services::set_start(name, auto, delayed) {
        Ok(()) => 0,
        Err(e) if e.contains("access denied") => 3,
        Err(_) => 4,
    }
}

/// The elevated helper itself (`RustTaskManager.exe --set-default | --restore-default`). Returns the exit code.
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
