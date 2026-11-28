//! Everything search through its window message IPC (WM_COPYDATA, QUERY2), without starting es.exe.
//! About 10-40 ms per query instead of 130-220 ms, so the GUI can search on every key and fetch all folder sizes
//! of a folder in one query. Runs on any thread: it makes a message-only reply window for the call.
use crate::util::{Res, wide};
use std::cell::RefCell;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const QUERY2W: usize = 18;
const REPLY_ID: u32 = 0x5746_4D31; // "WFM1"

pub const REQ_NAME: u32 = 0x1;
pub const REQ_PATH: u32 = 0x2;
pub const REQ_FULL: u32 = 0x4;
pub const REQ_EXT: u32 = 0x8;
pub const REQ_SIZE: u32 = 0x10;
pub const REQ_CREATED: u32 = 0x20;
pub const REQ_MODIFIED: u32 = 0x40;
pub const REQ_ACCESSED: u32 = 0x80;
pub const REQ_ATTRS: u32 = 0x100;

pub const SORT_NAME: u32 = 1;
pub const SORT_SIZE_ASC: u32 = 5;
pub const SORT_SIZE_DESC: u32 = 6;
pub const SORT_DATE_MODIFIED_DESC: u32 = 14;

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: u64,
    pub attrs: u32,
}

impl Item {
    pub fn full(&self) -> String {
        if self.path.is_empty() { self.name.clone() } else { format!("{}\\{}", self.path.trim_end_matches('\\'), self.name) }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub total: u32,
    pub items: Vec<Item>,
    pub ms: f64,
}

thread_local! {
    static GOT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static WND: RefCell<isize> = const { RefCell::new(0) };
}

unsafe extern "system" fn reply_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_COPYDATA {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == REPLY_ID as usize && !cds.lpData.is_null() {
                let b = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize).to_vec();
                GOT.with(|g| *g.borrow_mut() = Some(b));
                return LRESULT(1);
            }
        }
        DefWindowProcW(h, msg, wp, lp)
    }
}

fn reply_window() -> Option<HWND> {
    let cur = WND.with(|w| *w.borrow());
    if cur != 0 {
        return Some(HWND(cur as *mut _));
    }
    unsafe {
        let cls = w!("WinToolsFilesEvReply");
        let wc = WNDCLASSW { lpfnWndProc: Some(reply_proc), lpszClassName: cls, ..Default::default() };
        RegisterClassW(&wc);
        let h = CreateWindowExW(WINDOW_EX_STYLE(0), cls, w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None).ok()?;
        WND.with(|w| *w.borrow_mut() = h.0 as isize);
        Some(h)
    }
}

/// Is Everything running (its IPC window exists on this desktop, or on the Default desktop)?
pub fn available() -> bool {
    let here = unsafe { FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), PCWSTR::null()).is_ok() };
    here || on_default_desktop()
}

/// Window messages cannot cross desktops. On another desktop (a hidden test desktop, a kiosk desktop) Everything's
/// window lives on WinSta0\Default: there a windowless `wfm ev-raw` (next to this exe) asks for us.
fn on_default_desktop() -> bool {
    use windows::Win32::System::StationsAndDesktops::*;
    unsafe {
        let Ok(me) = GetThreadDesktop(windows::Win32::System::Threading::GetCurrentThreadId()) else { return false };
        let mut name = [0u16; 128];
        let mut need = 0u32;
        if GetUserObjectInformationW(HANDLE(me.0), UOI_NAME, Some(name.as_mut_ptr() as *mut _), 256, Some(&mut need)).is_err() {
            return false;
        }
        let n = String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)]);
        !n.eq_ignore_ascii_case("Default") && relay_exe().is_some()
    }
}

fn relay_exe() -> Option<std::path::PathBuf> {
    let p = std::env::current_exe().ok()?.with_file_name("wfm.exe");
    p.is_file().then_some(p)
}

/// Run `wfm ev-raw` on the Default desktop (CREATE_NO_WINDOW, nothing is shown) and read its reply bytes.
fn relay(search: &str, max: u32, request: u32, sort: u32, match_path: bool, timeout: Duration) -> Res<Reply> {
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::Storage::FileSystem::ReadFile;
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Threading::*;
    let exe = relay_exe().ok_or("Everything is not running")?;
    let t0 = Instant::now();
    unsafe {
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, bInheritHandle: true.into(), ..Default::default() };
        let (mut rd, mut wr) = (HANDLE::default(), HANDLE::default());
        CreatePipe(&mut rd, &mut wr, Some(&sa), 1 << 20).map_err(|e| e.message())?;
        let _ = SetHandleInformation(rd, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
        let q: String = search.encode_utf16().map(|u| format!("{u:04x}")).collect();
        let cmd = format!("\"{}\" ev-raw {q} {max} {request} {sort} {}", exe.display(), match_path as u8);
        let mut cw: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let mut desk = wide(r"WinSta0\Default");
        let si = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            lpDesktop: windows::core::PWSTR(desk.as_mut_ptr()),
            dwFlags: STARTF_USESTDHANDLES,
            hStdOutput: wr,
            hStdError: wr,
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        let ok = CreateProcessW(None, Some(windows::core::PWSTR(cw.as_mut_ptr())), None, None, true, CREATE_NO_WINDOW, None, PCWSTR::null(), &si, &mut pi);
        let _ = CloseHandle(wr);
        if let Err(e) = ok {
            let _ = CloseHandle(rd);
            return Err(e.message());
        }
        let _ = CloseHandle(pi.hThread);
        let mut out = vec![];
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let mut got = 0u32;
            if ReadFile(rd, Some(&mut buf), Some(&mut got), None).is_err() || got == 0 {
                break;
            }
            out.extend_from_slice(&buf[..got as usize]);
            if t0.elapsed() > timeout + Duration::from_secs(2) {
                break;
            }
        }
        let _ = CloseHandle(rd);
        let done = WaitForSingleObject(pi.hProcess, 2000) == WAIT_OBJECT_0;
        let mut code = 1u32;
        let _ = GetExitCodeProcess(pi.hProcess, &mut code);
        let _ = CloseHandle(pi.hProcess);
        // wfm prints "error: ..." (text, not a reply) and exits 1 when Everything failed: never parse that as a
        // reply (its letters read as a result count of a billion and the window died allocating for it)
        if !done || code != 0 {
            let text = String::from_utf8_lossy(&out[..out.len().min(200)]).trim().trim_start_matches("error: ").to_string();
            return Err(if text.is_empty() { "Everything did not answer".into() } else { text });
        }
        if out.len() < 20 {
            return Err("Everything did not answer".into());
        }
        let mut r = parse(&out).ok_or("bad reply from Everything")?;
        r.ms = t0.elapsed().as_secs_f64() * 1000.0;
        Ok(r)
    }
}

/// For `wfm ev-raw`: the raw QUERY2 reply bytes (the relay above parses them).
pub fn query_raw(search: &str, max: u32, request: u32, sort: u32, match_path: bool, timeout: Duration) -> Res<Vec<u8>> {
    raw(search, max, request, sort, match_path, timeout)
}

/// Run one query. `search` uses Everything syntax. `request` = REQ_* flags (name, path, size, modified and
/// attributes are parsed). Errors when Everything is not running or does not answer within `timeout`.
pub fn query(search: &str, max: u32, request: u32, sort: u32, match_path: bool, timeout: Duration) -> Res<Reply> {
    let here = unsafe { FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), PCWSTR::null()).is_ok() };
    if !here {
        if on_default_desktop() {
            return relay(search, max, request, sort, match_path, timeout);
        }
        return Err("Everything is not running".into());
    }
    let t0 = Instant::now();
    let data = raw(search, max, request, sort, match_path, timeout)?;
    let mut r = parse(&data).ok_or("bad reply from Everything")?;
    r.ms = t0.elapsed().as_secs_f64() * 1000.0;
    Ok(r)
}

/// The total size of every result of `search` (files only make sense), read straight from Everything's reply
/// without building a result list (a folder of 300 000 files would otherwise allocate tens of MB for a moment).
/// None when the index does not know a size or the reply was cut at `max`.
pub fn size_sum(search: &str, max: u32, timeout: Duration) -> Res<Option<(u32, u64)>> {
    let here = unsafe { FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), PCWSTR::null()).is_ok() };
    if !here {
        let r = query(search, max, REQ_SIZE, 0, false, timeout)?;
        if (r.total as usize) > r.items.len() || r.items.iter().any(|i| i.size.is_none()) {
            return Ok(None);
        }
        return Ok(Some((r.total, r.items.iter().map(|i| i.size.unwrap_or(0)).sum())));
    }
    let b = raw(search, max, REQ_SIZE, 0, false, timeout)?;
    let (Some(total), Some(num), Some(req)) = (u32_at(&b, 0), u32_at(&b, 4), u32_at(&b, 12)) else { return Err("bad reply from Everything".into()) };
    if req & REQ_SIZE == 0 || total > num {
        return Ok(None);
    }
    let mut sum = 0u64;
    for i in 0..num as usize {
        let Some(o) = u32_at(&b, 20 + i * 8 + 4) else { return Err("bad reply from Everything".into()) };
        match u64_at(&b, o as usize) {
            Some(u64::MAX) | None => return Ok(None),
            Some(v) => sum += v,
        }
    }
    Ok(Some((total, sum)))
}

fn raw(search: &str, max: u32, request: u32, sort: u32, match_path: bool, timeout: Duration) -> Res<Vec<u8>> {
    let t0 = Instant::now();
    unsafe {
        let ev = FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), PCWSTR::null()).map_err(|_| "Everything is not running".to_string())?;
        let me = reply_window().ok_or("cannot create the reply window")?;
        GOT.with(|g| *g.borrow_mut() = None);
        let ws = wide(search);
        let mut buf: Vec<u8> = Vec::with_capacity(28 + ws.len() * 2);
        for v in [me.0 as usize as u32, REPLY_ID, if match_path { 4 } else { 0 }, 0, max, request, sort] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for c in &ws {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        let cds = COPYDATASTRUCT { dwData: QUERY2W, cbData: buf.len() as u32, lpData: buf.as_ptr() as *mut _ };
        let mut res = 0usize;
        let ok = SendMessageTimeoutW(ev, WM_COPYDATA, WPARAM(me.0 as usize), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG | SMTO_BLOCK, timeout.as_millis() as u32, Some(&mut res));
        if ok.0 == 0 || res == 0 {
            return Err("Everything did not take the query".into());
        }
        // the reply arrives as a sent WM_COPYDATA: pump until it is here
        let deadline = t0 + timeout;
        let mut msg = MSG::default();
        loop {
            if GOT.with(|g| g.borrow().is_some()) {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err("Everything did not answer in time".into());
            }
            let left = (deadline - now).as_millis() as u32;
            let _ = windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjects(None, false, left.min(50), QS_ALLINPUT);
            while PeekMessageW(&mut msg, Some(me), 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(GOT.with(|g| g.borrow_mut().take()).unwrap_or_default())
    }
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

fn wstr_at(b: &[u8], o: usize) -> Option<(String, usize)> {
    let len = u32_at(b, o)? as usize;
    let s = b.get(o + 4..o + 4 + len * 2)?;
    let w: Vec<u16> = s.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((String::from_utf16_lossy(&w), o + 4 + (len + 1) * 2))
}

fn parse(b: &[u8]) -> Option<Reply> {
    let total = u32_at(b, 0)?;
    let num = u32_at(b, 4)? as usize;
    let req = u32_at(b, 12)?;
    // every result has an 8-byte entry after the 20-byte header: a count the reply cannot hold is not a reply
    if num.checked_mul(8).and_then(|n| n.checked_add(20)).is_none_or(|n| n > b.len()) {
        return None;
    }
    let mut items = Vec::with_capacity(num);
    for i in 0..num {
        let base = 20 + i * 8;
        let flags = u32_at(b, base)?;
        let mut o = u32_at(b, base + 4)? as usize;
        let mut it = Item { is_dir: flags & 3 != 0, ..Default::default() };
        if req & REQ_NAME != 0 {
            let (s, n) = wstr_at(b, o)?;
            it.name = s;
            o = n;
        }
        if req & REQ_PATH != 0 {
            let (s, n) = wstr_at(b, o)?;
            it.path = s;
            o = n;
        }
        if req & REQ_FULL != 0 {
            let (s, n) = wstr_at(b, o)?;
            if it.name.is_empty() {
                match s.rfind('\\') {
                    Some(k) => {
                        it.path = s[..k].to_string();
                        it.name = s[k + 1..].to_string();
                    }
                    None => it.name = s,
                }
            }
            o = n;
        }
        if req & REQ_EXT != 0 {
            let (_, n) = wstr_at(b, o)?;
            o = n;
        }
        if req & REQ_SIZE != 0 {
            let v = u64_at(b, o)?;
            it.size = if v == u64::MAX { None } else { Some(v) };
            o += 8;
        }
        if req & REQ_CREATED != 0 {
            o += 8;
        }
        if req & REQ_MODIFIED != 0 {
            let v = u64_at(b, o)?;
            it.modified = if v == u64::MAX { 0 } else { v };
            o += 8;
        }
        if req & REQ_ACCESSED != 0 {
            o += 8;
        }
        if req & REQ_ATTRS != 0 {
            it.attrs = u32_at(b, o)?;
        }
        items.push(it);
    }
    Some(Reply { total, items, ms: 0.0 })
}

/// Quote a path for Everything's search syntax.
pub fn quote(p: &str) -> String {
    format!("\"{}\"", p.replace('"', ""))
}

/// Sizes of the sub-folders of `dir` from the index, in one query: (folder name, bytes).
pub fn folder_sizes(dir: &str, timeout: Duration) -> Res<Vec<(String, u64)>> {
    let d = dir.trim_end_matches('\\');
    let q = format!("parent:{} folder:", quote(&format!("{d}\\")));
    let r = query(&q, 100_000, REQ_NAME | REQ_SIZE, SORT_NAME, false, timeout)?;
    Ok(r.items.into_iter().filter_map(|i| i.size.map(|s| (i.name, s))).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn text_is_not_a_reply() {
        // wfm's error text through the relay once read as 1 159 740 018 results ("r: E") and aborted the window
        assert!(super::parse(b"error: Everything did not answer in time\r\n").is_none());
        // a real empty reply: total 0, num 0, offset, request, sort
        let mut b = vec![];
        for v in [0u32, 0, 0, super::REQ_NAME, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(super::parse(&b).map(|r| r.items.len()), Some(0));
        // a count bigger than the bytes that follow
        b[4] = 3;
        assert!(super::parse(&b).is_none());
    }
}
