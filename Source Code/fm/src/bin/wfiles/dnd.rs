//! OLE drag and drop: the window is a drop target (files from Explorer, other apps, or our own panes) and a drag
//! source (DoDragDrop with the shell's own data object from fmcore::dnd).
use std::path::PathBuf;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::core::{BOOL, HRESULT, Ref, Result, implement};

/// What the window does with drag events (runs on the UI thread).
pub trait Sink {
    fn drag_enter(&mut self, paths: Vec<PathBuf>, x: i32, y: i32, keys: u32) -> u32;
    fn drag_over(&mut self, x: i32, y: i32, keys: u32) -> u32;
    fn drag_leave(&mut self);
    fn drop_files(&mut self, x: i32, y: i32, keys: u32) -> u32;
}

pub type SinkFn = fn(isize) -> Option<&'static mut dyn Sink>;

#[implement(IDropTarget)]
pub struct Target {
    hwnd: isize,
    get: SinkFn,
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = data.as_ref().and_then(|d| fmcore::dnd::paths_from_data_object(d).ok()).unwrap_or_default();
        let e = match (self.get)(self.hwnd) {
            Some(s) if !paths.is_empty() => s.drag_enter(paths, pt.x, pt.y, keys.0),
            _ => 0,
        };
        unsafe { *effect = DROPEFFECT(e) };
        Ok(())
    }
    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let e = (self.get)(self.hwnd).map(|s| s.drag_over(pt.x, pt.y, keys.0)).unwrap_or(0);
        unsafe { *effect = DROPEFFECT(e) };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        if let Some(s) = (self.get)(self.hwnd) {
            s.drag_leave();
        }
        Ok(())
    }
    fn Drop(&self, _data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let e = (self.get)(self.hwnd).map(|s| s.drop_files(pt.x, pt.y, keys.0)).unwrap_or(0);
        unsafe { *effect = DROPEFFECT(e) };
        Ok(())
    }
}

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & 0x0001 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// OleInitialize + RegisterDragDrop for the window.
pub fn register(hwnd: HWND, get: SinkFn) {
    unsafe {
        let _ = OleInitialize(None);
        let t: IDropTarget = Target { hwnd: hwnd.0 as isize, get }.into();
        let _ = RegisterDragDrop(hwnd, &t);
    }
}

pub fn revoke(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

/// Drag these files out (modal until the drop). Returns the effect (1 copy, 2 move, 4 link, 0 none).
pub fn drag_out(paths: &[PathBuf]) -> u32 {
    let Ok(data) = fmcore::dnd::data_object(paths) else { return 0 };
    let src: IDropSource = Source.into();
    let mut eff = DROPEFFECT(0);
    unsafe {
        let _ = DoDragDrop(&data, &src, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut eff);
    }
    eff.0
}

/// Files on the clipboard (CF_HDROP) and whether they were cut (Preferred DropEffect = move).
pub fn clipboard_files() -> (Vec<PathBuf>, bool) {
    unsafe {
        let Ok(d) = OleGetClipboard() else { return (vec![], false) };
        let paths = fmcore::dnd::paths_from_data_object(&d).unwrap_or_default();
        let mut cut = false;
        let name = fmcore::util::wide("Preferred DropEffect");
        let cf = windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::PCWSTR(name.as_ptr()));
        let fmt = windows::Win32::System::Com::FORMATETC { cfFormat: cf as u16, ptd: std::ptr::null_mut(), dwAspect: 1, lindex: -1, tymed: 1 };
        if let Ok(mut med) = d.GetData(&fmt) {
            let p = windows::Win32::System::Memory::GlobalLock(med.u.hGlobal) as *const u32;
            if !p.is_null() {
                cut = *p & 2 != 0;
                let _ = windows::Win32::System::Memory::GlobalUnlock(med.u.hGlobal);
            }
            ReleaseStgMedium(&mut med);
        }
        (paths, cut)
    }
}

/// Put files on the clipboard (CF_HDROP + Preferred DropEffect), like Explorer's Copy / Cut.
pub fn set_clipboard_files(hwnd: HWND, paths: &[PathBuf], cut: bool) -> bool {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = fmcore::dnd::hdrop_global(paths) {
            let _ = SetClipboardData(15, Some(HANDLE(h.0)));
        }
        let name = fmcore::util::wide("Preferred DropEffect");
        let cf = RegisterClipboardFormatW(windows::core::PCWSTR(name.as_ptr()));
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, 4) {
            let p = GlobalLock(h) as *mut u32;
            *p = if cut { 2 } else { 5 };
            let _ = GlobalUnlock(h);
            let _ = SetClipboardData(cf, Some(HANDLE(h.0)));
        }
        let _ = CloseClipboard();
        true
    }
}

pub fn set_clipboard_text(hwnd: HWND, s: &str) {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    let w = fmcore::util::wide(s);
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            let _ = GlobalUnlock(h);
            let _ = SetClipboardData(13, Some(HANDLE(h.0)));
        }
        let _ = CloseClipboard();
    }
}

pub fn clipboard_text(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let r = GetClipboardData(13).ok().map(|h| {
            let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
            let mut n = 0;
            while !p.is_null() && *p.add(n) != 0 {
                n += 1;
            }
            let s = if p.is_null() { String::new() } else { String::from_utf16_lossy(std::slice::from_raw_parts(p, n)) };
            let _ = GlobalUnlock(HGLOBAL(h.0));
            s
        });
        let _ = CloseClipboard();
        r
    }
}
