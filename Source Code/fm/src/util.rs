//! Shared helpers: wide strings, FILETIME conversion, size and time text, hidden child processes, the data folder.
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::core::PWSTR;

/// Errors in this crate are short readable sentences (they go straight to the CLI and the GUI).
pub type Res<T> = Result<T, String>;

/// CREATE_NO_WINDOW: a child console process gets no window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// UTF-16 with a trailing NUL.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// UTF-16 path with a trailing NUL. Long absolute paths (>= 248 chars) get the `\\?\` prefix so Win32 file APIs accept them.
pub fn wide_path(p: &Path) -> Vec<u16> {
    let s = p.as_os_str();
    let len = s.encode_wide().count();
    let raw = s.encode_wide().take(4).collect::<Vec<u16>>();
    let has_prefix = raw.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]);
    if len >= 248 && !has_prefix && p.is_absolute() {
        let st = p.to_string_lossy();
        let pre = if let Some(unc) = st.strip_prefix(r"\\") { format!(r"\\?\UNC\{unc}") } else { format!(r"\\?\{st}") };
        return wide(pre);
    }
    wide(s)
}

/// String from a NUL-terminated (or full) UTF-16 buffer.
pub fn from_wide(p: &[u16]) -> String {
    let n = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    String::from_utf16_lossy(&p[..n])
}

/// String from a PWSTR the shell allocated; frees it with CoTaskMemFree.
pub fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let s = p.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

pub fn ft_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

pub fn u64_ft(v: u64) -> FILETIME {
    FILETIME { dwLowDateTime: v as u32, dwHighDateTime: (v >> 32) as u32 }
}

/// FILETIME (100 ns since 1601) -> Unix seconds. 0 stays 0.
pub fn ft_unix(ft: u64) -> i64 {
    if ft == 0 { 0 } else { (ft / 10_000_000) as i64 - 11_644_473_600 }
}

/// Unix seconds -> FILETIME.
pub fn unix_ft(secs: i64) -> u64 {
    ((secs + 11_644_473_600).max(0) as u64) * 10_000_000
}

/// Now as FILETIME.
pub fn now_ft() -> u64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    unix_ft(d.as_secs() as i64) + (d.subsec_nanos() / 100) as u64
}

/// Local time text "YYYY-MM-DD HH:MM" (empty for 0).
pub fn fmt_time(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let mut st = SYSTEMTIME::default();
    let mut lt = SYSTEMTIME::default();
    unsafe {
        let f = u64_ft(ft);
        if windows::Win32::System::Time::FileTimeToSystemTime(&f, &mut st).is_err() {
            return String::new();
        }
        if windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime(None, &st, &mut lt).is_err() {
            lt = st;
        }
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", lt.wYear, lt.wMonth, lt.wDay, lt.wHour, lt.wMinute)
}

/// "1.4 GB" style size (1024 based, like Explorer).
pub fn fmt_bytes(n: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else if v < 10.0 { format!("{v:.1} {}", U[i]) } else { format!("{v:.0} {}", U[i]) }
}

/// Parse "10mb", "1.5 GB", "512k", "100" (bytes) -> bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let t = s.trim().to_ascii_lowercase().replace(' ', "");
    let idx = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(idx);
    let v: f64 = num.parse().ok()?;
    let m: f64 = match unit.trim_end_matches('b') {
        "" => 1.0,
        "k" | "ki" => 1024.0,
        "m" | "mi" => 1024.0 * 1024.0,
        "g" | "gi" => 1024.0 * 1024.0 * 1024.0,
        "t" | "ti" => 1024.0f64.powi(4),
        _ => return None,
    };
    Some((v * m) as u64)
}

pub const ATTR_READONLY: u32 = 0x1;
pub const ATTR_HIDDEN: u32 = 0x2;
pub const ATTR_SYSTEM: u32 = 0x4;
pub const ATTR_DIRECTORY: u32 = 0x10;
pub const ATTR_ARCHIVE: u32 = 0x20;
pub const ATTR_REPARSE: u32 = 0x400;
pub const ATTR_COMPRESSED: u32 = 0x800;
pub const ATTR_OFFLINE: u32 = 0x1000;
pub const ATTR_ENCRYPTED: u32 = 0x4000;
/// Cloud placeholder (OneDrive "online only"): reading it would download the file.
pub const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// Attribute letters, Explorer style: R H S A D L C E O.
pub fn attr_string(a: u32) -> String {
    let mut s = String::new();
    for (bit, ch) in [
        (ATTR_READONLY, 'R'), (ATTR_HIDDEN, 'H'), (ATTR_SYSTEM, 'S'), (ATTR_ARCHIVE, 'A'), (ATTR_DIRECTORY, 'D'),
        (ATTR_REPARSE, 'L'), (ATTR_COMPRESSED, 'C'), (ATTR_ENCRYPTED, 'E'), (ATTR_OFFLINE, 'O'),
    ] {
        if a & bit != 0 {
            s.push(ch);
        }
    }
    s
}

/// A Command that never shows a window (CREATE_NO_WINDOW).
pub fn hidden_command(exe: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(exe);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Where fm.json and fm-journal.json live: %LOCALAPPDATA%\WinTools (or $WFM_DATA_DIR, used by tests).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("WFM_DATA_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("WinTools")
}

/// Make a path absolute (without touching the disk) and drop a trailing separator (but keep "C:\").
pub fn abs(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    let a = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = a.to_string_lossy();
    if s.len() > 3 && (s.ends_with('\\') || s.ends_with('/')) {
        return PathBuf::from(s.trim_end_matches(['\\', '/']));
    }
    a
}

/// Text for the last Win32 error, with a context prefix.
pub fn win_err(ctx: &str, e: &windows::core::Error) -> String {
    let m = e.message();
    let m = m.trim();
    if m.is_empty() { format!("{ctx}: error 0x{:08X}", e.code().0) } else { format!("{ctx}: {m}") }
}

/// Initialise COM (apartment threaded) for the calling thread; uninitialises on drop when this call started it.
pub struct Com(bool);
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}
pub fn com() -> Com {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    Com(hr.is_ok())
}

/// Never show "insert a disk" / critical-error boxes for empty card readers and DVD drives.
pub fn quiet_errors() {
    use windows::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, SetErrorMode};
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
}

/// Lower-case extension without the dot ("" for none).
pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// Same volume? (compares the root of both absolute paths, case-insensitive)
pub fn same_volume(a: &Path, b: &Path) -> bool {
    fn root(p: &Path) -> String {
        use std::path::Component;
        match p.components().next() {
            Some(Component::Prefix(pre)) => pre.as_os_str().to_string_lossy().to_lowercase(),
            _ => String::new(),
        }
    }
    let (ra, rb) = (root(a), root(b));
    !ra.is_empty() && ra == rb
}

/// Case-insensitive path equality (Windows file names are case-insensitive).
pub fn path_eq(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}
