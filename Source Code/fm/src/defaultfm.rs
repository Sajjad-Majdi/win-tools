//! "Set as default file manager" / "Restore Explorer", per user (HKCU only, no admin rights).
//!
//! `set` makes folders, drives, Win+E and the This PC / Recycle Bin desktop icons open in RustFileExplorer.exe:
//! - `HKCU\Software\Classes\Directory\shell` and `...\Drive\shell`: the default verb is the canonical `open`, made ours
//!   (`"<RustFileExplorer.exe>" "%1"`, empty `DelegateExecute`), so Windows 11's menu shows one "Open" entry; a second
//!   verb `openinexplorer` ("Open in File Explorer") keeps Windows' Explorer one click away;
//! - `HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command` (Win+E);
//! - `HKCU\Software\Classes\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\shell\open\command` (This PC) and
//!   `...\CLSID\{645FF040-5081-101B-9F08-08002B27103E}\shell\open\command` (Recycle Bin): `"<exe>" "::{CLSID}"`, with
//!   an empty `DelegateExecute` value so the per-user command wins over Explorer's delegate (as Files / Explorer++ do).
//! `restore` deletes exactly those keys again, so Explorer is the handler once more.
//! Only the GUI's Settings buttons (a click by the user) and `wfm default set|restore --yes` run them. Every function has
//! an `_in(root, ...)` form: `root` replaces `Software\Classes` (tests use `Software\RustFileExplorerTest\Classes`).
use crate::util::{Res, wide};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

/// The real per-user classes root.
pub const CLASSES: &str = r"Software\Classes";
/// This PC and Recycle Bin (desktop icons, "shell:" names).
pub const THIS_PC: &str = "{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
pub const RECYCLE_BIN: &str = "{645FF040-5081-101B-9F08-08002B27103E}";
const MARK: &str = "WinToolsFiles";

const VERB: &str = "openinwfiles";
/// "Open in File Explorer": Windows' own Explorer, kept one click away while folders open here.
const EXPLORER_VERB: &str = "openinexplorer";
/// The delegate Explorer itself registers for Folder\shell\open (opens the item in an Explorer window).
const EXPLORER_DELEGATE: &str = "{11dbb47c-a525-400b-9e80-a54615a090c0}";
const WIN_E: &str = r"CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";

fn set_value(path: &str, name: &str, value: &str) -> Res<()> {
    let p = wide(path);
    let n = wide(name);
    let v = wide(value);
    unsafe {
        let mut key = HKEY::default();
        let r = RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None);
        if r != ERROR_SUCCESS {
            return Err(format!("cannot write HKCU\\{path}: error {}", r.0));
        }
        let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
        let r = RegSetValueExW(key, if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) }, None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        if r != ERROR_SUCCESS {
            return Err(format!("cannot write HKCU\\{path}: error {}", r.0));
        }
    }
    Ok(())
}

fn get_value(path: &str, name: &str) -> Option<String> {
    get_value_in(HKEY_CURRENT_USER, path, name)
}

/// A string value under any root (HKEY_CLASSES_ROOT = the merged HKCU + HKLM view Windows really uses).
fn get_value_in(root: HKEY, path: &str, name: &str) -> Option<String> {
    let p = wide(path);
    let n = wide(name);
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        let r = RegGetValueW(root, PCWSTR(p.as_ptr()), if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) }, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size));
        if r != ERROR_SUCCESS {
            return None;
        }
    }
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn delete_tree(path: &str) {
    let p = wide(path);
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()));
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()));
    }
}

/// Delete a key only when nothing is left in it (no subkeys, no values): keys the user had stay.
fn delete_if_empty(path: &str) {
    let p = wide(path);
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, KEY_READ, &mut key) != ERROR_SUCCESS {
            return;
        }
        let (mut subkeys, mut values) = (0u32, 0u32);
        let r = RegQueryInfoKeyW(key, None, None, None, Some(&mut subkeys), None, None, Some(&mut values), None, None, None, None);
        let _ = RegCloseKey(key);
        if r == ERROR_SUCCESS && subkeys == 0 && values == 0 {
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()));
        }
    }
}

/// What `set` has made ours, part by part (read only).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub folders: bool,
    pub drives: bool,
    pub win_e: bool,
    pub this_pc: bool,
    /// the Win+E helper (RustFileExplorerWinE.exe) starts at sign-in; always true for a test root
    pub win_e_key: bool,
}

impl Status {
    pub fn all(&self) -> bool {
        self.folders && self.drives && self.win_e && self.this_pc && self.win_e_key
    }
    pub fn parts(&self) -> [(&'static str, bool); 5] {
        [("folders", self.folders), ("drives", self.drives), ("win_e", self.win_e), ("this_pc", self.this_pc), ("win_e_key", self.win_e_key)]
    }
}

pub fn status() -> Status {
    status_in(CLASSES)
}

/// The Recycle Bin's own shell folder adds a built-in "open" that wins over a registry verb of the same name, so
/// its verb has its own name; This PC keeps "open" (it works there).
const RB_VERB: &str = "openinrfe";

fn clsid_verb(id: &str) -> &'static str {
    if id == RECYCLE_BIN { RB_VERB } else { "open" }
}

pub fn status_in(root: &str) -> Status {
    let verb = |class: &str| {
        let shell = format!(r"{root}\{class}\shell");
        get_value(&shell, "").map(|d| d.eq_ignore_ascii_case("open")).unwrap_or(false) && get_value(&format!(r"{shell}\open"), MARK).is_some() && get_value(&format!(r"{shell}\open\command"), "").is_some()
    };
    // ours AND effective: our open verb is there and it is the default verb Windows sees. On the real root the
    // merged HKEY_CLASSES_ROOT is read (HKLM sets This PC's default verb to "none", which beats a lone HKCU verb).
    let clsid = |id: &str| {
        let v = clsid_verb(id);
        let ours = get_value(&format!(r"{root}\CLSID\{id}\shell\{v}"), MARK).is_some() && get_value(&format!(r"{root}\CLSID\{id}\shell\{v}\command"), "").is_some();
        let default_verb = if root == CLASSES { get_value_in(HKEY_CLASSES_ROOT, &format!(r"CLSID\{id}\shell"), "") } else { get_value(&format!(r"{root}\CLSID\{id}\shell"), "") };
        let cmd = if root == CLASSES { get_value_in(HKEY_CLASSES_ROOT, &format!(r"CLSID\{id}\shell\{v}\command"), "") } else { get_value(&format!(r"{root}\CLSID\{id}\shell\{v}\command"), "") };
        ours && default_verb.as_deref().map(|d| d.eq_ignore_ascii_case(v)).unwrap_or(false) && cmd.map(|c| c.contains(&format!("::{id}"))).unwrap_or(false)
    };
    Status {
        folders: verb("Directory"),
        drives: verb("Drive"),
        win_e: get_value(&format!(r"{root}\{WIN_E}"), MARK).is_some(),
        this_pc: clsid(THIS_PC),
        win_e_key: root != CLASSES || get_value(RUN, HELPER_NAME).is_some(),
    }
}

/// Windows 11 (Nov 2026 builds) opens Explorer on Win+E without reading the registry verb: a tiny helper next to
/// RustFileExplorer.exe takes the key instead. It starts at sign-in from this Run value.
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const HELPER_NAME: &str = "RustFileExplorerWinE";

fn helper_of(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    exe.parent().map(|d| d.join("RustFileExplorerWinE.exe")).filter(|p| p.is_file())
}

fn delete_value(path: &str, name: &str) {
    let p = wide(path);
    let n = wide(name);
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, KEY_SET_VALUE, &mut key) == ERROR_SUCCESS {
            let _ = RegDeleteValueW(key, PCWSTR(n.as_ptr()));
            let _ = RegCloseKey(key);
        }
    }
}

/// Start (or with `--quit` stop) the helper, hidden, without waiting.
fn run_helper(helper: &std::path::Path, args: &[&str]) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new(helper).args(args).creation_flags(0x0800_0000 | 0x0000_0008).spawn();
}

/// Is Rust File Explorer the per-user default for folders right now? (read only)
pub fn is_default() -> bool {
    status().folders
}

/// Make `exe` (full path of RustFileExplorer.exe) the default for folders, drives, Win+E and This PC.
pub fn set(exe: &std::path::Path) -> Res<()> {
    let r = set_in(CLASSES, exe);
    assoc_changed();
    if r.is_ok()
        && let Some(h) = helper_of(exe)
    {
        set_value(RUN, HELPER_NAME, &format!("\"{}\"", h.display()))?;
        run_helper(&h, &[]);
    }
    r
}

/// Tell the shell that handlers changed: the running Explorer drops its cached verbs (Win+E kept the old handler
/// until the next sign-in without this).
fn assoc_changed() {
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

pub fn set_in(root: &str, exe: &std::path::Path) -> Res<()> {
    let exe = exe.to_string_lossy();
    if !exe.to_lowercase().ends_with(".exe") {
        return Err("needs the full path of RustFileExplorer.exe".into());
    }
    let explorer = format!(r"{}\Explorer.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\WINDOWS".into()));
    for class in ["Directory", "Drive"] {
        let base = format!(r"{root}\{class}\shell");
        // Windows 11's menu always shows its own "Open" (Enter) plus the default verb when that verb is not "open":
        // two entries. So ours IS "open": one "Open" entry that opens here (as Files does). A verb from 0.2.x goes.
        delete_tree(&format!(r"{base}\{VERB}"));
        let open = format!(r"{base}\open");
        // never over a user's own "open" (a command already there that is not ours)
        if get_value(&format!(r"{open}\command"), "").is_some() && get_value(&open, MARK).is_none() {
            return Err(format!(r"HKCU\{open} already has a command that is not ours"));
        }
        delete_tree(&open); // drops the ProgrammaticAccessOnly / delegate values an earlier build wrote
        set_value(&open, "Icon", &format!("{exe},0"))?;
        set_value(&open, "MultiSelectModel", "Document")?;
        set_value(&open, MARK, "1")?;
        set_value(&format!(r"{open}\command"), "", &format!("\"{exe}\" \"%1\""))?;
        // an empty DelegateExecute makes our command win over Explorer's delegate in the merged view
        set_value(&format!(r"{open}\command"), "DelegateExecute", "")?;
        set_value(&base, "", "open")?;
        let ex = format!(r"{base}\{EXPLORER_VERB}");
        set_value(&ex, "", "Open in File Explorer")?;
        set_value(&ex, "Icon", "explorer.exe,0")?;
        set_value(&ex, "MultiSelectModel", "Document")?;
        set_value(&ex, MARK, "1")?;
        set_value(&format!(r"{ex}\command"), "", &explorer)?;
        set_value(&format!(r"{ex}\command"), "DelegateExecute", EXPLORER_DELEGATE)?;
    }
    let we = format!(r"{root}\{WIN_E}");
    set_value(&format!(r"{we}\shell\opennewwindow\command"), "", &format!("\"{exe}\""))?;
    set_value(&format!(r"{we}\shell\opennewwindow\command"), "DelegateExecute", "")?;
    set_value(&we, MARK, "1")?;
    // Recycle Bin: Windows 11 opens the desktop icon by itself and ignores registry verbs, so it is not set any more
    // (Sajjad, 2026-11-19); keys an earlier 0.2.1 build wrote are removed here.
    let rb = format!(r"{root}\CLSID\{RECYCLE_BIN}\shell");
    if get_value(&rb, MARK).is_some() || get_value(&format!(r"{rb}\open"), MARK).is_some() || get_value(&format!(r"{rb}\{RB_VERB}"), MARK).is_some() {
        delete_tree(&rb);
        delete_if_empty(&format!(r"{root}\CLSID\{RECYCLE_BIN}"));
    }
    for id in [THIS_PC] {
        let shell = format!(r"{root}\CLSID\{id}\shell");
        // a verb from an earlier version (Recycle Bin used "open" before 0.2.1) goes first
        if get_value(&shell, MARK).is_some() {
            delete_tree(&shell);
        }
        let v = clsid_verb(id);
        let open = format!(r"{shell}\{v}");
        set_value(&open, "", "Open in Rust File Explorer")?;
        set_value(&format!(r"{open}\command"), "", &format!("\"{exe}\" \"::{id}\""))?;
        set_value(&format!(r"{open}\command"), "DelegateExecute", "")?;
        set_value(&open, MARK, "1")?;
        // the default verb: HKLM gives This PC "none" (its own navigation), so ours must be named here
        set_value(&shell, "", v)?;
        set_value(&shell, MARK, "1")?;
    }
    Ok(())
}

/// Undo `set`: Explorer opens folders, Win+E, This PC and the Recycle Bin again.
pub fn restore() -> Res<()> {
    let r = restore_in(CLASSES);
    assoc_changed();
    // the Win+E helper stops and no longer starts at sign-in
    if let Some(cmd) = get_value(RUN, HELPER_NAME) {
        let h = std::path::PathBuf::from(cmd.trim_matches('"'));
        if h.is_file() {
            run_helper(&h, &["--quit"]);
        }
        delete_value(RUN, HELPER_NAME);
    }
    r
}

pub fn restore_in(root: &str) -> Res<()> {
    for class in ["Directory", "Drive"] {
        let base = format!(r"{root}\{class}\shell");
        delete_tree(&format!(r"{base}\{VERB}"));
        for v in ["open", EXPLORER_VERB] {
            if get_value(&format!(r"{base}\{v}"), MARK).is_some() {
                delete_tree(&format!(r"{base}\{v}"));
            }
        }
        if get_value(&base, "").as_deref() == Some(VERB) || (get_value(&base, "").as_deref() == Some("open") && get_value(&format!(r"{base}\open"), "").is_none()) {
            // remove only our default verb value
            let p = wide(&base);
            unsafe {
                let mut key = HKEY::default();
                if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, KEY_SET_VALUE, &mut key) == ERROR_SUCCESS {
                    let _ = RegDeleteValueW(key, PCWSTR::null());
                    let _ = RegCloseKey(key);
                }
            }
        }
    }
    let we = format!(r"{root}\{WIN_E}");
    if get_value(&we, MARK).is_some() {
        delete_tree(&we);
    }
    for id in [THIS_PC, RECYCLE_BIN] {
        // our whole shell subtree goes (a default verb "open" left without its verb would break the icon), but only
        // when it carries our marker
        let shell = format!(r"{root}\CLSID\{id}\shell");
        let marked = |v: &str| get_value(&format!(r"{shell}\{v}"), MARK).is_some();
        if get_value(&shell, MARK).is_some() || marked("open") || marked(RB_VERB) {
            delete_tree(&shell);
            delete_if_empty(&format!(r"{root}\CLSID\{id}"));
        }
    }
    Ok(())
}
