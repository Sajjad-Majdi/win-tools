//! Drag and drop / clipboard helpers for the GUI: build CF_HDROP data or a full shell IDataObject for a list of
//! paths (what other apps understand when you drag files out), and read paths from a dropped IDataObject.
use crate::util::*;
use std::path::PathBuf;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_HDROP, ReleaseStgMedium};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{BHID_DataObject, ILCreateFromPathW, ILFree, SHCreateShellItemArrayFromIDLists};
use windows::core::PCWSTR;

const DROPFILES_SIZE: usize = 20;

/// CF_HDROP bytes (a DROPFILES header + a double-NUL-terminated wide path list).
pub fn hdrop_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let mut b = Vec::with_capacity(DROPFILES_SIZE + paths.len() * 64);
    b.extend((DROPFILES_SIZE as u32).to_le_bytes()); // pFiles
    b.extend([0u8; 8]); // pt
    b.extend(0u32.to_le_bytes()); // fNC
    b.extend(1u32.to_le_bytes()); // fWide
    for p in paths {
        for u in abs(p).as_os_str().to_string_lossy().encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        b.extend([0, 0]);
    }
    b.extend([0, 0]);
    b
}

/// Paths from CF_HDROP bytes (wide or ANSI lists).
pub fn paths_from_hdrop(b: &[u8]) -> Vec<PathBuf> {
    if b.len() < DROPFILES_SIZE {
        return vec![];
    }
    let off = u32::from_le_bytes(b[0..4].try_into().unwrap()) as usize;
    let wide_list = u32::from_le_bytes(b[16..20].try_into().unwrap()) != 0;
    let mut out = vec![];
    let data = &b[off.min(b.len())..];
    if wide_list {
        let w: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        for s in w.split(|&c| c == 0) {
            if s.is_empty() {
                break;
            }
            out.push(PathBuf::from(String::from_utf16_lossy(s)));
        }
    } else {
        for s in data.split(|&c| c == 0) {
            if s.is_empty() {
                break;
            }
            out.push(PathBuf::from(String::from_utf8_lossy(s).into_owned()));
        }
    }
    out
}

/// CF_HDROP in a movable HGLOBAL (for SetClipboardData or a custom IDataObject). The receiver owns it.
pub fn hdrop_global(paths: &[PathBuf]) -> Res<HGLOBAL> {
    let b = hdrop_bytes(paths);
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, b.len()).map_err(|e| win_err("GlobalAlloc", &e))?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

/// The shell's own data object for these files (CF_HDROP, shell ID lists, file names...): hand it to DoDragDrop
/// or OleSetClipboard. Needs COM on this thread for as long as it is used.
pub fn data_object(paths: &[PathBuf]) -> Res<IDataObject> {
    let mut pidls: Vec<*mut ITEMIDLIST> = vec![];
    for p in paths {
        let w = wide(abs(p).as_os_str());
        let pidl = unsafe { ILCreateFromPathW(PCWSTR(w.as_ptr())) };
        if pidl.is_null() {
            for x in &pidls {
                unsafe { ILFree(Some(*x)) };
            }
            return Err(format!("not found: {}", p.display()));
        }
        pidls.push(pidl);
    }
    let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
    let r = unsafe { SHCreateShellItemArrayFromIDLists(&consts) }.and_then(|arr| unsafe { arr.BindToHandler(None, &BHID_DataObject) });
    for x in &pidls {
        unsafe { ILFree(Some(*x)) };
    }
    r.map_err(|e| win_err("cannot build the drag data", &e))
}

/// File paths from a dropped (or pasted) IDataObject. Empty when it holds no files.
pub fn paths_from_data_object(d: &IDataObject) -> Res<Vec<PathBuf>> {
    let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    unsafe {
        let mut med = match d.GetData(&fmt) {
            Ok(m) => m,
            Err(_) => return Ok(vec![]),
        };
        let h = med.u.hGlobal;
        let n = GlobalSize(h);
        let p = GlobalLock(h) as *const u8;
        let out = if p.is_null() { vec![] } else { paths_from_hdrop(std::slice::from_raw_parts(p, n)) };
        let _ = GlobalUnlock(h);
        ReleaseStgMedium(&mut med);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hdrop_round_trip() {
        let paths = vec![PathBuf::from(r"C:\a b\c.txt"), PathBuf::from(r"D:\گزارش\x.pdf")];
        let b = hdrop_bytes(&paths);
        assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 20);
        assert_eq!(paths_from_hdrop(&b), paths);
        assert!(paths_from_hdrop(&hdrop_bytes(&[])).is_empty());
        // ANSI list
        let mut a = vec![20u8, 0, 0, 0];
        a.extend([0u8; 12]);
        a.extend([0u8; 4]);
        a.extend(b"C:\\x.txt\0C:\\y.txt\0\0");
        assert_eq!(paths_from_hdrop(&a), [PathBuf::from(r"C:\x.txt"), PathBuf::from(r"C:\y.txt")]);
    }
}
