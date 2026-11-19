//! "Pick a save location" for agents: find open standard Save / Open dialogs (#32770 with a file-name box), type a
//! full path into the file-name box and press Save, so an agent never gets stuck on a GUI dialog.
//! Works on the caller's desktop, or on a named desktop (`use_desktop`), e.g. a hidden test desktop.
use crate::util::*;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::StationsAndDesktops::{
    DESKTOP_CONTROL_FLAGS, DESKTOP_CREATEWINDOW, DESKTOP_ENUMERATE, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS, OpenDesktopW, SetThreadDesktop,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GW_OWNER, GetClassNameW, GetDlgCtrlID, GetDlgItem, GetParent, GetWindow, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    PostMessageW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COMMAND, WM_GETTEXT, WM_SETTEXT,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

/// An open file dialog.
#[derive(Clone, Debug)]
pub struct FileDialog {
    pub hwnd: isize,
    pub title: String,
    pub pid: u32,
    pub process: String,
    /// "save", "open" or "file" (unknown).
    pub kind: String,
    /// What the file-name box holds now.
    pub file_name: String,
    edit: isize,
    ok: isize,
}

/// Run this thread on another desktop of this window station (e.g. "wfm_hidden"). Call before any window work.
pub fn use_desktop(name: &str) -> Res<()> {
    let w = wide(name);
    unsafe {
        let access = DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0 | DESKTOP_ENUMERATE.0 | DESKTOP_CREATEWINDOW.0;
        let d = OpenDesktopW(PCWSTR(w.as_ptr()), DESKTOP_CONTROL_FLAGS(0), false, access).map_err(|e| win_err(&format!("cannot open desktop '{name}'"), &e))?;
        SetThreadDesktop(d).map_err(|e| win_err(&format!("cannot switch to desktop '{name}'"), &e))
    }
}

fn hwnd(v: isize) -> HWND {
    HWND(v as *mut _)
}

fn class_of(h: HWND) -> String {
    let mut b = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut b) };
    String::from_utf16_lossy(&b[..n.max(0) as usize])
}

/// Window text of a window in any process (WM_GETTEXT is marshalled across processes; GetWindowText is not).
pub fn text_of(h: HWND) -> String {
    let mut b = vec![0u16; 1024];
    let mut res = 0usize;
    unsafe {
        SendMessageTimeoutW(h, WM_GETTEXT, WPARAM(b.len()), LPARAM(b.as_mut_ptr() as isize), SMTO_ABORTIFHUNG, 1000, Some(&mut res));
    }
    from_wide(&b[..res.min(b.len())])
}

unsafe extern "system" fn collect(h: HWND, l: LPARAM) -> BOOL {
    let v = unsafe { &mut *(l.0 as *mut Vec<HWND>) };
    v.push(h);
    BOOL(1)
}

fn top_windows() -> Vec<HWND> {
    let mut v: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

fn children(h: HWND) -> Vec<HWND> {
    let mut v: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumChildWindows(Some(h), Some(collect), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut b = [0u16; 1024];
        let mut n = b.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(b.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let full = String::from_utf16_lossy(&b[..n as usize]);
        full.rsplit('\\').next().unwrap_or("").to_string()
    }
}

/// The file-name edit box of a dialog: inside the combo box with id 1148 (cmb13) or 1001, else edt1 (1152),
/// else the first Edit inside any combo box.
fn find_name_edit(dlg: HWND) -> Option<HWND> {
    let kids = children(dlg);
    let parent_id = |e: HWND| -> (i32, String) {
        let mut p = unsafe { GetParent(e) }.unwrap_or_default();
        // Edit -> ComboBox -> ComboBoxEx32: take the id of the outermost combo.
        let mut id = unsafe { GetDlgCtrlID(p) };
        let mut cls = class_of(p);
        if let Ok(pp) = unsafe { GetParent(p) } {
            if class_of(pp) == "ComboBoxEx32" {
                p = pp;
                id = unsafe { GetDlgCtrlID(p) };
                cls = "ComboBoxEx32".into();
            }
        }
        (id, cls)
    };
    let edits: Vec<HWND> = kids.iter().copied().filter(|&k| class_of(k) == "Edit").collect();
    for want in [1148, 1001] {
        if let Some(e) = edits.iter().copied().find(|&e| {
            let (id, cls) = parent_id(e);
            cls.starts_with("ComboBox") && (id == want || unsafe { GetDlgCtrlID(e) } == want)
        }) {
            return Some(e);
        }
    }
    if let Ok(e) = unsafe { GetDlgItem(Some(dlg), 1152) } {
        if class_of(e) == "Edit" {
            return Some(e);
        }
    }
    edits.into_iter().find(|&e| parent_id(e).1.starts_with("ComboBox"))
}

fn as_dialog(h: HWND) -> Option<FileDialog> {
    if class_of(h) != "#32770" || !unsafe { IsWindowVisible(h) }.as_bool() {
        return None;
    }
    let edit = find_name_edit(h)?;
    let ok = unsafe { GetDlgItem(Some(h), 1) }.ok()?;
    let ok_text = text_of(ok).replace('&', "").to_lowercase();
    let title = text_of(h);
    let tl = title.to_lowercase();
    let kind = if ok_text.contains("save") || tl.contains("save") {
        "save"
    } else if ok_text.contains("open") || tl.contains("open") {
        "open"
    } else {
        "file"
    };
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    Some(FileDialog {
        hwnd: h.0 as isize,
        title,
        pid,
        process: process_name(pid),
        kind: kind.into(),
        file_name: text_of(edit),
        edit: edit.0 as isize,
        ok: ok.0 as isize,
    })
}

/// All open file dialogs on this thread's desktop.
pub fn list() -> Vec<FileDialog> {
    top_windows().into_iter().filter_map(as_dialog).collect()
}

/// Outcome of `save`.
#[derive(Clone, Debug)]
pub struct SaveResult {
    pub dialog: FileDialog,
    /// The dialog closed (the app accepted the path).
    pub closed: bool,
    /// The app asked to confirm (e.g. "replace the existing file?"); text when it could be read.
    pub confirm: Option<String>,
    /// We pressed Yes on that question (only with `overwrite`).
    pub confirmed: bool,
}

/// Fill `full_path` into a file dialog and press its Save/Open button. `which`: a dialog handle from `list`
/// (needed when several are open). With `overwrite`, a "replace it?" question is answered Yes.
pub fn save(full_path: &str, which: Option<isize>, overwrite: bool, wait: Duration) -> Res<SaveResult> {
    let all = list();
    let dlg = match which {
        Some(h) => all.into_iter().find(|d| d.hwnd == h).ok_or_else(|| format!("no file dialog with handle {h}"))?,
        None => match all.len() {
            0 => return Err("no open Save/Open dialog found".into()),
            1 => all.into_iter().next().unwrap(),
            n => {
                return Err(format!(
                    "{n} file dialogs are open; pick one with --hwnd ({})",
                    all.iter().map(|d| format!("{} = {} [{}]", d.hwnd, d.title, d.process)).collect::<Vec<_>>().join(", ")
                ));
            }
        },
    };
    let w = wide(full_path);
    let mut res = 0usize;
    unsafe {
        let r = SendMessageTimeoutW(hwnd(dlg.edit), WM_SETTEXT, WPARAM(0), LPARAM(w.as_ptr() as isize), SMTO_ABORTIFHUNG, 3000, Some(&mut res));
        if r.0 == 0 {
            return Err("the dialog does not answer (hung, or it belongs to an elevated app)".into());
        }
    }
    if text_of(hwnd(dlg.edit)) != full_path {
        return Err("could not type into the file-name box (the app may run as administrator)".into());
    }
    // IDOK with BN_CLICKED, posted: a modal question the app shows next must not block us.
    unsafe { PostMessageW(Some(hwnd(dlg.hwnd)), WM_COMMAND, WPARAM(1), LPARAM(dlg.ok)) }.map_err(|e| win_err("cannot press Save", &e))?;
    let mut out = SaveResult { dialog: dlg.clone(), closed: false, confirm: None, confirmed: false };
    let t0 = Instant::now();
    while t0.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(50));
        if !unsafe { IsWindow(Some(hwnd(dlg.hwnd))) }.as_bool() || !unsafe { IsWindowVisible(hwnd(dlg.hwnd)) }.as_bool() {
            out.closed = true;
            return Ok(out);
        }
        // A question owned by the dialog ("... already exists. Do you want to replace it?").
        if let Some(q) = top_windows().into_iter().find(|&t| unsafe { GetWindow(t, GW_OWNER) }.map(|o| o.0 as isize).ok() == Some(dlg.hwnd) && class_of(t) == "#32770" && unsafe { IsWindowVisible(t) }.as_bool()) {
            let text: Vec<String> = children(q).into_iter().filter(|&c| class_of(c) == "Static").map(text_of).filter(|t| !t.trim().is_empty()).collect();
            out.confirm = Some(if text.is_empty() { text_of(q) } else { text.join(" ") });
            if !overwrite || out.confirmed {
                if !overwrite {
                    return Ok(out);
                }
                continue;
            }
            unsafe {
                // Task dialog: TDM_CLICK_BUTTON(IDYES); classic message box: WM_COMMAND(IDYES).
                let _ = PostMessageW(Some(q), 0x0400 + 102, WPARAM(6), LPARAM(0));
                let _ = PostMessageW(Some(q), WM_COMMAND, WPARAM(6), LPARAM(0));
            }
            out.confirmed = true;
        }
    }
    Ok(out)
}

/// Child controls of a window (class, id, text): for debugging dialogs that are not found.
pub fn describe(h: isize) -> Vec<(String, i32, String)> {
    children(hwnd(h)).into_iter().map(|c| (class_of(c), unsafe { GetDlgCtrlID(c) }, text_of(c))).collect()
}
