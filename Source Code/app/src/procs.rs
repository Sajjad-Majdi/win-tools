//! Process list: one NtQuerySystemInformation call per refresh (all processes, CPU times, memory, I/O),
//! deltas for CPU % and I/O rate, plus a cache of slow per-process facts (path, description, command line).
use crate::sys::*;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::Instant;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::*;
use windows::core::{BOOL, PCWSTR, PWSTR};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    App,
    Background,
    Windows,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::App => "app",
            Kind::Background => "background",
            Kind::Windows => "windows",
        }
    }
}

#[derive(Default, Debug)]
pub struct Meta {
    pub path: String,
    pub description: String,
    pub company: String,
    pub desc_done: bool,
}

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub create_time: i64,
    pub threads: u32,
    pub handles: u32,
    pub session: u32,
    pub priority: i32,
    /// private working set (what Task Manager shows as "Memory")
    pub mem: u64,
    pub ws: u64,
    pub commit: u64,
    pub cpu_time: u64,
    pub io_bytes: u64,
    /// percent of the whole machine (0..100)
    pub cpu: f32,
    /// bytes per second (file and device reads + writes)
    pub io_rate: f64,
    pub meta: Arc<Meta>,
    pub title: String,
    pub kind: Kind,
}

pub struct Sampler {
    prev: HashMap<(u32, i64), (u64, u64)>,
    meta: HashMap<(u32, i64), Arc<Meta>>,
    ver: HashMap<String, (String, String)>,
    last: Option<Instant>,
    ncpu: f64,
    pub procs: Vec<Proc>,
    /// whole-machine CPU % from the same sample (100 - idle)
    pub total_cpu: f32,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        Sampler { prev: HashMap::new(), meta: HashMap::new(), ver: HashMap::new(), last: None, ncpu: cpu_count() as f64, procs: vec![], total_cpu: 0.0 }
    }

    /// Take a new sample. `with_meta` = also look up the exe path of new processes (~0.03 ms each).
    /// Descriptions come from `fill_desc`.
    pub fn refresh(&mut self, with_meta: bool) {
        let Some(buf) = query_system(SYSTEM_PROCESS_INFORMATION, 1 << 20) else { return };
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);
        // first sample: no delta yet, so show CPU averaged over each process's life instead of 0%
        let now_ft = crate::insight::now_filetime();
        let titles = window_titles();
        let mut out = Vec::with_capacity(400);
        let mut next_prev = HashMap::with_capacity(400);
        let mut off = 0usize;
        let mut idle_delta = 0u64;
        loop {
            let e = unsafe { &*(buf.as_ptr().add(off) as *const Spi) };
            let pid = e.pid as u32;
            let mut name = unsafe { from_ustr(&e.image_name) };
            if pid == 0 {
                name = "System Idle Process".into();
            }
            let cpu_time = (e.user_time + e.kernel_time) as u64;
            let io = (e.read_bytes + e.write_bytes) as u64;
            let key = (pid, e.create_time);
            let (cpu, io_rate) = match (self.prev.get(&key), dt > 0.05) {
                (Some(&(c0, i0)), true) => {
                    let dc = cpu_time.saturating_sub(c0);
                    if pid == 0 {
                        idle_delta = dc;
                    }
                    ((dc as f64 / (dt * 1e7 * self.ncpu) * 100.0) as f32, io.saturating_sub(i0) as f64 / dt)
                }
                _ if pid != 0 && e.create_time > 0 => {
                    let alive = (now_ft - e.create_time).max(1) as f64;
                    ((cpu_time as f64 / (alive * self.ncpu) * 100.0) as f32, 0.0)
                }
                _ => (0.0, 0.0),
            };
            next_prev.insert(key, (cpu_time, io));
            let meta = if let Some(m) = self.meta.get(&key) {
                m.clone()
            } else if with_meta {
                let m = Arc::new(self.lookup_meta(pid));
                self.meta.insert(key, m.clone());
                m
            } else {
                Arc::new(Meta::default())
            };
            let title = titles.get(&pid).cloned().unwrap_or_default();
            let kind = classify(pid, &name, &meta.path, !title.is_empty(), e.session);
            out.push(Proc {
                pid,
                ppid: e.ppid as u32,
                name,
                create_time: e.create_time,
                threads: e.threads,
                handles: e.handles,
                session: e.session,
                priority: e.base_priority,
                mem: e.ws_private as u64,
                ws: e.ws as u64,
                commit: e.pagefile as u64,
                cpu_time,
                io_bytes: io,
                cpu: cpu.clamp(0.0, 100.0),
                io_rate,
                meta,
                title,
                kind,
            });
            if e.next == 0 {
                break;
            }
            off += e.next as usize;
        }
        if dt > 0.05 {
            self.total_cpu = (100.0 - idle_delta as f64 / (dt * 1e7 * self.ncpu) * 100.0).clamp(0.0, 100.0) as f32;
        } else if let Some(idle) = out.iter().find(|p| p.pid == 0) {
            // first sample: average load since boot (idle time vs uptime)
            let up = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() } as f64 * 1e4;
            self.total_cpu = (100.0 - idle.cpu_time as f64 / (up * self.ncpu) * 100.0).clamp(0.0, 100.0) as f32;
        }
        self.meta.retain(|k, _| next_prev.contains_key(k));
        self.prev = next_prev;
        self.procs = out;
    }

    /// Add description/company (version resource, slow on a cold disk) to up to `budget` processes.
    /// Returns how many were filled; 0 = all done.
    pub fn fill_desc(&mut self, budget: usize) -> usize {
        let mut done = 0;
        for i in 0..self.procs.len() {
            if done >= budget {
                break;
            }
            let key = (self.procs[i].pid, self.procs[i].create_time);
            let Some(m) = self.meta.get(&key) else { continue };
            if m.desc_done || m.path.is_empty() {
                continue;
            }
            let (description, company) = match self.ver.get(&m.path) {
                Some(v) => v.clone(),
                None => {
                    let v = version_strings(&m.path);
                    self.ver.insert(m.path.clone(), v.clone());
                    v
                }
            };
            let nm = Arc::new(Meta { path: m.path.clone(), description, company, desc_done: true });
            self.meta.insert(key, nm.clone());
            self.procs[i].meta = nm;
            done += 1;
        }
        done
    }

    fn lookup_meta(&mut self, pid: u32) -> Meta {
        let path = image_path(pid).unwrap_or_default();
        match self.ver.get(&path) {
            Some((d, c)) => Meta { path, description: d.clone(), company: c.clone(), desc_done: true },
            None => Meta { path, ..Default::default() },
        }
    }
}

fn classify(pid: u32, name: &str, path: &str, has_window: bool, session: u32) -> Kind {
    if has_window && session != 0 && !path.is_empty() {
        let lp = path.to_ascii_lowercase();
        // explorer and shell hosts own windows but are Windows parts; still shown as apps like Task Manager does
        if !lp.starts_with(r"c:\windows\") || name.eq_ignore_ascii_case("explorer.exe") || name.eq_ignore_ascii_case("notepad.exe") {
            return Kind::App;
        }
    }
    // no path = we may not open it (other user, protected): those are Windows / system processes
    if pid <= 4 || path.is_empty() {
        return Kind::Windows;
    }
    if path.to_ascii_lowercase().starts_with(r"c:\windows\") {
        Kind::Windows
    } else {
        Kind::Background
    }
}

pub fn open_limited(pid: u32) -> Option<HANDLE> {
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok() }
}

pub fn image_path(pid: u32) -> Option<String> {
    if pid <= 4 {
        return None;
    }
    let h = open_limited(pid)?;
    let mut buf = [0u16; 1024];
    let mut n = buf.len() as u32;
    let r = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n) };
    unsafe {
        let _ = CloseHandle(h);
    }
    r.ok()?;
    Some(String::from_utf16_lossy(&buf[..n as usize]))
}

pub fn command_line(pid: u32) -> Option<String> {
    let h = open_limited(pid)?;
    let mut buf = vec![0u8; 8192];
    let mut ret = 0u32;
    let mut st = unsafe { NtQueryInformationProcess(h.0, PROCESS_COMMAND_LINE_INFORMATION, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut ret) };
    if st == STATUS_INFO_LENGTH_MISMATCH || (st < 0 && ret as usize > buf.len()) {
        buf = vec![0u8; ret as usize + 16];
        st = unsafe { NtQueryInformationProcess(h.0, PROCESS_COMMAND_LINE_INFORMATION, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut ret) };
    }
    unsafe {
        let _ = CloseHandle(h);
    }
    if st < 0 {
        return None;
    }
    let u = unsafe { &*(buf.as_ptr() as *const UnicodeString) };
    Some(unsafe { from_ustr(u) })
}

/// FileDescription and CompanyName from the exe's version resource.
pub fn version_strings(path: &str) -> (String, String) {
    use windows::Win32::Storage::FileSystem::*;
    let wp = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wp.as_ptr()), None);
        if size == 0 {
            return Default::default();
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(wp.as_ptr()), Some(0), size, data.as_mut_ptr() as *mut c_void).is_err() {
            return Default::default();
        }
        let mut p: *mut c_void = std::ptr::null_mut();
        let mut n = 0u32;
        let tr = wide(r"\VarFileInfo\Translation");
        let mut langs = vec![];
        if VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(tr.as_ptr()), &mut p, &mut n).as_bool() && n >= 4 {
            let s = std::slice::from_raw_parts(p as *const u16, (n / 2) as usize);
            for c in s.chunks(2) {
                if c.len() == 2 {
                    langs.push(format!("{:04x}{:04x}", c[0], c[1]));
                }
            }
        }
        langs.extend(["040904b0".to_string(), "040904e4".to_string(), "000004b0".to_string()]);
        let get = |field: &str| -> String {
            for l in &langs {
                let q = wide(&format!(r"\StringFileInfo\{l}\{field}"));
                let mut p: *mut c_void = std::ptr::null_mut();
                let mut n = 0u32;
                if VerQueryValueW(data.as_ptr() as *const c_void, PCWSTR(q.as_ptr()), &mut p, &mut n).as_bool() && n > 1 {
                    let s = std::slice::from_raw_parts(p as *const u16, n as usize);
                    let v = from_wide(s).trim().to_string();
                    if !v.is_empty() {
                        return v;
                    }
                }
            }
            String::new()
        };
        (get("FileDescription"), get("CompanyName"))
    }
}

/// pid -> title of its main visible, un-owned, un-cloaked top-level window.
pub fn window_titles() -> HashMap<u32, String> {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        unsafe {
            let map = &mut *(l.0 as *mut HashMap<u32, String>);
            if !IsWindowVisible(h).as_bool() || GetWindow(h, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
                return true.into();
            }
            if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                return true.into();
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, 4);
            if cloaked != 0 {
                return true.into();
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(h, &mut buf);
            if n <= 0 {
                return true.into();
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            map.entry(pid).or_insert_with(|| String::from_utf16_lossy(&buf[..n as usize]));
            true.into()
        }
    }
    let mut map: HashMap<u32, String> = HashMap::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut map as *mut _ as isize));
    }
    map
}

/// All descendants of `pid` (children whose start time is after the parent's, so reused PIDs are not followed).
pub fn descendants(procs: &[Proc], pid: u32) -> Vec<u32> {
    let mut out = vec![];
    let mut stack = vec![pid];
    let ct: HashMap<u32, i64> = procs.iter().map(|p| (p.pid, p.create_time)).collect();
    while let Some(cur) = stack.pop() {
        let pt = ct.get(&cur).copied().unwrap_or(0);
        for p in procs {
            if p.ppid == cur && p.pid != cur && p.pid != 0 && p.create_time >= pt && !out.contains(&p.pid) {
                out.push(p.pid);
                stack.push(p.pid);
            }
        }
    }
    out
}

pub fn terminate(pid: u32) -> Result<(), String> {
    if pid <= 4 {
        return Err("system process".into());
    }
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(|e| e.message())?;
        let r = TerminateProcess(h, 1).map_err(|e| e.message());
        let _ = CloseHandle(h);
        r
    }
}

/// Ask a process to close: WM_CLOSE to each of its visible top-level windows. Returns the number of windows asked.
pub fn close_windows(pid: u32) -> usize {
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        unsafe {
            let st = &mut *(l.0 as *mut (u32, usize));
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == st.0 && IsWindowVisible(h).as_bool() && PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() {
                st.1 += 1;
            }
            true.into()
        }
    }
    let mut st = (pid, 0usize);
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut st as *mut _ as isize));
    }
    st.1
}

pub fn set_priority(pid: u32, level: &str) -> Result<(), String> {
    let class = match level {
        "idle" | "low" => IDLE_PRIORITY_CLASS,
        "below" | "belownormal" => BELOW_NORMAL_PRIORITY_CLASS,
        "normal" => NORMAL_PRIORITY_CLASS,
        "above" | "abovenormal" => ABOVE_NORMAL_PRIORITY_CLASS,
        "high" => HIGH_PRIORITY_CLASS,
        "realtime" => REALTIME_PRIORITY_CLASS,
        _ => return Err(format!("unknown priority '{level}' (idle|below|normal|above|high|realtime)")),
    };
    unsafe {
        let h = OpenProcess(PROCESS_SET_INFORMATION, false, pid).map_err(|e| e.message())?;
        let r = SetPriorityClass(h, class).map_err(|e| e.message());
        let _ = CloseHandle(h);
        r
    }
}

/// Lower-case haystack match: every space-separated token must appear in some field.
/// Returns a rank (lower = better) or None. Fields: name, title, description, company, pid, path, extra (cmdline).
pub fn match_rank(p: &Proc, tokens: &[String], cmdline: Option<&str>) -> Option<u32> {
    if tokens.is_empty() {
        return Some(100);
    }
    let name = p.name.to_lowercase();
    let fields = [
        name.clone(),
        p.title.to_lowercase(),
        p.meta.description.to_lowercase(),
        p.meta.company.to_lowercase(),
        p.pid.to_string(),
        p.meta.path.to_lowercase(),
        cmdline.map(|c| c.to_lowercase()).unwrap_or_default(),
    ];
    let mut rank = 0u32;
    for t in tokens {
        let pos = fields.iter().position(|f| f.contains(t.as_str()))?;
        let bonus = if pos == 0 && name.starts_with(t.as_str()) { 0 } else { 1 + pos as u32 * 2 };
        if pos == 4 && fields[4] != *t {
            // pid must match exactly or as prefix, not anywhere
            if !fields[4].starts_with(t.as_str()) {
                return None;
            }
        }
        rank += bonus;
    }
    Some(rank)
}

pub fn tokens(q: &str) -> Vec<String> {
    q.split_whitespace().map(|t| t.to_lowercase()).collect()
}

pub fn fmt_bytes(b: f64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{v:.0} {}", U[i]) } else if v < 10.0 { format!("{v:.1} {}", U[i]) } else { format!("{v:.0} {}", U[i]) }
}
