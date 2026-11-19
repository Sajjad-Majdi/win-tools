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
}

#[derive(Clone, Default, Debug)]
pub struct Gpu {
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
    pub snap: Snapshot,
}

impl Default for Perf {
    fn default() -> Self {
        Self::new()
    }
}

impl Perf {
    pub fn new() -> Self {
        Perf { last: None, cores_prev: vec![], disks_prev: HashMap::new(), nets_prev: HashMap::new(), gpu: None, cpu_name: cpu_name(), snap: Snapshot::default() }
    }

    /// `gpu`: also sample GPU counters (opens a PDH query on first use; ~30-60 ms the first time).
    pub fn refresh(&mut self, gpu: bool) {
        let now = Instant::now();
        let dt = self.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
        self.last = Some(now);
        let mut s = Snapshot { cpu_name: self.cpu_name.clone(), ..Default::default() };

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
            let mut d = Disk { name: format!("Disk {i}"), ..Default::default() };
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
                    let mut n = Net { name: from_wide(&r.Alias), desc: from_wide(&r.Description), link_bps: r.ReceiveLinkSpeed.max(r.TransmitLinkSpeed), ..Default::default() };
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

fn cpu_name() -> String {
    crate::reg::read_string(crate::reg::HKLM, r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").map(|s| s.trim().to_string()).unwrap_or_default()
}

/// PDH query over GPU engine utilization and adapter memory.
struct GpuQuery {
    q: PDH_HQUERY,
    engine: PDH_HCOUNTER,
    dedicated: PDH_HCOUNTER,
    shared: PDH_HCOUNTER,
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
            PdhCollectQueryData(q);
            Some(GpuQuery { q, engine, dedicated, shared })
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
        }
        let mut g = Gpu::default();
        let mut by_type: HashMap<String, f64> = HashMap::new();
        let mut per_pid: HashMap<u32, f64> = HashMap::new();
        // instance: pid_1234_luid_0x..._phys_0_eng_0_engtype_3D
        for (name, v) in self.array(self.engine) {
            let ty = name.rsplit("engtype_").next().unwrap_or("").to_string();
            *by_type.entry(ty).or_default() += v;
            if let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) {
                let e = per_pid.entry(pid).or_default();
                *e = e.max(v);
            }
        }
        let mut engines: Vec<(String, f32)> = by_type.into_iter().map(|(k, v)| (k, v.min(100.0) as f32)).collect();
        engines.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        g.util = engines.first().map(|e| e.1).unwrap_or(0.0);
        g.engines = engines;
        g.per_pid = per_pid.into_iter().map(|(k, v)| (k, v.min(100.0) as f32)).collect();
        g.dedicated_used = self.array(self.dedicated).iter().map(|x| x.1 as u64).sum();
        g.shared_used = self.array(self.shared).iter().map(|x| x.1 as u64).sum();
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
