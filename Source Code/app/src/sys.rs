//! Small FFI helpers: ntdll calls the `windows` crate does not expose cleanly, and wide-string helpers.
#![allow(non_snake_case)]
use std::ffi::c_void;

#[link(name = "ntdll")]
unsafe extern "system" {
    pub fn NtQuerySystemInformation(class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    pub fn NtQueryInformationProcess(h: *mut c_void, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

pub const SYSTEM_PROCESS_INFORMATION: u32 = 5;
pub const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: u32 = 8;
pub const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
pub const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC0000004u32 as i32;

#[repr(C)]
pub struct UnicodeString {
    pub len: u16,
    pub max: u16,
    pub buf: *const u16,
}

/// SYSTEM_PROCESS_INFORMATION (x64 layout; repr(C) inserts the same padding as the C header).
#[repr(C)]
pub struct Spi {
    pub next: u32,
    pub threads: u32,
    pub ws_private: i64,
    pub hard_faults: u32,
    pub threads_hwm: u32,
    pub cycle_time: u64,
    pub create_time: i64,
    pub user_time: i64,
    pub kernel_time: i64,
    pub image_name: UnicodeString,
    pub base_priority: i32,
    pub pid: usize,
    pub ppid: usize,
    pub handles: u32,
    pub session: u32,
    pub key: usize,
    pub peak_virtual: usize,
    pub virtual_size: usize,
    pub page_faults: u32,
    pub peak_ws: usize,
    pub ws: usize,
    pub qpp_peak: usize,
    pub qpp: usize,
    pub qnp_peak: usize,
    pub qnp: usize,
    pub pagefile: usize,
    pub pagefile_peak: usize,
    pub private_pages: usize,
    pub read_ops: i64,
    pub write_ops: i64,
    pub other_ops: i64,
    pub read_bytes: i64,
    pub write_bytes: i64,
    pub other_bytes: i64,
}

/// SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Sppi {
    pub idle: i64,
    pub kernel: i64,
    pub user: i64,
    pub dpc: i64,
    pub interrupt: i64,
    pub interrupt_count: u32,
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(p: &[u16]) -> String {
    let n = p.iter().position(|&c| c == 0).unwrap_or(p.len());
    String::from_utf16_lossy(&p[..n])
}

pub unsafe fn from_ustr(u: &UnicodeString) -> String {
    if u.buf.is_null() || u.len == 0 {
        return String::new();
    }
    let s = unsafe { std::slice::from_raw_parts(u.buf, (u.len / 2) as usize) };
    String::from_utf16_lossy(s)
}

/// Call NtQuerySystemInformation with a growing buffer.
pub fn query_system(class: u32, start: usize) -> Option<Vec<u8>> {
    let mut size = start;
    for _ in 0..8 {
        let mut buf: Vec<u8> = vec![0; size];
        let mut ret = 0u32;
        let st = unsafe { NtQuerySystemInformation(class, buf.as_mut_ptr() as *mut c_void, size as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH {
            size = (ret as usize).max(size * 2) + 64 * 1024;
            continue;
        }
        if st < 0 {
            return None;
        }
        buf.truncate(ret as usize);
        return Some(buf);
    }
    None
}

pub fn cpu_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// Is this process running with an admin token?
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).is_err() {
            return false;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut n = 0u32;
        let ok = GetTokenInformation(tok, TokenElevation, Some(&mut e as *mut _ as *mut c_void), size_of::<TOKEN_ELEVATION>() as u32, &mut n).is_ok();
        let _ = CloseHandle(tok);
        ok && e.TokenIsElevated != 0
    }
}
