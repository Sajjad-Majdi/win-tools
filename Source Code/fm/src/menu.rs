//! The shell right-click menu (IContextMenu) for one or more items: Windows' own entries plus app extensions
//! (7-Zip, "Open in Terminal", Git, ...). List the entries, or run one by its verb or its text.
use crate::util::*;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DIB_RGB_COLORS, DeleteDC, DeleteObject, FillRect, GdiFlush, GetDIBits, GetObjectW, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODA_DRAWENTIRE, ODS_DEFAULT, ODT_MENU};
use windows::Win32::UI::Shell::IContextMenu3;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, MIIM_BITMAP, MIIM_CHECKMARKS, MIIM_DATA, SM_CXSMICON, WM_DRAWITEM, WM_MEASUREITEM};
use windows::core::Interface;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{CMF_EXTENDEDVERBS, CMF_NORMAL, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, GCS_VERBW, IContextMenu, ILCreateFromPathW, ILFree, IShellFolder, SHBindToParent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STATE,
    MIIM_STRING, MIIM_SUBMENU, SW_SHOWNORMAL,
};
use windows::core::{PCSTR, PCWSTR, PSTR};

/// One entry of the right-click menu.
#[derive(Clone, Debug)]
pub struct MenuItem {
    /// Command id (valid only inside one `list`/`invoke` call).
    pub id: u32,
    /// Language-independent verb when the handler gives one ("open", "delete", "properties", ...), else "".
    pub verb: String,
    /// The text people see (without the & accelerator).
    pub text: String,
    /// Submenu path, e.g. "7-Zip" or "Send to" ("" for the top level).
    pub parent: String,
    pub enabled: bool,
    /// The entry's own small icon (from the menu bitmap, the handler's own drawing, or the verb's program), if any.
    pub icon: Option<Icon>,
}

/// A small icon: BGRA, premultiplied alpha, rows top to bottom.
#[derive(Clone)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Icon({}x{})", self.width, self.height)
    }
}

const FIRST: u32 = 1;
const LAST: u32 = 0x7FFF;

struct Menu {
    cm: IContextMenu,
    hmenu: HMENU,
    pidls: Vec<*mut ITEMIDLIST>,
    /// registry classes of the items (for the verb's program icon)
    classes: Vec<String>,
    icons: bool,
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.hmenu);
            for p in &self.pidls {
                ILFree(Some(*p));
            }
        }
    }
}

fn build(paths: &[PathBuf], extended: bool) -> Res<Menu> {
    if paths.is_empty() {
        return Err("no items".into());
    }
    let mut pidls = vec![];
    let mut parent: Option<IShellFolder> = None;
    let mut kids: Vec<*const ITEMIDLIST> = vec![];
    let first_parent = abs(&paths[0]).parent().map(|p| p.to_path_buf());
    for p in paths {
        let p = abs(p);
        if !std::fs::symlink_metadata(&p).is_ok() {
            return Err(format!("not found: {}", p.display()));
        }
        if p.parent().map(|x| x.to_path_buf()) != first_parent {
            return Err("all items of one menu must be in the same folder".into());
        }
        let w = wide(p.as_os_str());
        let pidl = unsafe { ILCreateFromPathW(PCWSTR(w.as_ptr())) };
        if pidl.is_null() {
            return Err(format!("cannot open {}", p.display()));
        }
        pidls.push(pidl);
        let mut last: *mut ITEMIDLIST = std::ptr::null_mut();
        let f: IShellFolder = unsafe { SHBindToParent(pidl, Some(&mut last)) }.map_err(|e| win_err("cannot open the folder", &e))?;
        if parent.is_none() {
            parent = Some(f);
        }
        kids.push(last);
    }
    let cleanup = |pidls: &Vec<*mut ITEMIDLIST>| {
        for p in pidls {
            unsafe { ILFree(Some(*p)) };
        }
    };
    let cm: IContextMenu = match unsafe { parent.unwrap().GetUIObjectOf(HWND::default(), &kids, None) } {
        Ok(c) => c,
        Err(e) => {
            cleanup(&pidls);
            return Err(win_err("no menu for these items", &e));
        }
    };
    let hmenu = match unsafe { CreatePopupMenu() } {
        Ok(h) => h,
        Err(e) => {
            cleanup(&pidls);
            return Err(win_err("CreatePopupMenu", &e));
        }
    };
    let m = Menu { cm, hmenu, pidls, classes: classes_of(&paths[0]), icons: false };
    let flags = CMF_NORMAL | if extended { CMF_EXTENDEDVERBS } else { 0 };
    let hr = unsafe { m.cm.QueryContextMenu(m.hmenu, 0, FIRST, LAST, flags) };
    if hr.is_err() {
        return Err(win_err("the menu could not be built", &windows::core::Error::from(hr)));
    }
    Ok(m)
}

fn walk(m: &Menu, h: HMENU, parent: &str, out: &mut Vec<MenuItem>) {
    let n = unsafe { GetMenuItemCount(Some(h)) };
    for i in 0..n.max(0) as u32 {
        let mut buf = vec![0u16; 512];
        let mut mi = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STRING | MIIM_ID | MIIM_SUBMENU | MIIM_FTYPE | MIIM_STATE | MIIM_BITMAP | MIIM_CHECKMARKS | MIIM_DATA,
            dwTypeData: windows::core::PWSTR(buf.as_mut_ptr()),
            cch: buf.len() as u32 - 1,
            ..Default::default()
        };
        if unsafe { GetMenuItemInfoW(h, i, true, &mut mi) }.is_err() || mi.fType.0 & MFT_SEPARATOR.0 != 0 {
            continue;
        }
        let text = from_wide(&buf).replace('&', "");
        if !mi.hSubMenu.is_invalid() {
            let p = if parent.is_empty() { text.clone() } else { format!("{parent} > {text}") };
            walk(m, mi.hSubMenu, &p, out);
            continue;
        }
        if mi.wID < FIRST || mi.wID > LAST || text.is_empty() {
            continue;
        }
        let mut vb = vec![0u16; 256];
        let verb = unsafe {
            if m.cm.GetCommandString((mi.wID - FIRST) as usize, GCS_VERBW, None, PSTR(vb.as_mut_ptr() as *mut u8), vb.len() as u32).is_ok() { from_wide(&vb) } else { String::new() }
        };
        let icon = if m.icons { item_icon(m, &mi, &verb) } else { None };
        out.push(MenuItem { id: mi.wID, verb, text, parent: parent.to_string(), enabled: mi.fState.0 & MFS_DISABLED.0 == 0, icon });
    }
}

/// The right-click menu of these items (all in one folder). `extended` = the Shift+right-click menu.
pub fn list(paths: &[PathBuf], extended: bool) -> Res<Vec<MenuItem>> {
    let _c = com();
    let mut m = build(paths, extended)?;
    m.icons = true;
    let mut out = vec![];
    walk(&m, m.hmenu, "", &mut out);
    Ok(out)
}

/// Run one menu entry, chosen by verb (exact) or by its text (exact, then "contains"). No error dialogs; the call
/// waits until the handler is done. Returns the entry that ran.
pub fn invoke(paths: &[PathBuf], key: &str, extended: bool) -> Res<MenuItem> {
    let _c = com();
    let m = build(paths, extended)?;
    let mut items = vec![];
    walk(&m, m.hmenu, "", &mut items);
    let k = key.to_lowercase();
    let it = items
        .iter()
        .find(|i| i.verb.to_lowercase() == k)
        .or_else(|| items.iter().find(|i| i.text.to_lowercase() == k))
        .or_else(|| items.iter().find(|i| i.text.to_lowercase().contains(&k)))
        .cloned()
        .ok_or_else(|| format!("no menu entry '{key}' (see: wfm menu <path>)"))?;
    if !it.enabled {
        return Err(format!("'{}' is disabled here", it.text));
    }
    let dir = paths[0].parent().map(|p| wide(abs(p).as_os_str())).unwrap_or_else(|| wide(""));
    let verb_a: Vec<u8> = it.verb.bytes().chain(std::iter::once(0)).collect();
    let verb_w = wide(&it.verb);
    let by_verb = !it.verb.is_empty() && it.verb.is_ascii();
    let offset = (it.id - FIRST) as usize;
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        // CMIC_MASK_UNICODE | CMIC_MASK_FLAG_NO_UI | CMIC_MASK_NOASYNC
        fMask: 0x4000 | 0x400 | 0x100,
        lpVerb: if by_verb { PCSTR(verb_a.as_ptr()) } else { PCSTR(offset as *const u8) },
        lpVerbW: if by_verb { PCWSTR(verb_w.as_ptr()) } else { PCWSTR(offset as *const u16) },
        lpDirectoryW: PCWSTR(dir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { m.cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO) }.map_err(|e| win_err(&format!("'{}' failed", it.text), &e))?;
    Ok(it)
}

// ---------------------------------------------------------------------- icons of the entries

/// The entry's icon: 1) the bitmap the handler put on the menu item, 2) the handler's own drawing (owner-drawn
/// items, IContextMenu3), 3) the "Icon" value or the program of the verb in the registry.
fn item_icon(m: &Menu, mi: &MENUITEMINFOW, verb: &str) -> Option<Icon> {
    bitmap_icon(mi.hbmpItem)
        .or_else(|| bitmap_icon(mi.hbmpUnchecked))
        .or_else(|| if mi.hbmpItem.0 as isize == -1 { owner_drawn_icon(m, mi) } else { None })
        .or_else(|| verb_icon(&m.classes, verb))
}

fn special_bitmap(h: HBITMAP) -> bool {
    (-1..=11).contains(&(h.0 as isize))
}

/// 32-bit pixels of a bitmap (top-down); None for empty or special menu bitmaps.
fn bitmap_icon(h: HBITMAP) -> Option<Icon> {
    if special_bitmap(h) {
        return None;
    }
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(HGDIOBJ(h.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
            return None;
        }
        let (w, ht) = (bm.bmWidth.max(0) as u32, bm.bmHeight.unsigned_abs());
        if w == 0 || ht == 0 || w > 256 || ht > 256 {
            return None;
        }
        let mut px = vec![0u8; (w * ht * 4) as usize];
        let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w as i32, biHeight: -(ht as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let dc = CreateCompatibleDC(None);
        let got = GetDIBits(dc, h, 0, ht, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if got == 0 {
            return None;
        }
        let has_alpha = bm.bmBitsPixel == 32 && px.chunks_exact(4).any(|p| p[3] != 0);
        if !has_alpha {
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
        } else if px.chunks_exact(4).any(|p| p[0] > p[3] || p[1] > p[3] || p[2] > p[3]) {
            for p in px.chunks_exact_mut(4) {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * a + 127) / 255) as u8;
                }
            }
        }
        Some(Icon { width: w, height: ht, bgra: px })
    }
}

/// Draw something twice, on black and on white, and recover colour + alpha from the two results.
fn render_alpha(w: u32, h: u32, draw: impl Fn(HDC)) -> Option<Icon> {
    unsafe {
        let mut shots = vec![];
        for bg in [0u32, 0x00FF_FFFF] {
            let dc = CreateCompatibleDC(None);
            let bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w as i32, biHeight: -(h as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            let br = CreateSolidBrush(COLORREF(bg));
            FillRect(dc, &RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }, br);
            let _ = DeleteObject(HGDIOBJ(br.0));
            draw(dc);
            let _ = GdiFlush();
            shots.push(std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec());
            SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(dc);
        }
        let (b, wt) = (&shots[0], &shots[1]);
        let mut px = vec![0u8; (w * h * 4) as usize];
        let mut any = false;
        for i in 0..(w * h) as usize {
            let k = i * 4;
            let d = ((wt[k] as i32 - b[k] as i32) + (wt[k + 1] as i32 - b[k + 1] as i32) + (wt[k + 2] as i32 - b[k + 2] as i32)) / 3;
            let a = (255 - d).clamp(0, 255) as u8;
            px[k] = b[k].min(a);
            px[k + 1] = b[k + 1].min(a);
            px[k + 2] = b[k + 2].min(a);
            px[k + 3] = a;
            any |= a > 8;
        }
        any.then_some(Icon { width: w, height: h, bgra: px })
    }
}

/// Owner-drawn menu bitmaps (HBMMENU_CALLBACK): ask the handler to measure and draw the item's bitmap.
fn owner_drawn_icon(m: &Menu, mi: &MENUITEMINFOW) -> Option<Icon> {
    let cm3: IContextMenu3 = m.cm.cast().ok()?;
    unsafe {
        let mut mis = MEASUREITEMSTRUCT { CtlType: ODT_MENU, CtlID: 0, itemID: mi.wID, itemWidth: 0, itemHeight: 0, itemData: mi.dwItemData };
        let _ = cm3.HandleMenuMsg2(WM_MEASUREITEM, WPARAM(0), LPARAM(&mut mis as *mut _ as isize), None);
        let size = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let (w, h) = (mis.itemWidth.clamp(size, 64), mis.itemHeight.clamp(size, 64));
        let hmenu = m.hmenu;
        render_alpha(w, h, |dc| {
            let mut dis = DRAWITEMSTRUCT { CtlType: ODT_MENU, CtlID: 0, itemID: mi.wID, itemAction: ODA_DRAWENTIRE, itemState: ODS_DEFAULT, hwndItem: HWND(hmenu.0), hDC: dc, rcItem: RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }, itemData: mi.dwItemData };
            let _ = cm3.HandleMenuMsg2(WM_DRAWITEM, WPARAM(0), LPARAM(&mut dis as *mut _ as isize), None);
        })
        .map(|i| crop(i, size))
    }
}

/// Keep the opaque part of an owner-drawn bitmap when it is bigger than an icon.
fn crop(i: Icon, size: u32) -> Icon {
    if i.width <= size && i.height <= size {
        return i;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i.width, i.height, 0, 0);
    for y in 0..i.height {
        for x in 0..i.width {
            if i.bgra[((y * i.width + x) * 4 + 3) as usize] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return i;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let s = (((y0 + y) * i.width + x0) * 4) as usize;
        px[(y * w * 4) as usize..((y + 1) * w * 4) as usize].copy_from_slice(&i.bgra[s..s + (w * 4) as usize]);
    }
    Icon { width: w, height: h, bgra: px }
}

/// Registry classes whose "shell" keys can hold the verbs of this item.
fn classes_of(p: &Path) -> Vec<String> {
    let p = abs(p);
    let s = p.to_string_lossy().to_string();
    if s.len() <= 3 {
        return vec!["Drive".into(), "Directory".into(), "Folder".into(), "AllFilesystemObjects".into()];
    }
    if p.is_dir() {
        return vec!["Directory".into(), "Folder".into(), "AllFilesystemObjects".into(), r"Directory\Background".into()];
    }
    let ext = format!(".{}", ext_of(&s));
    let mut v = vec![];
    if let Some(prog) = reg_string(&ext, "") {
        v.push(prog);
    }
    v.push(format!(r"SystemFileAssociations\{ext}"));
    v.push(ext);
    v.push("*".into());
    v.push("AllFilesystemObjects".into());
    v
}

fn reg_string(key: &str, value: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    let k = wide(key);
    let v = wide(value);
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let name = if value.is_empty() { PCWSTR::null() } else { PCWSTR(v.as_ptr()) };
    let r = unsafe { RegGetValueW(HKEY_CLASSES_ROOT, PCWSTR(k.as_ptr()), name, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) };
    if r.0 != 0 {
        return None;
    }
    let s = from_wide(&buf);
    (!s.trim().is_empty()).then_some(s)
}

/// %VAR% -> its value.
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out += &rest[..i];
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                out += &std::env::var(name).unwrap_or_else(|_| format!("%{name}%"));
                rest = &after[j + 1..];
            }
            None => {
                out += &rest[i..];
                rest = "";
            }
        }
    }
    out + rest
}

/// "path,index" (an Icon value) or the program of a command line -> (file, index).
fn icon_source(s: &str, command: bool) -> Option<(String, i32)> {
    let s = expand(s.trim());
    if command {
        let exe = if let Some(r) = s.strip_prefix('"') {
            r.split('"').next()?.to_string()
        } else {
            match s.to_lowercase().find(".exe") {
                Some(i) => s[..i + 4].to_string(),
                None => s.split_whitespace().next()?.to_string(),
            }
        };
        return Some((exe, 0));
    }
    let (file, idx) = match s.rsplit_once(',') {
        Some((f, i)) if i.trim().parse::<i32>().is_ok() => (f.to_string(), i.trim().parse().unwrap_or(0)),
        _ => (s.clone(), 0),
    };
    Some((file.trim().trim_matches('"').to_string(), idx))
}

/// The verb's own icon from the registry: shell\<verb> "Icon", else the icon of its program.
fn verb_icon(classes: &[String], verb: &str) -> Option<Icon> {
    if verb.is_empty() || !verb.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        return None;
    }
    for c in classes {
        let key = format!(r"{c}\shell\{verb}");
        let src = reg_string(&key, "Icon").and_then(|s| icon_source(&s, false)).or_else(|| reg_string(&format!(r"{key}\command"), "").and_then(|s| icon_source(&s, true)));
        if let Some((file, idx)) = src
            && let Some(i) = file_icon(&file, idx)
        {
            return Some(i);
        }
    }
    None
}

fn file_icon(file: &str, idx: i32) -> Option<Icon> {
    use windows::Win32::UI::Shell::ExtractIconExW;
    use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
    if file.is_empty() {
        return None;
    }
    let w = wide(file);
    let mut small = HICON::default();
    let n = unsafe { ExtractIconExW(PCWSTR(w.as_ptr()), idx, None, Some(&mut small), 1) };
    if n == 0 || small.is_invalid() {
        return None;
    }
    let size = unsafe { GetSystemMetrics(SM_CXSMICON).max(16) } as u32;
    let r = render_alpha(size, size, |dc| unsafe {
        let _ = DrawIconEx(dc, 0, 0, small, size as i32, size as i32, 0, None, DI_NORMAL);
    });
    unsafe {
        let _ = DestroyIcon(small);
    }
    r
}

/// Convenience for one path.
pub fn list_one(path: &Path) -> Res<Vec<MenuItem>> {
    list(&[path.to_path_buf()], false)
}
