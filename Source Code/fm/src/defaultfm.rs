//! "Set as default file manager" / "Restore Explorer", per user (HKCU only, no admin rights).
//!
//! `set` makes folders, drives, Win+E and the This PC / Recycle Bin desktop icons open in RustFileExplorer.exe:
//! - `HKCU\Software\Classes\Directory\shell` and `...\Drive\shell`: the canonical verb `open` is made ours
//!   (`"<RustFileExplorer.exe>" "%1"`, empty `DelegateExecute`) and kept plain: no Icon, no name and no default-verb
//!   value ([`DEFAULT_OPEN`]), so Windows 11's main right-click menu shows only Windows' own rows. Two classic verbs,
//!   shown under "Show more options": `openinwfiles` ("Open in Rust File Explorer", our icon) and `openinexplorer`
//!   ("Open in File Explorer", Explorer's icon and delegate). The `explore` verb (asked for by name by Visual
//!   Studio and other apps) is ours too, hidden from menus (`ProgrammaticAccessOnly`);
//! - `HKCU\Software\Classes\Folder\shell\open` and `\explore` (callers that name the class "Folder": AutoPlay
//!   "Open folder", ShellExecuteEx with lpClass "Folder", virtual folders) = ours, plus `Folder\shell\openinexplorer`
//!   (Explorer's own delegate, hidden) for the hand-back of virtual folders (Control Panel, Network, phones): our
//!   exe gets them as "::{CLSID}" names and gives them to Explorer through that verb (never through open/explore,
//!   which would loop). No default value is written on `Folder\shell`;
//! - `HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command` (Win+E);
//! - `HKCU\Software\Classes\CLSID\{20D04FE0-3AEA-1069-A2D8-08002B30309D}\shell\open\command` (This PC) and
//!   `...\CLSID\{645FF040-5081-101B-9F08-00AA002F954E}\shell\open\command` (Recycle Bin): `"<exe>" "::{CLSID}"`, with
//!   an empty `DelegateExecute` value so the per-user command wins over Explorer's delegate (as Files / Explorer++ do).
//! `restore` deletes exactly those keys again (only keys that carry our `WinToolsFiles` marker), so Explorer is the
//! handler once more. [`guard_missing_exe`] restores at start when our keys point to an exe that is gone.
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
pub const RECYCLE_BIN: &str = "{645FF040-5081-101B-9F08-00AA002F954E}";
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
    /// the `explore` verb of folders and drives is ours
    pub explore: bool,
    /// the class "Folder" (`open`, `explore`, plus the `openinexplorer` hand-back) is ours
    pub folder_class: bool,
    /// the right-click menus: nothing of ours in Windows 11's main list (`open` has no Icon or name); "Open in Rust
    /// File Explorer" and "Open in File Explorer" with their icons under "Show more options"
    pub menu: bool,
}

impl Status {
    pub fn all(&self) -> bool {
        self.parts().iter().all(|p| p.1)
    }
    pub fn parts(&self) -> [(&'static str, bool); 8] {
        [
            ("menu", self.menu),
            ("folders", self.folders),
            ("drives", self.drives),
            ("explore", self.explore),
            ("folder_class", self.folder_class || !FOLDER_CLASS),
            ("win_e", self.win_e),
            ("this_pc", self.this_pc),
            ("win_e_key", self.win_e_key),
        ]
    }
}

/// `set` makes the class "Folder" ours too (advisor item 2b). Off: `set` writes no Folder keys (restore still
/// removes any it finds).
pub const FOLDER_CLASS: bool = true;

/// `set` names `open` as the default verb of Directory and Drive. Off (since 2026-11-26): no default value is
/// written, so HKLM's "none" stays and Windows picks the verb exactly as it does without us (its main menu then
/// shows only its own rows); double-click and ShellExecute without a verb still reach our `open` (`wfm default
/// probe`: no verb, "open", "explore", class "Folder" all open here). A default value an earlier build wrote is
/// removed by `set`.
pub const DEFAULT_OPEN: bool = false;

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
        let default_ok = !DEFAULT_OPEN || get_value(&shell, "").map(|d| d.eq_ignore_ascii_case("open")).unwrap_or(false);
        default_ok && get_value(&format!(r"{shell}\open"), MARK).is_some() && get_value(&format!(r"{shell}\open\command"), "").is_some()
    };
    // the menus: `open` plain (no Icon / name: Windows' own row in the main menu), our two named entries there
    let menu = |class: &str| {
        let shell = format!(r"{root}\{class}\shell");
        let plain = get_value(&format!(r"{shell}\open"), "Icon").is_none() && get_value(&format!(r"{shell}\open"), "MUIVerb").is_none() && get_value(&format!(r"{shell}\open"), "").is_none();
        let named = |v: &str| get_value(&format!(r"{shell}\{v}"), MARK).is_some() && get_value(&format!(r"{shell}\{v}"), "Icon").is_some() && get_value(&format!(r"{shell}\{v}"), "ProgrammaticAccessOnly").is_none();
        plain && named(VERB) && named(EXPLORER_VERB)
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
    let ours = |key: &str| get_value(key, MARK).is_some() && get_value(&format!(r"{key}\command"), "").is_some();
    Status {
        folders: verb("Directory"),
        drives: verb("Drive"),
        win_e: get_value(&format!(r"{root}\{WIN_E}"), MARK).is_some(),
        this_pc: clsid(THIS_PC),
        win_e_key: root != CLASSES || get_value(RUN, HELPER_NAME).is_some(),
        explore: ours(&format!(r"{root}\Directory\shell\explore")) && ours(&format!(r"{root}\Drive\shell\explore")),
        menu: menu("Directory") && menu("Drive"),
        folder_class: ["open", "explore", EXPLORER_VERB].iter().all(|v| ours(&format!(r"{root}\Folder\shell\{v}"))),
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

/// One verb that runs our exe: `"<exe>" "%1"`, an empty DelegateExecute (beats Explorer's delegate in the merged
/// view), our marker, no Icon and no name (Windows' own row in its menus); `hidden` adds ProgrammaticAccessOnly (asked for by name only, never in a menu). Never over a
/// command that is not ours.
fn our_verb(key: &str, exe: &str, hidden: bool) -> Res<()> {
    if get_value(&format!(r"{key}\command"), "").is_some() && get_value(key, MARK).is_none() {
        return Err(format!(r"HKCU\{key} already has a command that is not ours"));
    }
    delete_tree(key);
    if hidden {
        set_value(key, "ProgrammaticAccessOnly", "")?;
    }
    set_value(key, "MultiSelectModel", "Document")?;
    set_value(key, MARK, "1")?;
    set_value(&format!(r"{key}\command"), "", &format!("\"{exe}\" \"%1\""))?;
    set_value(&format!(r"{key}\command"), "DelegateExecute", "")?;
    Ok(())
}

pub fn set_in(root: &str, exe: &std::path::Path) -> Res<()> {
    let exe = exe.to_string_lossy();
    if !exe.to_lowercase().ends_with(".exe") {
        return Err("needs the full path of RustFileExplorer.exe".into());
    }
    let explorer = format!(r"{}\Explorer.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\WINDOWS".into()));
    for class in ["Directory", "Drive"] {
        let base = format!(r"{root}\{class}\shell");
        // `open` is ours and stays plain: no Icon, no name, so Windows 11's main menu draws its own normal row (an
        // Icon here made a half-styled row with our icon there; Sajjad, 2026-11-26). Our named entries below are
        // classic verbs, shown only under "Show more options".
        our_verb(&format!(r"{base}\open"), &exe, false)?;
        if DEFAULT_OPEN {
            set_value(&base, "", "open")?;
        } else if get_value(&base, "").as_deref() == Some("open") && get_value(&format!(r"{base}\open"), MARK).is_some() {
            delete_value(&base, "");
        }
        our_verb(&format!(r"{base}\explore"), &exe, true)?;
        // "Open in Rust File Explorer" (classic menu only)
        let me = format!(r"{base}\{VERB}");
        delete_tree(&me);
        set_value(&me, "", "Open in Rust File Explorer")?;
        set_value(&me, "Icon", &format!("{exe},0"))?;
        set_value(&me, "MultiSelectModel", "Document")?;
        set_value(&me, MARK, "1")?;
        set_value(&format!(r"{me}\command"), "", &format!("\"{exe}\" \"%1\""))?;
        // "Open in File Explorer" (classic menu only): Explorer's own delegate
        let ex = format!(r"{base}\{EXPLORER_VERB}");
        set_value(&ex, "", "Open in File Explorer")?;
        set_value(&ex, "Icon", &format!("{explorer},0"))?;
        set_value(&ex, "MultiSelectModel", "Document")?;
        set_value(&ex, MARK, "1")?;
        set_value(&format!(r"{ex}\command"), "", &explorer)?;
        set_value(&format!(r"{ex}\command"), "DelegateExecute", EXPLORER_DELEGATE)?;
    }
    if FOLDER_CLASS {
        // the class "Folder": no default value on Folder\shell (Windows keeps choosing the verb per item)
        let base = format!(r"{root}\Folder\shell");
        our_verb(&format!(r"{base}\open"), &exe, false)?;
        our_verb(&format!(r"{base}\explore"), &exe, true)?;
        let ex = format!(r"{base}\{EXPLORER_VERB}");
        if get_value(&format!(r"{ex}\command"), "").is_some() && get_value(&ex, MARK).is_none() {
            return Err(format!(r"HKCU\{ex} already has a command that is not ours"));
        }
        delete_tree(&ex);
        set_value(&ex, "", "Open in File Explorer")?;
        set_value(&ex, "ProgrammaticAccessOnly", "")?;
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
        for v in ["open", "explore", EXPLORER_VERB] {
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
    // the class "Folder": only our verbs, then the keys left empty (Folder\shell itself is Windows' in HKLM)
    let fs = format!(r"{root}\Folder\shell");
    for v in ["open", "explore", EXPLORER_VERB] {
        if get_value(&format!(r"{fs}\{v}"), MARK).is_some() {
            delete_tree(&format!(r"{fs}\{v}"));
        }
    }
    delete_if_empty(&fs);
    delete_if_empty(&format!(r"{root}\Folder"));
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

/// The exe our marked `Directory\shell\open` verb runs (None when that verb is not ours).
pub fn registered_exe() -> Option<std::path::PathBuf> {
    registered_exe_in(CLASSES)
}

pub fn registered_exe_in(root: &str) -> Option<std::path::PathBuf> {
    let open = format!(r"{root}\Directory\shell\open");
    get_value(&open, MARK)?;
    exe_of_command(&get_value(&format!(r"{open}\command"), "")?)
}

/// `"C:\a b\x.exe" "%1"` -> `C:\a b\x.exe`; an unquoted command ends at the first ".exe".
pub fn exe_of_command(cmd: &str) -> Option<std::path::PathBuf> {
    let c = cmd.trim();
    let p = if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let i = c.to_lowercase().find(".exe")?;
        c[..i + 4].to_string()
    };
    (!p.is_empty()).then(|| std::path::PathBuf::from(p))
}

/// Called at start (RustFileExplorer.exe and the Win+E helper): when our keys point to an exe that no longer
/// exists (moved, deleted, a build folder cleaned), folders would open nowhere, so Explorer is put back. True when
/// it restored.
pub fn guard_missing_exe() -> bool {
    guard_missing_exe_in(CLASSES)
}

pub fn guard_missing_exe_in(root: &str) -> bool {
    match registered_exe_in(root) {
        Some(exe) if !exe.is_file() => {
            if root == CLASSES {
                let _ = restore();
            } else {
                let _ = restore_in(root);
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private test root under HKCU\Software\RustFileExplorerTest (never the real classes).
    fn root(name: &str) -> String {
        format!(r"Software\RustFileExplorerTest\Unit-{name}-{}\Classes", std::process::id())
    }

    fn drop_root(r: &str) {
        assert!(r.starts_with(r"Software\RustFileExplorerTest\"));
        delete_tree(r.trim_end_matches(r"\Classes"));
    }

    #[test]
    fn exe_of_command_reads_quoted_and_plain() {
        assert_eq!(exe_of_command(r#""C:\a b\RustFileExplorer.exe" "%1""#), Some(r"C:\a b\RustFileExplorer.exe".into()));
        assert_eq!(exe_of_command(r"C:\x\y.exe %1"), Some(r"C:\x\y.exe".into()));
        assert_eq!(exe_of_command(""), None);
    }

    #[test]
    fn explore_and_folder_class_set_status_restore() {
        let r = root("verbs");
        let exe = std::env::current_exe().unwrap();
        set_in(&r, &exe).unwrap();
        let st = status_in(&r);
        assert!(st.explore && st.folder_class && st.folders && st.drives && st.menu, "{st:?}");
        assert_eq!(get_value(&format!(r"{r}\Directory\shell"), "").is_some(), DEFAULT_OPEN, "default verb value only when DEFAULT_OPEN");
        // Windows 11's main menu: `open` carries nothing of ours (no Icon, no name); the named entries are classic
        for class in ["Directory", "Drive"] {
            let sh = format!(r"{r}\{class}\shell");
            assert_eq!(get_value(&format!(r"{sh}\open"), "Icon"), None);
            assert_eq!(get_value(&format!(r"{sh}\open"), ""), None);
            assert_eq!(get_value(&format!(r"{sh}\{VERB}"), "").as_deref(), Some("Open in Rust File Explorer"));
            assert_eq!(get_value(&format!(r"{sh}\{VERB}"), "Icon"), Some(format!("{},0", exe.display())));
            assert_eq!(get_value(&format!(r"{sh}\{EXPLORER_VERB}"), "").as_deref(), Some("Open in File Explorer"));
            assert!(get_value(&format!(r"{sh}\{EXPLORER_VERB}"), "Icon").unwrap().to_lowercase().ends_with("explorer.exe,0"));
        }
        // an Icon put back on `open` (as 0.2.7 wrote it) is reported
        set_value(&format!(r"{r}\Directory\shell\open"), "Icon", "x.exe,0").unwrap();
        assert!(!status_in(&r).menu);
        set_in(&r, &exe).unwrap();
        assert!(status_in(&r).menu);
        for class in ["Directory", "Drive", "Folder"] {
            let ex = format!(r"{r}\{class}\shell\explore");
            assert_eq!(get_value(&ex, MARK).as_deref(), Some("1"));
            assert!(get_value(&ex, "ProgrammaticAccessOnly").is_some(), "{class} explore is hidden");
            assert_eq!(get_value(&format!(r"{ex}\command"), "DelegateExecute").as_deref(), Some(""));
            assert!(get_value(&format!(r"{ex}\command"), "").unwrap().contains(&format!("\"{}\" \"%1\"", exe.display())));
        }
        let fo = format!(r"{r}\Folder\shell");
        assert_eq!(get_value(&fo, ""), None, "no default verb on Folder\\shell");
        assert!(get_value(&format!(r"{fo}\open"), "ProgrammaticAccessOnly").is_none());
        assert_eq!(get_value(&format!(r"{fo}\openinexplorer\command"), "DelegateExecute").as_deref(), Some(EXPLORER_DELEGATE));
        assert!(get_value(&format!(r"{fo}\openinexplorer"), "ProgrammaticAccessOnly").is_some());
        assert_eq!(registered_exe_in(&r), Some(exe.clone()));
        // a second set gives the same keys
        set_in(&r, &exe).unwrap();
        assert_eq!(status_in(&r), st);
        restore_in(&r).unwrap();
        let st = status_in(&r);
        assert!(!st.explore && !st.folder_class && !st.folders && !st.menu, "{st:?}");
        assert_eq!(get_value(&format!(r"{r}\Directory\shell\{VERB}\command"), ""), None);
        assert_eq!(get_value(&format!(r"{r}\Folder\shell\open\command"), ""), None);
        drop_root(&r);
    }

    #[test]
    fn restore_keeps_keys_without_our_marker() {
        let r = root("foreign");
        let exe = std::env::current_exe().unwrap();
        // someone else's Folder verb, and someone else's Directory explore verb
        set_value(&format!(r"{r}\Folder\shell\other\command"), "", "x.exe").unwrap();
        set_value(&format!(r"{r}\Directory\shell\explore\command"), "", "theirs.exe \"%1\"").unwrap();
        assert!(set_in(&r, &exe).is_err(), "never over a command that is not ours");
        restore_in(&r).unwrap();
        assert_eq!(get_value(&format!(r"{r}\Directory\shell\explore\command"), "").as_deref(), Some("theirs.exe \"%1\""), "foreign explore stays");
        delete_tree(&format!(r"{r}\Directory\shell\explore"));
        set_in(&r, &exe).unwrap();
        restore_in(&r).unwrap();
        assert_eq!(get_value(&format!(r"{r}\Folder\shell\other\command"), "").as_deref(), Some("x.exe"), "foreign verb stays");
        assert!(get_value(&format!(r"{r}\Folder\shell\open\command"), "").is_none());
        drop_root(&r);
    }

    #[test]
    fn guard_restores_when_the_exe_is_gone() {
        let r = root("guard");
        let gone = std::env::temp_dir().join(format!("wfm-test-gone-{}", std::process::id())).join("RustFileExplorer.exe");
        set_in(&r, &gone).unwrap();
        assert!(status_in(&r).folders);
        assert!(guard_missing_exe_in(&r));
        assert!(!status_in(&r).folders && !status_in(&r).folder_class && !status_in(&r).explore);
        assert!(!guard_missing_exe_in(&r), "nothing left to restore");
        // an exe that exists stays registered
        set_in(&r, &std::env::current_exe().unwrap()).unwrap();
        assert!(!guard_missing_exe_in(&r));
        assert!(status_in(&r).folders);
        restore_in(&r).unwrap();
        drop_root(&r);
    }
}
