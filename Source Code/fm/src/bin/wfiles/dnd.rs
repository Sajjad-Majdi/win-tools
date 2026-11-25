//! OLE drag and drop: the window is a drop target (files from Explorer, other apps, or our own panes) and a drag
//! source (SHDoDragDrop with the shell's own data object from fmcore::dnd, so the shell draws the drag image: the
//! items' icons / thumbnails with a count badge, as Explorer does). Over our window the shell's drag image helper
//! keeps drawing that image and we write the "Move to <folder>" line under it (DropDescription).
use std::cell::RefCell;
use std::path::PathBuf;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{CLSID_DragDropHelper, DROPDESCRIPTION, DROPIMAGE_COPY, DROPIMAGE_INVALID, DROPIMAGE_MOVE, DROPIMAGE_NONE, DROPIMAGETYPE, IDropTargetHelper, SHDoDragDrop};
use windows::core::{BOOL, HRESULT, Ref, Result, implement, w};

/// What the window does with drag events (runs on the UI thread). Effects: 0 none, 1 copy, 2 move; `allowed` is
/// what the source permits (the same bits).
pub trait Sink {
    fn drag_enter(&mut self, paths: Vec<PathBuf>, x: i32, y: i32, keys: u32, allowed: u32, image: bool) -> u32;
    fn drag_over(&mut self, x: i32, y: i32, keys: u32, allowed: u32) -> u32;
    fn drag_leave(&mut self);
    fn drop_files(&mut self, x: i32, y: i32, keys: u32, allowed: u32) -> u32;
    /// the name of the folder a drop goes to now ("Move to <name>")
    fn drop_name(&self) -> String;
}

pub type SinkFn = fn(isize) -> Option<&'static mut dyn Sink>;

#[derive(Default)]
struct TargetState {
    helper: Option<IDropTargetHelper>,
    helper_tried: bool,
    data: Option<IDataObject>,
    /// the description last written (effect, folder name): written again only when it changes
    said: Option<(u32, String)>,
}

#[implement(IDropTarget)]
pub struct Target {
    hwnd: isize,
    get: SinkFn,
    st: RefCell<TargetState>,
}

impl Target {
    fn helper(&self) -> Option<IDropTargetHelper> {
        let mut st = self.st.borrow_mut();
        if !st.helper_tried {
            st.helper_tried = true;
            st.helper = unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() };
        }
        st.helper.clone()
    }

    /// "Move to <folder>" / "Copy to <folder>" under the drag image (only when it changed).
    fn describe(&self, effect: u32) {
        let name = if effect == 0 { String::new() } else { (self.get)(self.hwnd).map(|s| s.drop_name()).unwrap_or_default() };
        let data = {
            let mut st = self.st.borrow_mut();
            let now = Some((effect, name.clone()));
            if st.said == now {
                return;
            }
            st.said = now;
            st.data.clone()
        };
        let Some(d) = data else { return };
        let (kind, msg) = match effect {
            2 => (DROPIMAGE_MOVE, "Move to %1"),
            1 => (DROPIMAGE_COPY, "Copy to %1"),
            _ => (DROPIMAGE_NONE, ""),
        };
        set_description(&d, kind, msg, &name);
    }

    /// The drag left or ended: no description of ours stays on the source's data object.
    fn forget(&self) {
        let data = {
            let mut st = self.st.borrow_mut();
            st.said = None;
            st.data.take()
        };
        if let Some(d) = data {
            set_description(&d, DROPIMAGE_INVALID, "", "");
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let allowed = unsafe { (*effect).0 };
        let d = data.as_ref().cloned();
        let paths = d.as_ref().and_then(|d| fmcore::dnd::paths_from_data_object(d).ok()).unwrap_or_default();
        let image = d.as_ref().map(has_drag_image).unwrap_or(false);
        let e = match (self.get)(self.hwnd) {
            Some(s) if !paths.is_empty() => s.drag_enter(paths, pt.x, pt.y, keys.0, allowed, image),
            _ => 0,
        };
        unsafe { *effect = DROPEFFECT(e) };
        {
            let mut st = self.st.borrow_mut();
            st.data = d.clone();
            st.said = None;
        }
        self.describe(e);
        if let (Some(h), Some(d)) = (self.helper(), d.as_ref()) {
            unsafe {
                let _ = h.DragEnter(HWND(self.hwnd as *mut _), d, &POINT { x: pt.x, y: pt.y }, DROPEFFECT(e));
            }
        }
        Ok(())
    }
    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let allowed = unsafe { (*effect).0 };
        let e = (self.get)(self.hwnd).map(|s| s.drag_over(pt.x, pt.y, keys.0, allowed)).unwrap_or(0);
        unsafe { *effect = DROPEFFECT(e) };
        self.describe(e);
        if let Some(h) = self.helper() {
            unsafe {
                let _ = h.DragOver(&POINT { x: pt.x, y: pt.y }, DROPEFFECT(e));
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        if let Some(s) = (self.get)(self.hwnd) {
            s.drag_leave();
        }
        if let Some(h) = self.helper() {
            unsafe {
                let _ = h.DragLeave();
            }
        }
        self.forget();
        Ok(())
    }
    fn Drop(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let allowed = unsafe { (*effect).0 };
        let e = (self.get)(self.hwnd).map(|s| s.drop_files(pt.x, pt.y, keys.0, allowed)).unwrap_or(0);
        unsafe { *effect = DROPEFFECT(e) };
        if let (Some(h), Some(d)) = (self.helper(), data.as_ref()) {
            unsafe {
                let _ = h.Drop(d, &POINT { x: pt.x, y: pt.y }, DROPEFFECT(e));
            }
        }
        self.forget();
        Ok(())
    }
}

/// The source put a drag image on the data object (the shell's helpers do: Explorer, SHDoDragDrop, browsers).
fn has_drag_image(d: &IDataObject) -> bool {
    unsafe {
        let cf = RegisterClipboardFormatW(w!("DragImageBits"));
        let fmt = FORMATETC { cfFormat: cf as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        cf != 0 && d.QueryGetData(&fmt) == S_OK
    }
}

/// Write the DropDescription the drag image shows under itself (`%1` in `msg` becomes `insert`, drawn in bold).
fn set_description(d: &IDataObject, kind: DROPIMAGETYPE, msg: &str, insert: &str) {
    unsafe {
        let cf = RegisterClipboardFormatW(w!("DropDescription"));
        if cf == 0 {
            return;
        }
        // DROPDESCRIPTION is packed: written as bytes (type, then two NUL-terminated 260-char strings)
        let mut b = vec![0u8; size_of::<DROPDESCRIPTION>()];
        b[..4].copy_from_slice(&kind.0.to_le_bytes());
        for (off, s) in [(4usize, msg), (4 + 520, insert)] {
            for (i, u) in s.encode_utf16().take(259).enumerate() {
                b[off + i * 2..off + i * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
        }
        let Ok(h) = GlobalAlloc(GHND, b.len()) else { return };
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return;
        }
        std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
        let _ = GlobalUnlock(h);
        let fmt = FORMATETC { cfFormat: cf as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        let med = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: h }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
        // with fRelease the data object owns the memory on success; on failure it is still ours
        if d.SetData(&fmt, &med, true).is_err() {
            let _ = GlobalFree(Some(h));
        }
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
        let t: IDropTarget = Target { hwnd: hwnd.0 as isize, get, st: RefCell::new(TargetState::default()) }.into();
        let _ = RegisterDragDrop(hwnd, &t);
    }
}

pub fn revoke(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

/// Drag these files out (modal until the drop). The shell makes the drag image and the default drop source (which
/// shows drop descriptions such as Explorer's "Move to Documents"). Returns the effect (1 copy, 2 move, 4 link, 0 none).
pub fn drag_out(hwnd: HWND, paths: &[PathBuf]) -> u32 {
    let Ok(data) = fmcore::dnd::data_object(paths) else { return 0 };
    let effects = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
    match unsafe { SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, effects) } {
        Ok(e) => e.0,
        Err(_) => {
            // no shell helper (should not happen): plain OLE drag with the default cursors
            let src: IDropSource = Source.into();
            let mut eff = DROPEFFECT(0);
            unsafe {
                let _ = DoDragDrop(&data, &src, effects, &mut eff);
            }
            eff.0
        }
    }
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

/// Tests of the text boxes (WFILES_TEST_CLIP set) keep copied text inside the process: the hidden test desktop
/// shares the real clipboard with the owner's screen, and a test must not overwrite what he copied.
static TEST_CLIP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
fn test_clip() -> bool {
    std::env::var_os("WFILES_TEST_CLIP").is_some()
}

pub fn set_clipboard_text(hwnd: HWND, s: &str) {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    if test_clip() {
        *TEST_CLIP.lock().unwrap() = Some(s.to_string());
        return;
    }
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
    if test_clip() {
        return TEST_CLIP.lock().unwrap().clone();
    }
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
