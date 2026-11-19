//! Whole-machine performance: CPU (total + per core), memory, disks, network adapters, GPU.
//! CPU/memory/network use plain Win32 calls; disks use IOCTL_DISK_PERFORMANCE; GPU uses PDH (lazy, only when asked).
use crate::sys::*;
use std::collections::HashMap;
use std::ffi::c_void;
use std::time::Instant;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use windows::Win32::System::Performance::*;
use windows::Win32::System::SystemInformation::*;
use windows::core::PCWSTR;

#[derive(Clone, Default, Debug)]
pub struct Disk {
    pub name: String,
    /// drive letters on this disk ("C: D:")
    pub letters: String,
    /// size and free space of its volumes (bytes), SSD or not (None = unknown), model name
    pub total: u64,
    pub free: u64,
    pub ssd: Option<bool>,
    pub model: String,
    pub read_rate: f64,
    pub write_rate: f64,
    /// percent of the interval the disk was busy
    pub active: f32,
}

#[derive(Clone, Default, Debug)]
pub struct Net {
    pub name: String,
    pub desc: String,
    pub recv_rate: f64,
    pub send_rate: f64,
    pub link_bps: u64,
    pub ipv4: String,
}

/// One graphics adapter (from DXGI) with its own engines and memory.
#[derive(Clone, Default, Debug)]
pub struct Adapter {
    pub name: String,
    /// busiest engine type, %
    pub util: f32,
    /// the main engines (3D, Compute, Copy, Video ...), busiest first
    pub engines: Vec<(String, f32)>,
    /// the other engines (Security, True Audio ...)
    pub other: Vec<(String, f32)>,
    pub dedicated_used: u64,
    pub dedicated_total: u64,
    pub shared_used: u64,
    pub shared_total: u64,
    pub driver: String,
}

#[derive(Clone, Default, Debug)]
pub struct Gpu {
    /// every adapter that has engines, busiest first
    pub adapters: Vec<Adapter>,
    /// busiest engine type, percent (what Task Manager shows)
    pub util: f32,
    pub engines: Vec<(String, f32)>,
    pub dedicated_used: u64,
    pub shared_used: u64,
    /// pid -> percent
    pub per_pid: HashMap<u32, f32>,
}

#[derive(Clone, Default, Debug)]
pub struct Snapshot {
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub cpu_name: String,
    pub mem_total: u64,
    pub mem_avail: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub cached: u64,
    pub processes: u32,
    pub threads: u32,
    pub handles: u32,
    pub uptime_s: u64,
    /// base speed (MHz, from the CPU) and the current speed (MHz; 0 until the counter has a value)
    pub base_mhz: u32,
    pub cur_mhz: u32,
    pub phys_cores: u32,
    /// kernel memory pools (bytes)
    pub paged_pool: u64,
    pub nonpaged_pool: u64,
    pub disks: Vec<Disk>,
    pub nets: Vec<Net>,
    pub gpu: Option<Gpu>,
}

pub struct Perf {
    last: Option<Instant>,
    cores_prev: Vec<Sppi>,
    disks_prev: HashMap<u32, (i64, i64, i64, i64)>, // read bytes, write bytes, idle time, query time
    nets_prev: HashMap<u64, (u64, u64)>,
    gpu: Option<GpuQuery>,
    cpu_name: String,
    base_mhz: u32,
    phys_cores: u32,
    letters: HashMap<u32, String>,
    disk_info: HashMap<u32, (u64, u64, Option<bool>, String)>,
    ipv4: HashMap<u64, String>,
    ticks: u64,
    pub snap: Snapshot,
}

impl Default for Perf {
    fn default() -> Self {
        Self::new()
    }
}

impl Perf {
    pub fn new() -> Self {
        let base_mhz = crate::reg::read_dword(crate::reg::HKLM, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "~MHz").unwrap_or(0);
        Perf { last: None, cores_prev: vec![], disks_prev: HashMap::new(), nets_prev: HashMap::new(), gpu: None, cpu_name: cpu_name(), base_mhz, phys_cores: physical_cores(), letters: HashMap::new(), disk_info: HashMap::new(), ipv4: HashMap::new(), ticks: 0, snap: Snapshot::default() }
    }

    /// `gpu`: also sample GPU counters (opens a PDH query on first use; ~30-60 ms the first time).
    pub fn refresh(&mut self, gpu: bool) {
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);
        let mut s = Snapshot { cpu_name: self.cpu_name.clone(), base_mhz: self.base_mhz, phys_cores: self.phys_cores, ..Default::default() };
        // drive letters per disk: read now and then (drives rarely change)
        if self.ticks % 30 == 0 {
            self.letters = drive_letters();
            self.disk_info = disk_info(&self.letters);
            self.ipv4 = ipv4_by_luid();
        }
        self.ticks += 1;

        // CPU per core
        let n = cpu_count();
        if let Some(buf) = query_system(SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION, n * size_of::<Sppi>()) {
            let cur: Vec<Sppi> = buf.chunks_exact(size_of::<Sppi>()).map(|c| unsafe { std::ptr::read_unaligned(c.as_ptr() as *const Sppi) }).collect();
            if self.cores_prev.len() == cur.len() {
                let (mut ti, mut tt) = (0i64, 0i64);
                for (a, b) in self.cores_prev.iter().zip(&cur) {
                    let idle = b.idle - a.idle;
                    let total = (b.kernel - a.kernel) + (b.user - a.user);
                    ti += idle;
                    tt += total;
                    s.cores.push(if total > 0 { (100.0 * (1.0 - idle as f64 / total as f64)).clamp(0.0, 100.0) as f32 } else { 0.0 });
                }
                s.cpu = if tt > 0 { (100.0 * (1.0 - ti as f64 / tt as f64)).clamp(0.0, 100.0) as f32 } else { 0.0 };
            } else {
                s.cores = vec![0.0; cur.len()];
            }
            self.cores_prev = cur;
        }

        // memory
        unsafe {
            let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if GlobalMemoryStatusEx(&mut m).is_ok() {
                s.mem_total = m.ullTotalPhys;
                s.mem_avail = m.ullAvailPhys;
            }
            let mut pi = windows::Win32::System::ProcessStatus::PERFORMANCE_INFORMATION { cb: size_of::<windows::Win32::System::ProcessStatus::PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
            if windows::Win32::System::ProcessStatus::GetPerformanceInfo(&mut pi, pi.cb).is_ok() {
                let pg = pi.PageSize as u64;
                s.commit_total = pi.CommitTotal as u64 * pg;
                s.commit_limit = pi.CommitLimit as u64 * pg;
                s.cached = pi.SystemCache as u64 * pg;
                s.paged_pool = pi.KernelPaged as u64 * pg;
                s.nonpaged_pool = pi.KernelNonpaged as u64 * pg;
                s.processes = pi.ProcessCount;
                s.threads = pi.ThreadCount;
                s.handles = pi.HandleCount;
            }
            s.uptime_s = GetTickCount64() / 1000;
        }

        // disks
        for i in 0..16u32 {
            let Some(dp) = disk_perf(i) else { continue };
            let cur = (dp.BytesRead, dp.BytesWritten, dp.IdleTime, dp.QueryTime);
            let mut d = Disk { name: format!("Disk {i}"), letters: self.letters.get(&i).cloned().unwrap_or_default(), ..Default::default() };
            if let Some((total, free, ssd, model)) = self.disk_info.get(&i) {
                (d.total, d.free, d.ssd, d.model) = (*total, *free, *ssd, model.clone());
            }
            if let (Some(p), true) = (self.disks_prev.get(&i), dt > 0.0) {
                d.read_rate = (cur.0 - p.0).max(0) as f64 / dt;
                d.write_rate = (cur.1 - p.1).max(0) as f64 / dt;
                let span = (cur.3 - p.3).max(1) as f64;
                d.active = (100.0 * (1.0 - (cur.2 - p.2) as f64 / span)).clamp(0.0, 100.0) as f32;
            }
            self.disks_prev.insert(i, cur);
            s.disks.push(d);
        }

        // network adapters
        unsafe {
            let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
            if GetIfTable2(&mut table).is_ok() {
                let t = &*table;
                let rows = std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize);
                for r in rows {
                    let hw = r.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
                    if !hw || r.OperStatus.0 != 1 || !(r.Type == 6 || r.Type == 71) {
                        continue;
                    }
                    let id = r.InterfaceLuid.Value;
                    let cur = (r.InOctets, r.OutOctets);
                    let mut n = Net { name: from_wide(&r.Alias), desc: from_wide(&r.Description), link_bps: r.ReceiveLinkSpeed.max(r.TransmitLinkSpeed), ipv4: self.ipv4.get(&id).cloned().unwrap_or_default(), ..Default::default() };
                    if let (Some(p), true) = (self.nets_prev.get(&id), dt > 0.0) {
                        n.recv_rate = cur.0.saturating_sub(p.0) as f64 / dt;
                        n.send_rate = cur.1.saturating_sub(p.1) as f64 / dt;
                    }
                    self.nets_prev.insert(id, cur);
                    s.nets.push(n);
                }
                FreeMibTable(table as *const c_void);
            }
        }

        if gpu {
            if self.gpu.is_none() {
                self.gpu = GpuQuery::open();
            }
            if let Some(g) = &mut self.gpu {
                s.gpu = g.read();
                if g.perf_pct > 0.0 && self.base_mhz > 0 {
                    s.cur_mhz = (self.base_mhz as f64 * g.perf_pct / 100.0).round() as u32;
                }
            }
        }
        self.snap = s;
    }
}

fn disk_perf(i: u32) -> Option<DISK_PERFORMANCE> {
    let path = wide(&format!(r"\\.\PhysicalDrive{i}"));
    unsafe {
        let h: HANDLE = CreateFileW(PCWSTR(path.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None).ok()?;
        let mut dp = DISK_PERFORMANCE::default();
        let mut n = 0u32;
        let r = DeviceIoControl(h, IOCTL_DISK_PERFORMANCE, None, 0, Some(&mut dp as *mut _ as *mut c_void), size_of::<DISK_PERFORMANCE>() as u32, Some(&mut n), None);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(dp)
    }
}

/// Physical CPU cores (logical processors are `cores.len()`).
fn physical_cores() -> u32 {
    unsafe {
        let mut len = 0u32;
        let _ = GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len);
        if len == 0 {
            return 0;
        }
        let mut buf = vec![0u8; len as usize];
        if GetLogicalProcessorInformationEx(RelationProcessorCore, Some(buf.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX), &mut len).is_err() {
            return 0;
        }
        let (mut off, mut n) = (0usize, 0u32);
        while off + 8 <= len as usize {
            let size = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap()) as usize;
            if size == 0 {
                break;
            }
            n += 1;
            off += size;
        }
        n
    }
}

/// Disk number -> its drive letters ("C: D:"), from each volume's storage device number.
fn drive_letters() -> HashMap<u32, String> {
    use windows::Win32::System::Ioctl::{IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
    let mut out: HashMap<u32, String> = HashMap::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let path = wide(&format!(r"\\.\{letter}:"));
        unsafe {
            let Ok(h) = CreateFileW(PCWSTR(path.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) else { continue };
            let mut dn = STORAGE_DEVICE_NUMBER::default();
            let mut n = 0u32;
            let r = DeviceIoControl(h, IOCTL_STORAGE_GET_DEVICE_NUMBER, None, 0, Some(&mut dn as *mut _ as *mut c_void), size_of::<STORAGE_DEVICE_NUMBER>() as u32, Some(&mut n), None);
            let _ = CloseHandle(h);
            if r.is_ok() {
                let e = out.entry(dn.DeviceNumber).or_default();
                if !e.is_empty() {
                    e.push(' ');
                }
                e.push(letter);
                e.push(':');
            }
        }
    }
    out
}

/// Per disk number: size and free space of its volumes, SSD (no seek penalty) or HDD, model.
fn disk_info(letters: &HashMap<u32, String>) -> HashMap<u32, (u64, u64, Option<bool>, String)> {
    use windows::Win32::System::Ioctl::*;
    let mut out = HashMap::new();
    for (&n, ls) in letters {
        let (mut total, mut free) = (0u64, 0u64);
        for l in ls.split(' ').filter(|x| !x.is_empty()) {
            let root = wide(&format!("{l}\\"));
            let (mut fa, mut tot, mut fr) = (0u64, 0u64, 0u64);
            if unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut fa), Some(&mut tot), Some(&mut fr)) }.is_ok() {
                total += tot;
                free += fr;
            }
        }
        let path = wide(&format!(r"\\.\PhysicalDrive{n}"));
        let (mut ssd, mut model) = (None, String::new());
        unsafe {
            if let Ok(h) = CreateFileW(PCWSTR(path.as_ptr()), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) {
                let mut q = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceSeekPenaltyProperty, QueryType: PropertyStandardQuery, ..Default::default() };
                let mut sp = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
                let mut got = 0u32;
                if DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(&q as *const _ as *const c_void), size_of::<STORAGE_PROPERTY_QUERY>() as u32, Some(&mut sp as *mut _ as *mut c_void), size_of::<DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32, Some(&mut got), None).is_ok() {
                    ssd = Some(!sp.IncursSeekPenalty);
                }
                q.PropertyId = StorageDeviceProperty;
                let mut buf = vec![0u8; 1024];
                if DeviceIoControl(h, IOCTL_STORAGE_QUERY_PROPERTY, Some(&q as *const _ as *const c_void), size_of::<STORAGE_PROPERTY_QUERY>() as u32, Some(buf.as_mut_ptr() as *mut c_void), buf.len() as u32, Some(&mut got), None).is_ok() {
                    let d = &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR);
                    let off = d.ProductIdOffset as usize;
                    if off > 0 && off < buf.len() {
                        let end = buf[off..].iter().position(|b| *b == 0).map(|e| off + e).unwrap_or(off);
                        model = String::from_utf8_lossy(&buf[off..end]).trim().to_string();
                    }
                }
                let _ = CloseHandle(h);
            }
        }
        out.insert(n, (total, free, ssd, model));
    }
    out
}

/// IPv4 address per network interface (by its LUID).
fn ipv4_by_luid() -> HashMap<u64, String> {
    use windows::Win32::Networking::WinSock::AF_INET;
    let mut out = HashMap::new();
    unsafe {
        let mut t: *mut MIB_UNICASTIPADDRESS_TABLE = std::ptr::null_mut();
        if GetUnicastIpAddressTable(AF_INET, &mut t).is_ok() && !t.is_null() {
            let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
            for r in rows {
                let a = r.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes();
                out.entry(r.InterfaceLuid.Value).or_insert_with(|| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]));
            }
            FreeMibTable(t as *const c_void);
        }
    }
    out
}

/// The display driver version of an adapter (from the display class keys, matched by name).
fn driver_version(name: &str) -> String {
    for i in 0..8 {
        let key = format!(r"SYSTEM\CurrentControlSet\Control\Class\{{4d36e968-e325-11ce-bfc1-08002be10318}}\{i:04}");
        if crate::reg::read_string(crate::reg::HKLM, &key, "DriverDesc").map(|d| d.trim() == name).unwrap_or(false) {
            return crate::reg::read_string(crate::reg::HKLM, &key, "DriverVersion").unwrap_or_default();
        }
    }
    String::new()
}

/// The graphics adapters DXGI knows (hardware only): luid key as the GPU counters write it, name, VRAM, shared.
fn dxgi_adapters() -> Vec<(String, String, u64, u64, String)> {
    use windows::Win32::Graphics::Dxgi::*;
    let mut v = vec![];
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return v };
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & 2 != 0 {
                continue; // software adapter (Microsoft Basic Render Driver)
            }
            let name = String::from_utf16_lossy(&d.Description).trim_end_matches('\0').trim().to_string();
            let key = format!("luid_0x{:08x}_0x{:08x}", d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart);
            if !v.iter().any(|x: &(String, String, u64, u64, String)| x.0 == key) {
                let drv = driver_version(&name);
                v.push((key, name, d.DedicatedVideoMemory as u64, d.SharedSystemMemory as u64, drv));
            }
        }
    }
    v
}

/// Engine types shown in the main view; the rest (Security, True Audio, ...) are listed small.
fn main_engine(ty: &str) -> bool {
    let t = ty.to_lowercase();
    ["3d", "compute", "copy", "video", "graphics", "cuda"].iter().any(|k| t.starts_with(k))
}

/// "VideoDecode" / "Compute_1" / "3D" -> "Video decode" / "Compute 1" / "3D"
fn engine_label(ty: &str) -> String {
    let t = ty.replace('_', " ");
    let mut out = String::new();
    for (i, ch) in t.chars().enumerate() {
        if i > 0 && ch.is_uppercase() && out.chars().last().map(|p| p.is_lowercase()).unwrap_or(false) {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn cpu_name() -> String {
    crate::reg::read_string(crate::reg::HKLM, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").map(|s| s.trim().to_string()).unwrap_or_default()
}

/// PDH query over GPU engine utilization and adapter memory.
struct GpuQuery {
    q: PDH_HQUERY,
    engine: PDH_HCOUNTER,
    dedicated: PDH_HCOUNTER,
    shared: PDH_HCOUNTER,
    /// \Processor Information(_Total)\% Processor Performance (current speed = base x this / 100)
    perf: PDH_HCOUNTER,
    perf_pct: f64,
    /// (luid key, name, dedicated total, shared total) from DXGI
    adapters: Vec<(String, String, u64, u64, String)>,
}

impl GpuQuery {
    fn open() -> Option<Self> {
        unsafe {
            let mut q = PDH_HQUERY::default();
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut q) != 0 {
                return None;
            }
            let add = |path: &str| -> PDH_HCOUNTER {
                let w = wide(path);
                let mut c = PDH_HCOUNTER::default();
                let _ = PdhAddEnglishCounterW(q, PCWSTR(w.as_ptr()), 0, &mut c);
                c
            };
            let engine = add(r"\GPU Engine(*)\Utilization Percentage");
            let dedicated = add(r"\GPU Adapter Memory(*)\Dedicated Usage");
            let shared = add(r"\GPU Adapter Memory(*)\Shared Usage");
            let perf = add(r"\Processor Information(_Total)\% Processor Performance");
            PdhCollectQueryData(q);
            Some(GpuQuery { q, engine, dedicated, shared, perf, perf_pct: 0.0, adapters: dxgi_adapters() })
        }
    }

    fn array(&self, c: PDH_HCOUNTER) -> Vec<(String, f64)> {
        unsafe {
            let mut size = 0u32;
            let mut count = 0u32;
            let _ = PdhGetFormattedCounterArrayW(c, PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000), &mut size, &mut count, None);
            if size == 0 {
                return vec![];
            }
            let mut buf = vec![0u8; size as usize];
            if PdhGetFormattedCounterArrayW(c, PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000), &mut size, &mut count, Some(buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W)) != 0 {
                return vec![];
            }
            let items = std::slice::from_raw_parts(buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W, count as usize);
            items.iter().map(|it| (it.szName.to_string().unwrap_or_default(), it.FmtValue.Anonymous.doubleValue)).collect()
        }
    }

    fn read(&mut self) -> Option<Gpu> {
        unsafe {
            if PdhCollectQueryData(self.q) != 0 {
                return None;
            }
            let mut v = PDH_FMT_COUNTERVALUE::default();
            if PdhGetFormattedCounterValue(self.perf, PDH_FMT_DOUBLE, None, &mut v) == 0 {
                self.perf_pct = v.Anonymous.doubleValue;
            }
        }
        let mut g = Gpu::default();
        // per adapter (luid) and engine type: the sum over processes
        let mut by: HashMap<String, HashMap<String, f64>> = HashMap::new();
        let mut per_pid: HashMap<u32, f64> = HashMap::new();
        let luid_of = |name: &str| -> String {
            name.find("luid_").map(|i| name[i..].split("_phys").next().unwrap_or("").to_lowercase()).unwrap_or_default()
        };
        // instance: pid_1234_luid_0x..._0x..._phys_0_eng_0_engtype_3D
        for (name, v) in self.array(self.engine) {
            let ty = name.rsplit("engtype_").next().unwrap_or("").to_string();
            *by.entry(luid_of(&name)).or_default().entry(ty).or_default() += v;
            if let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) {
                let e = per_pid.entry(pid).or_default();
                *e = e.max(v);
            }
        }
        let mut ded: HashMap<String, u64> = HashMap::new();
        for (name, v) in self.array(self.dedicated) {
            *ded.entry(luid_of(&name)).or_default() += v as u64;
        }
        let mut sh: HashMap<String, u64> = HashMap::new();
        for (name, v) in self.array(self.shared) {
            *sh.entry(luid_of(&name)).or_default() += v as u64;
        }
        for (luid, types) in by {
            let info = self.adapters.iter().find(|a| a.0 == luid);
            // adapters DXGI does not list as hardware (Basic Render) are left out
            if info.is_none() && !self.adapters.is_empty() {
                continue;
            }
            let mut eng: Vec<(String, f32)> = types.into_iter().map(|(k, v)| (k, v.min(100.0) as f32)).collect();
            eng.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let (main, other): (Vec<_>, Vec<_>) = eng.into_iter().partition(|e| main_engine(&e.0));
            let util = main.first().map(|e| e.1).unwrap_or(0.0);
            g.adapters.push(Adapter {
                name: info.map(|i| i.1.clone()).unwrap_or_else(|| "GPU".into()),
                util,
                engines: main.into_iter().map(|(k, v)| (engine_label(&k), v)).collect(),
                other: other.into_iter().map(|(k, v)| (engine_label(&k), v)).collect(),
                dedicated_used: ded.get(&luid).copied().unwrap_or(0),
                dedicated_total: info.map(|i| i.2).unwrap_or(0),
                shared_used: sh.get(&luid).copied().unwrap_or(0),
                shared_total: info.map(|i| i.3).unwrap_or(0),
                driver: info.map(|i| i.4.clone()).unwrap_or_default(),
            });
        }
        // one fixed order everywhere: the Windows (DXGI) adapter order, as Windows Task Manager numbers them
        let order = |name: &str| self.adapters.iter().position(|x| x.1 == name).unwrap_or(usize::MAX);
        g.adapters.sort_by_key(|a| order(&a.name));
        g.util = g.adapters.iter().map(|a| a.util).fold(0.0, f32::max);
        g.engines = g.adapters.first().map(|a| a.engines.clone()).unwrap_or_default();
        g.per_pid = per_pid.into_iter().map(|(k, v)| (k, v.min(100.0) as f32)).collect();
        g.dedicated_used = g.adapters.iter().map(|a| a.dedicated_used).sum();
        g.shared_used = g.adapters.iter().map(|a| a.shared_used).sum();
        Some(g)
    }
}

impl Drop for GpuQuery {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.q);
        }
    }
}
