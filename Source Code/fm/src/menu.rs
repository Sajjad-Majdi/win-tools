//! The shell right-click menu (IContextMenu) for one or more items: Windows' own entries plus app extensions
//! (7-Zip, "Open in Terminal", Git, ...). List the entries, or run one by its verb or its text.
use crate::util::*;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::HWND;
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
}

const FIRST: u32 = 1;
const LAST: u32 = 0x7FFF;

struct Menu {
    cm: IContextMenu,
    hmenu: HMENU,
    pidls: Vec<*mut ITEMIDLIST>,
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
    let m = Menu { cm, hmenu, pidls };
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
            fMask: MIIM_STRING | MIIM_ID | MIIM_SUBMENU | MIIM_FTYPE | MIIM_STATE,
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
        out.push(MenuItem { id: mi.wID, verb, text, parent: parent.to_string(), enabled: mi.fState.0 & MFS_DISABLED.0 == 0 });
    }
}

/// The right-click menu of these items (all in one folder). `extended` = the Shift+right-click menu.
pub fn list(paths: &[PathBuf], extended: bool) -> Res<Vec<MenuItem>> {
    let _c = com();
    let m = build(paths, extended)?;
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

/// Convenience for one path.
pub fn list_one(path: &Path) -> Res<Vec<MenuItem>> {
    list(&[path.to_path_buf()], false)
}
