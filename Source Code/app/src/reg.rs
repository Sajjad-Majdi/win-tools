//! Tiny registry helpers.
use crate::sys::wide;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;
use windows::core::PCWSTR;

pub const HKLM: HKEY = HKEY_LOCAL_MACHINE;
pub const HKCU: HKEY = HKEY_CURRENT_USER;

pub fn read_string(root: HKEY, path: &str, name: &str) -> Option<String> {
    let (p, n) = (wide(path), wide(name));
    let mut size = 0u32;
    unsafe {
        if RegGetValueW(root, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u16; size as usize / 2 + 1];
        if RegGetValueW(root, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        Some(crate::sys::from_wide(&buf))
    }
}

pub fn read_binary(root: HKEY, path: &str, name: &str) -> Option<Vec<u8>> {
    let (p, n) = (wide(path), wide(name));
    let mut size = 0u32;
    unsafe {
        if RegGetValueW(root, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_BINARY, None, None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if RegGetValueW(root, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }
}

pub fn write_binary(root: HKEY, path: &str, name: &str, data: &[u8]) -> Result<(), String> {
    let (p, n) = (wide(path), wide(name));
    unsafe {
        let mut k = HKEY::default();
        let r = RegCreateKeyExW(root, PCWSTR(p.as_ptr()), None, None, REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut k, None);
        if r != ERROR_SUCCESS {
            return Err(format!("open key: {}", r.0));
        }
        let r = RegSetValueExW(k, PCWSTR(n.as_ptr()), None, REG_BINARY, Some(data));
        let _ = RegCloseKey(k);
        if r != ERROR_SUCCESS { Err(format!("set value: {}", r.0)) } else { Ok(()) }
    }
}

/// (name, string value) pairs of one key.
pub fn values(root: HKEY, path: &str) -> Vec<(String, String)> {
    let p = wide(path);
    let mut out = vec![];
    unsafe {
        let mut k = HKEY::default();
        if RegOpenKeyExW(root, PCWSTR(p.as_ptr()), None, KEY_READ, &mut k) != ERROR_SUCCESS {
            return out;
        }
        for i in 0.. {
            let mut name = [0u16; 512];
            let mut nlen = name.len() as u32;
            let mut ty = 0u32;
            let mut data = vec![0u8; 8192];
            let mut dlen = data.len() as u32;
            let r = RegEnumValueW(k, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut nlen, None, Some(&mut ty as *mut u32), Some(data.as_mut_ptr()), Some(&mut dlen));
            if r != ERROR_SUCCESS {
                break;
            }
            if ty == REG_SZ.0 || ty == REG_EXPAND_SZ.0 {
                let w: Vec<u16> = data[..dlen as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                out.push((String::from_utf16_lossy(&name[..nlen as usize]), crate::sys::from_wide(&w)));
            }
        }
        let _ = RegCloseKey(k);
    }
    out
}

pub fn read_dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
    let (p, n) = (wide(path), wide(name));
    let mut v = 0u32;
    let mut size = 4u32;
    unsafe {
        if RegGetValueW(root, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
    }
    Some(v)
}
