//! Recycle Bin: send items there (SHFileOperationW + FOF_ALLOWUNDO, no UI), find the record Windows wrote for each
//! item ($I / $R pair in <drive>\$Recycle.Bin\<user SID>) and restore it later (used by undo).
use crate::util::*;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::Storage::FileSystem::{FILE_FLAGS_AND_ATTRIBUTES, GetDriveTypeW, MOVEFILE_COPY_ALLOWED, MoveFileExW, SetFileAttributesW};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW, SHFileOperationW};
use windows::core::{PCWSTR, PWSTR};

/// One item in the Recycle Bin that we can put back.
#[derive(Clone, Debug)]
pub struct Recycled {
    pub original: PathBuf,
    /// The $R... file or folder that holds the data.
    pub data: PathBuf,
    /// The $I... record file.
    pub record: PathBuf,
    pub deleted: u64,
}

/// The current user's SID as text (S-1-5-21-...).
pub fn user_sid() -> Option<String> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).ok()?;
        let mut buf = vec![0u8; 256];
        let mut n = 0u32;
        let ok = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr() as *mut _), buf.len() as u32, &mut n).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(tok);
        if !ok {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(tu.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        out
    }
}

/// "C:\" for "C:\x\y" (None for UNC paths).
pub fn drive_root(p: &Path) -> Option<String> {
    let s = p.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() { Some(format!("{}:\\", (b[0] as char).to_ascii_uppercase())) } else { None }
}

/// The Recycle Bin folder of the current user on the drive of `p`.
pub fn bin_dir(p: &Path) -> Option<PathBuf> {
    Some(PathBuf::from(drive_root(p)?).join("$Recycle.Bin").join(user_sid()?))
}

/// Does this drive have a Recycle Bin? (fixed local drives do; removable and network drives do not)
pub fn has_recycle_bin(p: &Path) -> bool {
    let Some(root) = drive_root(p) else { return false };
    let w = wide(&root);
    unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) == 3 }
}

/// Read one $I record: (original path, deletion FILETIME).
pub fn read_record(i_file: &Path) -> Option<(PathBuf, u64)> {
    let b = std::fs::read(i_file).ok()?;
    if b.len() < 28 {
        return None;
    }
    let ver = u64::from_le_bytes(b[0..8].try_into().ok()?);
    let ft = u64::from_le_bytes(b[16..24].try_into().ok()?);
    let words: Vec<u16> = match ver {
        2 => {
            let n = u32::from_le_bytes(b[24..28].try_into().ok()?) as usize;
            b.get(28..28 + n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
        }
        1 => b.get(24..24 + 520)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect(),
        _ => return None,
    };
    Some((PathBuf::from(from_wide(&words)), ft))
}

/// Size recorded in a $I record (bytes of the file or folder when it was deleted).
pub fn record_size(i_file: &Path) -> u64 {
    std::fs::read(i_file).ok().filter(|b| b.len() >= 16).map(|b| u64::from_le_bytes(b[8..16].try_into().unwrap())).unwrap_or(0)
}

/// Short id of an item: "<drive letter>:<$I file name>", e.g. "C:$IAB12CD.txt".
pub fn item_id(r: &Recycled) -> String {
    let d = r.record.to_string_lossy().chars().next().unwrap_or('?');
    format!("{d}:{}", r.record.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// Everything in this user's Recycle Bin on every fixed drive (newest first).
pub fn list_all() -> Vec<Recycled> {
    let mut out: Vec<Recycled> = vec![];
    for d in crate::shell::drives_fast() {
        if d.kind == "fixed" {
            out.extend(list_bin(Path::new(&d.root)));
        }
    }
    out.sort_by(|a, b| b.deleted.cmp(&a.deleted));
    out
}

/// Find a Recycle Bin item by its id (see `item_id`) or by its original path (the newest one wins).
pub fn find_item(key: &str) -> Res<Recycled> {
    let all = list_all();
    if let Some(r) = all.iter().find(|r| item_id(r).eq_ignore_ascii_case(key)) {
        return Ok(r.clone());
    }
    let p = abs(key);
    all.into_iter().find(|r| path_eq(&r.original, &p)).ok_or_else(|| format!("not in the Recycle Bin: {key}"))
}

/// Empty this user's Recycle Bin on every drive, for good, without any UI or sound.
pub fn empty_all() -> Res<()> {
    use windows::Win32::UI::Shell::{SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW};
    let _c = com();
    match unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) } {
        Ok(()) => Ok(()),
        // E_UNEXPECTED: the bin was already empty.
        Err(e) if e.code().0 as u32 == 0x8000_FFFF => Ok(()),
        Err(e) => Err(win_err("cannot empty the Recycle Bin", &e)),
    }
}

/// Every restorable item of this user on the drive of `p` (newest first).
pub fn list_bin(p: &Path) -> Vec<Recycled> {
    let Some(dir) = bin_dir(p) else { return vec![] };
    let Ok(list) = crate::list::list_dir(&dir, true) else { return vec![] };
    let mut out: Vec<Recycled> = list
        .iter()
        .filter(|e| e.name.starts_with("$I"))
        .filter_map(|e| {
            let rec = dir.join(&e.name);
            let (orig, ft) = read_record(&rec)?;
            let data = dir.join(format!("$R{}", &e.name[2..]));
            Some(Recycled { original: orig, data, record: rec, deleted: ft })
        })
        .collect();
    out.sort_by(|a, b| b.deleted.cmp(&a.deleted));
    out
}

/// Send items to the Recycle Bin without any dialog. Refuses drives that have no Recycle Bin (there Windows would
/// delete for good). Returns the restore record of each item it could find.
pub fn recycle(paths: &[PathBuf]) -> Res<Vec<Recycled>> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let mut list: Vec<u16> = vec![];
    for p in paths {
        if !p.exists() {
            return Err(format!("not found: {}", p.display()));
        }
        if !has_recycle_bin(p) {
            return Err(format!("{} is on a drive without a Recycle Bin; use --permanent to delete it for good", p.display()));
        }
        list.extend(p.as_os_str().to_string_lossy().encode_utf16());
        list.push(0);
    }
    list.push(0);
    let t0 = now_ft().saturating_sub(20_000_000); // 2 s slack
    let _c = com();
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(list.as_ptr()),
        // FOF_WANTNUKEWARNING: if an item cannot go to the Recycle Bin (too big), Windows asks instead of
        // silently destroying it.
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING).0 as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 {
        return Err(format!("could not move to the Recycle Bin (shell error 0x{r:X})"));
    }
    if op.fAnyOperationsAborted.as_bool() {
        return Err("moving to the Recycle Bin was cancelled".into());
    }
    let mut out = vec![];
    let mut bins: Vec<(PathBuf, Vec<Recycled>)> = vec![];
    for p in paths {
        let Some(bd) = bin_dir(p) else { continue };
        if !bins.iter().any(|(d, _)| d == &bd) {
            bins.push((bd.clone(), list_bin(p)));
        }
        let items = &bins.iter().find(|(d, _)| d == &bd).unwrap().1;
        if let Some(r) = items.iter().find(|r| r.deleted >= t0 && path_eq(&r.original, p)) {
            out.push(r.clone());
        }
    }
    Ok(out)
}

/// Put a recycled item back where it was. Fails (and changes nothing) when something already uses that path.
pub fn restore(r: &Recycled) -> Res<()> {
    if r.original.exists() {
        return Err(format!("cannot restore: {} already exists", r.original.display()));
    }
    if !r.data.exists() {
        return Err(format!("cannot restore {}: it is no longer in the Recycle Bin", r.original.display()));
    }
    if let Some(parent) = r.original.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let (a, b) = (wide_path(&r.data), wide_path(&r.original));
    unsafe { MoveFileExW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr()), MOVEFILE_COPY_ALLOWED) }
        .map_err(|e| win_err(&format!("cannot restore {}", r.original.display()), &e))?;
    let _ = std::fs::remove_file(&r.record);
    Ok(())
}

/// Remove one item from the Recycle Bin for good ("Delete" inside the Recycle Bin).
pub fn purge(r: &Recycled) -> Res<()> {
    if journal_exists(&r.data) {
        delete_permanently(&r.data)?;
    }
    let _ = std::fs::remove_file(&r.record);
    Ok(())
}

fn journal_exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

/// Delete for good (no Recycle Bin). Clears read-only flags first. Folders are removed with all their content.
pub fn delete_permanently(p: &Path) -> Res<()> {
    let md = std::fs::symlink_metadata(p).map_err(|_| format!("not found: {}", p.display()))?;
    clear_readonly(p);
    if md.is_dir() && !md.file_type().is_symlink() {
        // Clear read-only below, then remove the tree.
        let mut stack = vec![p.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    let ep = e.path();
                    clear_readonly(&ep);
                    if e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
                        stack.push(ep);
                    }
                }
            }
        }
        std::fs::remove_dir_all(p).map_err(|e| format!("cannot delete {}: {e}", p.display()))
    } else if md.is_dir() {
        std::fs::remove_dir(p).map_err(|e| format!("cannot delete {}: {e}", p.display()))
    } else {
        std::fs::remove_file(p).map_err(|e| format!("cannot delete {}: {e}", p.display()))
    }
}

fn clear_readonly(p: &Path) {
    if let Ok(md) = std::fs::symlink_metadata(p) {
        use std::os::windows::fs::MetadataExt;
        let a = md.file_attributes();
        if a & ATTR_READONLY != 0 {
            let w = wide_path(p);
            unsafe {
                let _ = SetFileAttributesW(PCWSTR(w.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(a & !ATTR_READONLY));
            }
        }
    }
}
