//! "Set as default file manager" / "Restore Explorer", per user (HKCU only, no admin rights).
//!
//! `set` makes folders, drives and Win+E open in wfiles.exe:
//! - `HKCU\Software\Classes\Directory\shell` and `...\Drive\shell`: default verb `openinwfiles` with
//!   `"<wfiles.exe>" "%1"`;
//! - `HKCU\Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command` (Win+E).
//! `restore` deletes exactly those keys again, so Explorer is the handler once more.
//! Only the GUI's Settings buttons (a click by the user) and `wfm default set|restore` run them.
use crate::util::{Res, wide};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

const VERB: &str = "openinwfiles";
const WIN_E: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";

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
    let p = wide(path);
    let n = wide(name);
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        let r = RegGetValueW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), if name.is_empty() { PCWSTR::null() } else { PCWSTR(n.as_ptr()) }, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size));
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

/// Is wfiles the per-user default for folders right now? (read only)
pub fn is_default() -> bool {
    get_value(r"Software\Classes\Directory\shell", "").as_deref() == Some(VERB)
}

/// Make `exe` (full path of wfiles.exe) the default for folders, drives and Win+E.
pub fn set(exe: &std::path::Path) -> Res<()> {
    let exe = exe.to_string_lossy();
    if !exe.to_lowercase().ends_with(".exe") {
        return Err("needs the full path of wfiles.exe".into());
    }
    for class in ["Directory", "Drive"] {
        let base = format!(r"Software\Classes\{class}\shell");
        set_value(&format!(r"{base}\{VERB}"), "", "Open in Win Tools Files")?;
        set_value(&format!(r"{base}\{VERB}\command"), "", &format!("\"{exe}\" \"%1\""))?;
        set_value(&base, "", VERB)?;
    }
    set_value(&format!(r"{WIN_E}\shell\opennewwindow\command"), "", &format!("\"{exe}\""))?;
    set_value(&format!(r"{WIN_E}\shell\opennewwindow\command"), "DelegateExecute", "")?;
    set_value(WIN_E, "WinToolsFiles", "1")?;
    Ok(())
}

/// Undo `set`: Explorer opens folders and Win+E again.
pub fn restore() -> Res<()> {
    for class in ["Directory", "Drive"] {
        let base = format!(r"Software\Classes\{class}\shell");
        delete_tree(&format!(r"{base}\{VERB}"));
        if get_value(&base, "").as_deref() == Some(VERB) {
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
    if get_value(WIN_E, "WinToolsFiles").is_some() {
        delete_tree(WIN_E);
    }
    Ok(())
}
