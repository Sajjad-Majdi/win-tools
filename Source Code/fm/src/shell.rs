//! Shell actions: open with the default app, "Open with" handlers, reveal in a folder window, and the drive list.
use crate::util::*;
use std::path::Path;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ASSOC_FILTER_NONE, ASSOC_FILTER_RECOMMENDED, BHID_DataObject, IAssocHandler, IShellItem, ILFree, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHAssocEnumHandlers,
    SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHOpenFolderAndSelectItems, SHParseDisplayName, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

/// Open a file or folder with its default app (like a double-click). Errors instead of showing a dialog.
pub fn open(path: &Path) -> Res<()> {
    open_verb(path, "open", "")
}

/// Run a shell verb ("open", "edit", "print", "runas", ...) on a path, with optional arguments.
pub fn open_verb(path: &Path, verb: &str, args: &str) -> Res<()> {
    let path = abs(path);
    if !path.exists() {
        return Err(format!("not found: {}", path.display()));
    }
    let _c = com();
    let (f, v, a) = (wide(path.as_os_str()), wide(verb), wide(args));
    let dir = path.parent().map(|p| wide(p.as_os_str())).unwrap_or_else(|| wide(""));
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(v.as_ptr()),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: if args.is_empty() { PCWSTR::null() } else { PCWSTR(a.as_ptr()) },
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei) }.map_err(|e| win_err(&format!("cannot open {}", path.display()), &e))
}

/// An app that can open a file type ("Open with").
#[derive(Clone, Debug)]
pub struct Handler {
    /// Internal name (usually the exe path or an app id).
    pub name: String,
    /// The name people see.
    pub ui_name: String,
    pub recommended: bool,
    /// Where its icon is: "file,index", or an indirect "@{package?ms-resource://...}" string (packaged apps).
    pub icon: String,
    /// The app Windows uses now for this file type.
    pub default: bool,
}

/// ".mp4" style key for an extension or file name.
pub fn ext_key(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_lowercase();
    format!(".{e}")
}

/// Open a file with its Windows default app.
pub fn open_file(path: &Path) -> Res<()> {
    open(path)
}

/// The current default app of a file type: (friendly name, program path); empty strings when unknown.
pub fn default_app(ext: &str) -> (String, String) {
    use windows::Win32::UI::Shell::{ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR, ASSOCSTR_EXECUTABLE, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW};
    let e = if ext.starts_with('.') { ext.to_string() } else { format!(".{ext}") };
    let w = wide(&e);
    let q = |what: ASSOCSTR| -> String {
        let mut buf = vec![0u16; 1024];
        let mut n = buf.len() as u32;
        let hr = unsafe { AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN, what, PCWSTR(w.as_ptr()), PCWSTR::null(), Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut n) };
        if hr.is_ok() { from_wide(&buf) } else { String::new() }
    };
    (q(ASSOCSTR_FRIENDLYAPPNAME), q(ASSOCSTR_EXECUTABLE))
}

/// The Windows "Open with" dialog for a file, with "Always use this app to open .ext files": the only supported way to
/// change a default app (UserChoice is hash-protected and never written here). `owner` = the parent window handle.
pub fn open_with_dialog(path: &Path, owner: isize) -> Res<()> {
    use windows::Win32::UI::Shell::{OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OAIF_REGISTER_EXT, OPENASINFO, SHOpenWithDialog};
    let path = abs(path);
    if !path.is_file() {
        return Err(format!("not a file: {}", path.display()));
    }
    let _c = com();
    let w = wide(path.as_os_str());
    // Explorer's own "Choose another app": the shell's "openas" verb. Its dialog has "Always" (SHOpenWithDialog on
    // Windows 11 shows only "Just once"), so the Windows default can really be set there.
    {
        let v = wide("openas");
        let dir = path.parent().map(|p| wide(p.as_os_str())).unwrap_or_else(|| wide(""));
        let mut sei = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: windows::Win32::UI::Shell::SEE_MASK_INVOKEIDLIST | SEE_MASK_FLAG_NO_UI,
            hwnd: windows::Win32::Foundation::HWND(owner as *mut _),
            lpVerb: PCWSTR(v.as_ptr()),
            lpFile: PCWSTR(w.as_ptr()),
            lpDirectory: PCWSTR(dir.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut sei) }.is_ok() {
            return Ok(());
        }
    }
    let info = OPENASINFO { pcszFile: PCWSTR(w.as_ptr()), pcszClass: PCWSTR::null(), oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_REGISTER_EXT | OAIF_EXEC };
    let owner = (owner != 0).then_some(windows::Win32::Foundation::HWND(owner as *mut _));
    match unsafe { SHOpenWithDialog(owner, &info) } {
        Ok(()) => Ok(()),
        // the user closed the dialog
        Err(e) if e.code().0 as u32 == 0x800704C7 => Ok(()),
        Err(e) => Err(win_err("the Open with dialog failed", &e)),
    }
}

fn handlers_raw(ext: &str, all: bool) -> Res<Vec<(Handler, IAssocHandler)>> {
    let e = if ext.starts_with('.') { ext.to_string() } else { format!(".{ext}") };
    let w = wide(&e);
    let mut out = vec![];
    unsafe {
        let en = SHAssocEnumHandlers(PCWSTR(w.as_ptr()), if all { ASSOC_FILTER_NONE } else { ASSOC_FILTER_RECOMMENDED })
            .map_err(|er| win_err(&format!("no apps for {e}"), &er))?;
        loop {
            let mut slot: [Option<IAssocHandler>; 1] = [None];
            let mut got = 0u32;
            if en.Next(&mut slot, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(h) = slot[0].take() else { break };
            let name = h.GetName().map(take_pwstr).unwrap_or_default();
            let ui_name = h.GetUIName().map(take_pwstr).unwrap_or_default();
            let recommended = h.IsRecommended().is_ok() && h.IsRecommended().0 == 0;
            let mut ip = windows::core::PWSTR::null();
            let mut idx = 0i32;
            let icon = if h.GetIconLocation(&mut ip, &mut idx).is_ok() && !ip.is_null() {
                let p = take_pwstr(ip);
                if p.starts_with('@') { p } else { format!("{p},{idx}") }
            } else {
                String::new()
            };
            out.push((Handler { name, ui_name, recommended, icon, default: false }, h));
        }
    }
    Ok(out)
}

/// Apps that can open files with this extension (recommended ones, or every registered one with `all`).
pub fn open_with_list(ext: &str, all: bool) -> Res<Vec<Handler>> {
    let _c = com();
    let (friendly, exe) = default_app(ext);
    let mut v: Vec<Handler> = handlers_raw(ext, all)?.into_iter().map(|(h, _)| h).collect();
    let mut found = false;
    for h in v.iter_mut() {
        let is = !found && ((!exe.is_empty() && h.name.eq_ignore_ascii_case(&exe)) || (!friendly.is_empty() && h.ui_name.eq_ignore_ascii_case(&friendly)));
        h.default = is;
        found |= is;
    }
    // the default app first
    v.sort_by_key(|h| !h.default);
    Ok(v)
}

/// Open a file with one app from `open_with_list` (matched by name, UI name, or a part of the UI name).
pub fn open_with(path: &Path, app: &str) -> Res<Handler> {
    let path = abs(path);
    if !path.exists() {
        return Err(format!("not found: {}", path.display()));
    }
    let _c = com();
    let ext = format!(".{}", ext_of(&path.to_string_lossy()));
    let list = handlers_raw(&ext, true)?;
    let a = app.to_lowercase();
    let pick = list
        .iter()
        .find(|(h, _)| h.name.to_lowercase() == a || h.ui_name.to_lowercase() == a)
        .or_else(|| list.iter().find(|(h, _)| h.ui_name.to_lowercase().contains(&a) || h.name.to_lowercase().contains(&a)))
        .ok_or_else(|| format!("no app named '{app}' can open {ext} files"))?;
    let w = wide(path.as_os_str());
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).map_err(|e| win_err("cannot open item", &e))?;
        let dobj: IDataObject = item.BindToHandler(None, &BHID_DataObject).map_err(|e| win_err("cannot open item", &e))?;
        pick.1.Invoke(&dobj).map_err(|e| win_err(&format!("{} failed", pick.0.ui_name), &e))?;
    }
    Ok(pick.0.clone())
}

/// Show the item selected in a folder window of the current file manager (Explorer until Rust File Explorer is the default).
pub fn reveal(path: &Path) -> Res<()> {
    let path = abs(path);
    if !path.exists() {
        return Err(format!("not found: {}", path.display()));
    }
    let _c = com();
    let w = wide(path.as_os_str());
    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).map_err(|e| win_err("cannot find item", &e))?;
        let r = SHOpenFolderAndSelectItems(pidl, None, 0);
        ILFree(Some(pidl));
        r.map_err(|e| win_err("cannot open the folder", &e))
    }
}

/// Where `explorer /select,<path>` would go: (parent folder, name to select). The GUI opens the parent with the
/// item selected; until then `wfm open --select` prints both.
pub fn select_target(path: &Path) -> Res<(std::path::PathBuf, String)> {
    let path = abs(path);
    if !std::fs::symlink_metadata(&path).is_ok() {
        return Err(format!("not found: {}", path.display()));
    }
    let parent = path.parent().ok_or_else(|| format!("{} is a drive root", path.display()))?.to_path_buf();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((parent, name))
}

#[derive(Clone, Debug)]
pub struct Drive {
    /// "C:\"
    pub root: String,
    pub label: String,
    /// fixed | removable | network | cdrom | ramdisk | unknown
    pub kind: &'static str,
    pub fs: String,
    pub free: u64,
    pub total: u64,
    /// false for an empty card reader or DVD drive, or a network drive that did not answer in time.
    pub ready: bool,
    /// The share behind a mapped network drive (\\server\share).
    pub remote: String,
    /// The drive did not answer within the time limit (a dead network share).
    pub timed_out: bool,
}

fn drive_kind(root: &str) -> &'static str {
    let w = wide(root);
    match unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) } {
        2 => "removable",
        3 => "fixed",
        4 => "network",
        5 => "cdrom",
        6 => "ramdisk",
        _ => "unknown",
    }
}

/// Drive letters and types only (instant; touches no disk).
pub fn drives_fast() -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| {
            let root = format!("{}:\\", (b'A' + i as u8) as char);
            Drive { kind: drive_kind(&root), root, label: String::new(), fs: String::new(), free: 0, total: 0, ready: false, remote: String::new(), timed_out: false }
        })
        .collect()
}

/// The \\server\share behind a mapped drive letter ("" when it is not a network drive).
pub fn mapped_remote(root: &str) -> String {
    use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
    let local = wide(root.trim_end_matches('\\'));
    let mut buf = vec![0u16; 1024];
    let mut n = buf.len() as u32;
    let r = unsafe { WNetGetConnectionW(PCWSTR(local.as_ptr()), Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut n) };
    if r.0 == 0 { from_wide(&buf) } else { String::new() }
}

fn fill_drive(d: &mut Drive) {
    quiet_errors();
    let w = wide(&d.root);
    let mut label = [0u16; 261];
    let mut fsn = [0u16; 261];
    d.ready = unsafe { GetVolumeInformationW(PCWSTR(w.as_ptr()), Some(&mut label), None, None, None, Some(&mut fsn)) }.is_ok();
    d.label = from_wide(&label);
    d.fs = from_wide(&fsn);
    if d.ready {
        unsafe {
            let _ = GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut d.free), Some(&mut d.total), None);
        }
    }
}

/// All drive letters (local, removable and mapped network drives) with label, type and space. Each drive is
/// asked on its own thread; one that does not answer within `limit` (a dead share) is reported with
/// `timed_out` instead of hanging the list. Never shows "insert a disk" boxes.
pub fn drives_within(limit: std::time::Duration) -> Vec<Drive> {
    quiet_errors();
    let list = drives_fast();
    let (tx, rx) = std::sync::mpsc::channel::<(usize, Drive)>();
    for (i, d) in list.iter().enumerate() {
        let tx = tx.clone();
        let mut d = d.clone();
        std::thread::spawn(move || {
            if d.kind == "network" {
                d.remote = mapped_remote(&d.root);
            }
            fill_drive(&mut d);
            let _ = tx.send((i, d));
        });
    }
    drop(tx);
    let deadline = std::time::Instant::now() + limit;
    let mut out: Vec<Drive> = list.into_iter().map(|mut d| {
        d.timed_out = true;
        d
    }).collect();
    let mut left = out.len();
    while left > 0 {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(wait) {
            Ok((i, d)) => {
                out[i] = d;
                left -= 1;
            }
            Err(_) => break,
        }
    }
    for d in out.iter_mut().filter(|d| d.timed_out && d.kind == "network" && d.remote.is_empty()) {
        d.remote = mapped_remote(&d.root);
    }
    out
}

/// All drives, waiting at most 2 seconds for slow ones.
pub fn drives() -> Vec<Drive> {
    drives_within(std::time::Duration::from_secs(2))
}

/// Shares of a server (`\\server`), like Explorer's network view. Disk shares only; hidden ($) shares with `all`.
pub fn server_shares(server: &str, all: bool) -> Res<Vec<String>> {
    use windows::Win32::NetworkManagement::NetManagement::NetApiBufferFree;
    use windows::Win32::Storage::FileSystem::{NetShareEnum, SHARE_INFO_1};
    let srv = wide(server.trim_end_matches('\\'));
    let mut buf: *mut u8 = std::ptr::null_mut();
    let (mut read, mut total) = (0u32, 0u32);
    let r = unsafe { NetShareEnum(PCWSTR(srv.as_ptr()), 1, &mut buf, u32::MAX, &mut read, &mut total, None) };
    if r != 0 {
        if !buf.is_null() {
            unsafe { NetApiBufferFree(Some(buf as *const _)) };
        }
        return Err(format!("cannot list shares of {server} (error {r})"));
    }
    let mut out = vec![];
    unsafe {
        let items = std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize);
        for it in items {
            let name = it.shi1_netname.to_string().unwrap_or_default();
            let disk = it.shi1_type.0 & 0xFF == 0; // STYPE_DISKTREE
            if disk && (all || !name.ends_with('$')) {
                out.push(name);
            }
        }
        NetApiBufferFree(Some(buf as *const _));
    }
    out.sort_by_key(|s| s.to_lowercase());
    Ok(out)
}

/// Safely eject a removable drive (USB stick, card, DVD): lock, dismount, allow removal, eject. Refuses fixed
/// drives (including the system drive). Open files on the drive make it fail with a clear error.
pub fn eject(drive: &str) -> Res<()> {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::IO::DeviceIoControl;
    let letter = drive.trim().trim_end_matches(['\\', ':']).to_ascii_uppercase();
    if letter.len() != 1 || !letter.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!("not a drive letter: {drive}"));
    }
    let root = format!("{letter}:\\");
    let kind = drive_kind(&root);
    let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
    if sys.starts_with(&letter) || !matches!(kind, "removable" | "cdrom") {
        return Err(format!("{root} is not removable ({kind}); only USB sticks, cards and DVD drives can be ejected"));
    }
    let dev = wide(format!(r"\\.\{letter}:"));
    unsafe {
        let h = CreateFileW(PCWSTR(dev.as_ptr()), GENERIC_READ.0 | GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)
            .map_err(|e| win_err(&format!("cannot open {root}"), &e))?;
        let io = |code: u32, input: Option<&[u8]>| -> windows::core::Result<()> {
            let mut n = 0u32;
            DeviceIoControl(h, code, input.map(|b| b.as_ptr() as *const _), input.map(|b| b.len() as u32).unwrap_or(0), None, 0, Some(&mut n), None)
        };
        // FSCTL_LOCK_VOLUME: retry a little (the indexer or AV may hold it for a moment).
        let mut locked = false;
        for _ in 0..10 {
            if io(0x0009_0018, None).is_ok() {
                locked = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if !locked {
            let _ = CloseHandle(h);
            return Err(format!("{root} is in use (close the files and windows open on it, then try again)"));
        }
        let r = io(0x0009_0020, None) // FSCTL_DISMOUNT_VOLUME
            .and_then(|_| io(0x002D_4804, Some(&[0u8]))) // IOCTL_STORAGE_MEDIA_REMOVAL (PreventMediaRemoval = FALSE)
            .and_then(|_| io(0x002D_4808, None)); // IOCTL_STORAGE_EJECT_MEDIA
        let _ = CloseHandle(h);
        r.map_err(|e| win_err(&format!("cannot eject {root}"), &e))
    }
}
