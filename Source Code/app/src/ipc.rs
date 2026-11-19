//! Local control channels: a server listens on a named pipe, a client sends one line and reads one reply.
//! "taskman" = the open window (`wtm gui ...`), "history" = the optional sampler (`wtm sampler`).
use crate::sys::wide;
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::core::PCWSTR;

fn pipe(name: &str) -> String {
    format!(r"\\.\pipe\wintools-{name}")
}

/// Packaged (MSIX) parents, like the Claude desktop app, may only create pipes under LOCAL\.
fn pipe_local(name: &str) -> String {
    format!(r"\\.\pipe\LOCAL\wintools-{name}")
}

fn open(p: &str) -> windows::core::Result<HANDLE> {
    let name = wide(p);
    unsafe { CreateFileW(PCWSTR(name.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }
}

pub fn send_to(name: &str, line: &str) -> Result<String, String> {
    let h = open(&pipe(name)).or_else(|_| open(&pipe_local(name))).map_err(|_| format!("no '{name}' server is running"))?;
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

/// The open Task Manager window.
pub fn send(line: &str) -> Result<String, String> {
    send_to("taskman", line).map_err(|_| "the Rust Task Manager window is not running (start RustTaskManager.exe)".to_string())
}

fn create(p: &str) -> HANDLE {
    let name = wide(p);
    unsafe { CreateNamedPipeW(PCWSTR(name.as_ptr()), PIPE_ACCESS_DUPLEX, PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 4, 1 << 20, 65536, 0, None) }
}

/// Is a server already listening on this channel?
pub fn exists(name: &str) -> bool {
    let w1 = wide(&pipe(name));
    let w2 = wide(&pipe_local(name));
    unsafe { WaitNamedPipeW(PCWSTR(w1.as_ptr()), 1).as_bool() || WaitNamedPipeW(PCWSTR(w2.as_ptr()), 1).as_bool() }
}

/// Blocking server loop (run it on its own thread). `handle` gets one line and returns the reply.
pub fn serve_on(name: &str, handle: impl Fn(String) -> String) {
    let (p1, p2) = (pipe(name), pipe_local(name));
    let mut cur = p1.clone();
    loop {
        unsafe {
            let mut h = create(&cur);
            if h.is_invalid() && cur == p1 {
                cur = p2.clone();
                h = create(&cur);
            }
            if h.is_invalid() {
                crate::gui::app::trace(&format!("CreateNamedPipe {name} failed: {:?}", GetLastError()));
                return;
            }
            if ConnectNamedPipe(h, None).is_ok() || GetLastError() == ERROR_PIPE_CONNECTED {
                let mut buf = vec![0u8; 8192];
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

pub fn serve(handle: impl Fn(String) -> String) {
    serve_on("taskman", handle)
}
