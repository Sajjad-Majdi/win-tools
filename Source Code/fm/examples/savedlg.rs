//! Test helper only (not shipped). A windowless app that opens a standard Save dialog (GetSaveFileNameW) and writes
//! the chosen path to a result file. The tests start it on a hidden desktop, so nothing shows on screen.
//!   savedlg <result file> [initial dir]   -> shows the Save dialog, writes "OK <path>" or "CANCEL <code>"
//!   savedlg                               -> no window: writes a marker file ($WFM_TEST_MARKER) and exits
#![windows_subsystem = "windows"]
use windows::Win32::UI::Controls::Dialogs::{CommDlgExtendedError, GetSaveFileNameW, OFN_EXPLORER, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows::core::{PCWSTR, PWSTR};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        // Started by `wfm open` in the tests: prove we ran, show nothing.
        let marker = std::env::var("WFM_TEST_MARKER").unwrap_or_else(|_| std::env::current_exe().unwrap().with_file_name("savedlg-opened.txt").to_string_lossy().into_owned());
        let _ = std::fs::write(marker, "opened");
        return;
    }
    let result = &args[0];
    let init = args.get(1).cloned().unwrap_or_default();
    let mut buf = vec![0u16; 1024];
    let filter: Vec<u16> = "Text files\0*.txt\0All files\0*.*\0\0".encode_utf16().collect();
    let (init_w, title, defext) = (wide(&init), wide("Save As (wfm test)"), wide("txt"));
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrInitialDir: if init.is_empty() { PCWSTR::null() } else { PCWSTR(init_w.as_ptr()) },
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(defext.as_ptr()),
        Flags: OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    let ok = unsafe { GetSaveFileNameW(&mut ofn) }.as_bool();
    let out = if ok {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        format!("OK {}", String::from_utf16_lossy(&buf[..n]))
    } else {
        format!("CANCEL {}", unsafe { CommDlgExtendedError() }.0)
    };
    let _ = std::fs::write(result, out);
}
