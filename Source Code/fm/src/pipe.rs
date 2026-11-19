//! Local control channel of the file manager window: `RustFileExplorer.exe` serves `\\.\pipe\wintools-files`, `wfm gui ...`
//! sends one line and reads one reply. Packaged (MSIX) parents may only create pipes under `LOCAL\`, so both
//! names are tried.
use crate::util::wide;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::core::PCWSTR;

pub const NAME: &str = "wintools-files";

/// The pipe name in use: `WFM_PIPE` when set (test windows use their own pipe so they never meet the real one).
pub fn name() -> String {
    std::env::var("WFM_PIPE").ok().filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or_else(|| NAME.to_string())
}

fn pipe(name: &str) -> String {
    format!(r"\\.\pipe\{name}")
}

fn pipe_local(name: &str) -> String {
    format!(r"\\.\pipe\LOCAL\{name}")
}

fn open(p: &str) -> windows::core::Result<HANDLE> {
    let name = wide(p);
    unsafe { CreateFileW(PCWSTR(name.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }
}

/// Send one line to the window, return its reply (JSON text).
pub fn send(line: &str) -> Result<String, String> {
    let mut h = Err(windows::core::Error::empty());
    // the server re-creates its pipe instance between clients: retry for a short while
    for _ in 0..40 {
        h = open(&pipe(&name())).or_else(|_| open(&pipe_local(&name())));
        if h.is_ok() {
            break;
        }
        let w1 = wide(&pipe(&name()));
        if !unsafe { WaitNamedPipeW(PCWSTR(w1.as_ptr()), 50) }.as_bool() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    let h = h.map_err(|_| "the Rust File Explorer window is not running (start RustFileExplorer.exe)".to_string())?;
    unsafe {
        let mut n = 0u32;
        if WriteFile(h, Some(line.as_bytes()), Some(&mut n), None).is_err() {
            let _ = CloseHandle(h);
            return Err("write failed".into());
        }
        let mut out = vec![];
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut got = 0u32;
            let r = ReadFile(h, Some(&mut buf), Some(&mut got), None);
            out.extend_from_slice(&buf[..got as usize]);
            match r {
                Ok(()) => break,
                Err(e) if e.code().0 as u32 == 0x800700EA => continue, // ERROR_MORE_DATA
                Err(e) => {
                    let _ = CloseHandle(h);
                    return Err(e.message());
                }
            }
        }
        let _ = CloseHandle(h);
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

/// Is a window already serving the pipe?
pub fn exists() -> bool {
    let w1 = wide(&pipe(&name()));
    let w2 = wide(&pipe_local(&name()));
    unsafe { WaitNamedPipeW(PCWSTR(w1.as_ptr()), 1).as_bool() || WaitNamedPipeW(PCWSTR(w2.as_ptr()), 1).as_bool() }
}

fn create(p: &str) -> HANDLE {
    let name = wide(p);
    unsafe { CreateNamedPipeW(PCWSTR(name.as_ptr()), PIPE_ACCESS_DUPLEX, PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 4, 1 << 20, 65536, 0, None) }
}

/// Blocking server loop (run it on its own thread). `handle` gets one line and returns the reply.
pub fn serve(handle: impl Fn(String) -> String) {
    let (p1, p2) = (pipe(&name()), pipe_local(&name()));
    let mut cur = p1.clone();
    loop {
        unsafe {
            let mut h = create(&cur);
            if h.is_invalid() && cur == p1 {
                cur = p2.clone();
                h = create(&cur);
            }
            if h.is_invalid() {
                let _ = GetLastError();
                return;
            }
            if ConnectNamedPipe(h, None).is_ok() || GetLastError() == ERROR_PIPE_CONNECTED {
                let mut buf = vec![0u8; 65536];
                let mut got = 0u32;
                if ReadFile(h, Some(&mut buf), Some(&mut got), None).is_ok() {
                    let reply = handle(String::from_utf8_lossy(&buf[..got as usize]).into_owned());
                    let mut n = 0u32;
                    let _ = WriteFile(h, Some(reply.as_bytes()), Some(&mut n), None);
                    let _ = FlushFileBuffers(h);
                }
                let _ = DisconnectNamedPipe(h);
            }
            let _ = CloseHandle(h);
        }
    }
}
