//! The app's own taskbar identity: an AppUserModelID and a jump list (right-click on its taskbar button) with the
//! app's pinned folders and the folders opened lately.
//!
//! Windows does not let any app take over the File Explorer button on the taskbar, so after "Set as default" the
//! user can pin Rust File Explorer there instead; its jump list then offers the same quick folders.
use crate::util::*;
use std::path::Path;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READWRITE};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::*;
use windows::core::{Interface, PCWSTR};

/// Rust File Explorer's AppUserModelID (taskbar grouping, pins and jump list).
pub const AUMID: &str = "WinTools.RustFileExplorer";

const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9), pid: 2 };
const PKEY_AUMID: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

/// The process's own ID: call first thing in the window process (before any window).
pub fn set_process_id() {
    let w = wide(AUMID);
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(PCWSTR(w.as_ptr()));
    }
}

/// A VT_LPWSTR PROPVARIANT (shell links want this type for their title; PropVariantClear frees it).
fn pv_str(s: &str) -> windows::core::Result<PROPVARIANT> {
    let w = wide(s);
    unsafe {
        let p = SHStrDupW(PCWSTR(w.as_ptr()))?;
        let mut v = PROPVARIANT::default();
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = p;
        Ok(v)
    }
}

/// One jump list entry: our exe with the folder as its argument, the folder's name as title, a folder icon.
fn link(exe: &Path, folder: &str, title: &str) -> windows::core::Result<IShellLinkW> {
    unsafe {
        let l: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let e = wide(exe.as_os_str());
        l.SetPath(PCWSTR(e.as_ptr()))?;
        let a = wide(&quote_arg(folder));
        l.SetArguments(PCWSTR(a.as_ptr()))?;
        let d = wide(&folder.to_string());
        let _ = l.SetDescription(PCWSTR(d.as_ptr()));
        let icon = wide(&format!(r"{}\System32\imageres.dll", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())));
        let _ = l.SetIconLocation(PCWSTR(icon.as_ptr()), 3);
        let ps: IPropertyStore = l.cast()?;
        ps.SetValue(&PKEY_TITLE, &pv_str(title)?)?;
        ps.Commit()?;
        Ok(l)
    }
}

/// A folder as one command-line argument: `"C:\x"`, and a drive root as `"C:\\"` (one backslash right before the
/// closing quote would escape it).
pub fn quote_arg(folder: &str) -> String {
    let f = folder.trim_end_matches('\\');
    if f.len() == 2 && f.ends_with(':') { format!("\"{f}\\\\\"") } else { format!("\"{f}\"") }
}

/// The jump list's entries: (category, [(folder, title)]).
pub fn entries(top_recent: usize) -> Vec<(&'static str, Vec<(String, String)>)> {
    let pinned: Vec<(String, String)> = crate::pins::pins().into_iter().filter(|p| p.exists && Path::new(&p.path).is_dir()).take(12).map(|p| (p.path, p.name)).collect();
    let recent: Vec<(String, String)> = crate::pins::recent(80)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.is_dir && r.exists && !pinned.iter().any(|p| path_eq(Path::new(&p.0), Path::new(&r.target))))
        .take(top_recent)
        .map(|r| {
            let name = Path::new(&r.target).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.target.clone());
            (r.target, name)
        })
        .collect();
    vec![("Pinned", pinned), ("Recent folders", recent)]
}

/// Write the jump list for `exe` (RustFileExplorer.exe). Runs in any thread (it starts COM itself).
pub fn update(exe: &Path) -> Res<usize> {
    let _c = com();
    unsafe {
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).map_err(|e| win_err("jump list", &e))?;
        let id = wide(AUMID);
        list.SetAppID(PCWSTR(id.as_ptr())).map_err(|e| win_err("jump list", &e))?;
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&mut slots).map_err(|e| win_err("jump list", &e))?;
        let mut n = 0;
        for (cat, items) in entries((slots as usize).clamp(4, 10)) {
            if items.is_empty() {
                continue;
            }
            let coll: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER).map_err(|e| win_err("jump list", &e))?;
            for (folder, title) in &items {
                if let Ok(l) = link(exe, folder, title) {
                    let _ = coll.AddObject(&l);
                    n += 1;
                }
            }
            let arr: IObjectArray = coll.cast().map_err(|e| win_err("jump list", &e))?;
            let c = wide(cat);
            // a category the user cleared items from may refuse an entry: the list still commits
            let _ = list.AppendCategory(PCWSTR(c.as_ptr()), &arr);
        }
        list.CommitList().map_err(|e| win_err("jump list", &e))?;
        Ok(n)
    }
}

/// Our Start menu shortcut ("Rust File Explorer.lnk", made by the installer) gets our AppUserModelID, so a pin
/// made from it and the running window share one taskbar button. Only a shortcut that points at `exe` itself is
/// touched, and only when its ID differs.
pub fn fix_shortcut(exe: &Path) -> bool {
    let Some(appdata) = std::env::var_os("APPDATA") else { return false };
    let lnk = Path::new(&appdata).join(r"Microsoft\Windows\Start Menu\Programs\Rust File Explorer.lnk");
    if !lnk.is_file() {
        return false;
    }
    let _c = com();
    unsafe {
        let Ok(l): windows::core::Result<IShellLinkW> = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return false };
        let Ok(pf) = l.cast::<IPersistFile>() else { return false };
        let w = wide(lnk.as_os_str());
        if pf.Load(PCWSTR(w.as_ptr()), STGM_READWRITE).is_err() {
            return false;
        }
        let mut buf = [0u16; 260];
        if l.GetPath(&mut buf, std::ptr::null_mut(), 0).is_err() || !path_eq(Path::new(&from_wide(&buf)), exe) {
            return false;
        }
        let Ok(ps) = l.cast::<IPropertyStore>() else { return false };
        let now = ps.GetValue(&PKEY_AUMID).ok().and_then(|v| windows::core::BSTR::try_from(&v).ok()).map(|b| b.to_string()).unwrap_or_default();
        if now == AUMID {
            return false;
        }
        let Ok(v) = pv_str(AUMID) else { return false };
        ps.SetValue(&PKEY_AUMID, &v).is_ok() && ps.Commit().is_ok() && pf.Save(PCWSTR::null(), true).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_arguments_survive_command_line_parsing() {
        assert_eq!(quote_arg(r"C:\Users\Me\My Music"), r#""C:\Users\Me\My Music""#);
        assert_eq!(quote_arg(r"D:\Games\"), r#""D:\Games""#);
        // "C:\" would end in \" (an escaped quote): the root gets a doubled backslash
        assert_eq!(quote_arg(r"C:\"), r#""C:\\""#);
        assert_eq!(quote_arg("E:"), r#""E:\\""#);
    }

    /// Writes the real jump list of Rust File Explorer (run by hand: cargo test -- --ignored jump_list).
    #[test]
    #[ignore]
    fn jump_list_is_written() {
        let exe = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap()).join(r"Programs\Rust File Explorer\RustFileExplorer.exe");
        let n = update(&exe).expect("jump list");
        println!("jump list items: {n}; entries: {:?}", entries(10));
    }
}
