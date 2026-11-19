//! RustFileExplorerWinE.exe: makes Win+E open Rust File Explorer.
//!
//! Windows 11 builds from November 2026 (feature 63202390) launch Explorer on Win+E directly and no longer read the
//! per-user `opennewwindow` verb, so the registry way stopped working for every file manager. This tiny resident
//! process (no window, a few MB) watches only the E key with a low-level keyboard hook: plain Win+E is swallowed and
//! RustFileExplorer.exe (next to this file) opens instead. Every other key passes through untouched.
//!
//! Started at sign-in from HKCU\...\Run (written by "Set as default"; "Restore Explorer" removes it and stops it).
//! `--quit` stops a running copy. Only one copy runs.
#![windows_subsystem = "windows"]
#![allow(non_snake_case)]
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const CLASS: PCWSTR = w!("RustFileExplorerWinE");
const WM_LAUNCH: u32 = WM_APP + 1;
const VK_E: u32 = 0x45;
static HWND_MAIN: AtomicIsize = AtomicIsize::new(0);
/// the E key-down was swallowed: its repeats and key-up are swallowed too
static HELD: AtomicBool = AtomicBool::new(false);

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        if k.vkCode == VK_E && k.flags.0 & LLKHF_INJECTED.0 == 0 {
            let msg = wp.0 as u32;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                // plain Win+E only (Win+Shift+E and friends stay Windows' own)
                if (down(VK_LWIN) || down(VK_RWIN)) && !down(VK_SHIFT) && !down(VK_CONTROL) && !down(VK_MENU) {
                    if !HELD.swap(true, Ordering::SeqCst) {
                        // the work runs on the message loop: the hook must return at once
                        let h = HWND(HWND_MAIN.load(Ordering::SeqCst) as *mut _);
                        let _ = unsafe { PostMessageW(Some(h), WM_LAUNCH, WPARAM(0), LPARAM(0)) };
                    }
                    return LRESULT(1);
                }
            } else if HELD.swap(false, Ordering::SeqCst) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// A key nobody uses (0xE8) between Win down and Win up, so letting go of Win does not open Start.
fn mask_win() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0xE8), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn launch() {
    let Some(exe) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("RustFileExplorer.exe"))) else { return };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let e = wide(&exe.to_string_lossy());
    let d = wide(&exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    unsafe {
        // the new window may take the foreground (this process just had the key)
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        ShellExecuteW(None, w!("open"), PCWSTR(e.as_ptr()), PCWSTR::null(), PCWSTR(d.as_ptr()), SW_SHOWNORMAL);
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_LAUNCH => {
            mask_win();
            launch();
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(h) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, m, wp, lp) },
    }
}

fn main() {
    unsafe {
        let running = FindWindowExW(Some(HWND_MESSAGE), None, CLASS, PCWSTR::null()).ok();
        if std::env::args().any(|a| a == "--quit") {
            if let Some(h) = running {
                let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return;
        }
        if running.is_some() {
            return;
        }
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: CLASS, ..Default::default() };
        RegisterClassW(&wc);
        let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), CLASS, CLASS, WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None) else { return };
        HWND_MAIN.store(h.0 as isize, Ordering::SeqCst);
        let Ok(hk) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), Some(inst.into()), 0) else { return };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hk);
    }
}
