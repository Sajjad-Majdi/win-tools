//! Worker thread: samples processes, performance, services, startup entries, hung windows, command lines and icons,
//! then tells the window. The first sample is published before anything slow runs, so the first frame is instant.
use crate::perf::{Perf, Snapshot};
use crate::procs::{Proc, Sampler};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const WM_DATA: u32 = WM_APP + 1;

/// An anomaly the optional `wtm sampler` reported (it keeps 30 min of history).
#[derive(Clone, Debug)]
pub struct HistAnomaly {
    pub pid: u32,
    pub kind: String,
    pub minutes: u32,
    pub value: f64,
}

#[derive(Default)]
pub struct Shared {
    pub generation: u64,
    pub procs: Arc<Vec<Proc>>,
    pub total_cpu: f32,
    pub perf: Snapshot,
    /// pid -> display names of the services it hosts
    pub services: HashMap<u32, Vec<String>>,
    pub services_all: Arc<Vec<crate::services::Service>>,
    pub startup: Arc<Vec<crate::startup::Entry>>,
    pub startup_gen: u64,
    /// pid -> command line
    pub cmdlines: HashMap<u32, String>,
    /// new icons since the window last took them: (exe path, size in px, BGRA pixels)
    pub icons: Vec<(String, u32, Vec<u8>)>,
    pub hung: Vec<u32>,
    pub hist: Vec<HistAnomaly>,
    pub gpu_names: Vec<String>,
    pub sample_ms: f32,
}

pub struct Ctl {
    pub hwnd: AtomicIsize,
    pub interval_ms: AtomicU64,
    pub paused: AtomicBool,
    pub now: AtomicBool,
    pub icon_px: AtomicU32,
    pub gpu: AtomicBool,
    pub reload_startup: AtomicBool,
    /// big icons the window wants: (path, px)
    pub icon_req: Mutex<Vec<(String, u32)>>,
}

pub fn start() -> (Arc<Mutex<Shared>>, Arc<Ctl>) {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let ctl = Arc::new(Ctl {
        hwnd: AtomicIsize::new(0),
        interval_ms: AtomicU64::new(1000),
        paused: AtomicBool::new(false),
        now: AtomicBool::new(false),
        icon_px: AtomicU32::new(16),
        gpu: AtomicBool::new(std::env::var("WT_NOGPU").is_err()),
        reload_startup: AtomicBool::new(false),
        icon_req: Mutex::new(vec![]),
    });
    let (s2, c2) = (shared.clone(), ctl.clone());
    std::thread::Builder::new().name("sampler".into()).stack_size(1 << 20).spawn(move || run(s2, c2)).unwrap();
    (shared, ctl)
}

fn notify(ctl: &Ctl) {
    let h = ctl.hwnd.load(Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_DATA, WPARAM(0), LPARAM(0));
        }
    }
}

/// Display names of the graphics adapters (registry, display class).
fn gpu_names() -> Vec<String> {
    let mut v = vec![];
    for i in 0..6 {
        let key = format!(r"SYSTEM\CurrentControlSet\Control\Class\{{4d36e968-e325-11ce-bfc1-08002be10318}}\{i:04}");
        if let Some(n) = crate::reg::read_string(crate::reg::HKLM, &key, "DriverDesc") {
            let n = n.trim().to_string();
            if !n.is_empty() && !n.contains("Basic Display") && !n.contains("Basic Render") && !v.contains(&n) {
                v.push(n);
            }
        }
    }
    v
}

fn history_anomalies() -> Option<Vec<HistAnomaly>> {
    if !crate::ipc::exists("history") {
        return None;
    }
    let r = crate::ipc::send_to("history", "anomalies").ok()?;
    let v: serde_json::Value = serde_json::from_str(&r).ok()?;
    let a = v["anomalies"].as_array()?;
    Some(
        a.iter()
            .map(|x| HistAnomaly { pid: x["pid"].as_u64().unwrap_or(0) as u32, kind: x["kind"].as_str().unwrap_or("").to_string(), minutes: x["minutes"].as_u64().unwrap_or(0) as u32, value: x["value"].as_f64().unwrap_or(0.0) })
            .collect(),
    )
}

fn run(shared: Arc<Mutex<Shared>>, ctl: Arc<Ctl>) {
    let mut s = Sampler::new();
    let mut pf = Perf::new();
    let mut tick = 0u64;
    let mut cmd_seen: HashSet<(u32, i64)> = HashSet::new();
    let mut cmdlines: HashMap<u32, String> = HashMap::new();
    let mut icon_done: HashSet<(String, u32)> = HashSet::new();
    let mut services: HashMap<u32, Vec<String>> = HashMap::new();
    let mut services_all: Arc<Vec<crate::services::Service>> = Arc::new(vec![]);
    let mut hist: Vec<HistAnomaly> = vec![];
    loop {
        let t0 = Instant::now();
        s.refresh(true);
        pf.refresh(ctl.gpu.load(Relaxed) && tick > 0);
        let hung = crate::insight::hung_pids();
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        let publish = |s: &Sampler, pf: &Perf, icons: Vec<(String, u32, Vec<u8>)>, extra: &dyn Fn(&mut Shared)| {
            let mut sh = shared.lock().unwrap();
            sh.generation += 1;
            sh.procs = Arc::new(s.procs.clone());
            sh.total_cpu = s.total_cpu;
            sh.perf = pf.snap.clone();
            sh.hung = hung.clone();
            sh.icons.extend(icons);
            sh.sample_ms = ms;
            extra(&mut sh);
        };
        publish(&s, &pf, vec![], &|sh| {
            sh.services = services.clone();
            sh.cmdlines = cmdlines.clone();
            sh.services_all = services_all.clone();
            sh.hist = hist.clone();
        });
        notify(&ctl);
        // slower facts, after the frame is out
        let mut more = false;
        let live: HashSet<(u32, i64)> = s.procs.iter().map(|p| (p.pid, p.create_time)).collect();
        cmd_seen.retain(|k| live.contains(k));
        cmdlines.retain(|pid, _| live.iter().any(|k| k.0 == *pid));
        for p in &s.procs {
            if cmd_seen.insert((p.pid, p.create_time))
                && let Some(c) = crate::procs::command_line(p.pid)
            {
                cmdlines.insert(p.pid, c);
                more = true;
            }
        }
        more |= s.fill_desc(usize::MAX) > 0;
        if tick == 0 {
            // names and command lines are in: publish before the slower services, icons and startup entries
            publish(&s, &pf, vec![], &|sh| {
                sh.cmdlines = cmdlines.clone();
            });
            notify(&ctl);
        }
        if tick % 5 == 0 {
            services.clear();
            let all = crate::services::list();
            for x in &all {
                if x.pid != 0 && x.state == "running" {
                    services.entry(x.pid).or_default().push(x.display.clone());
                }
            }
            services_all = Arc::new(all);
            more = true;
        }
        let px = ctl.icon_px.load(Relaxed);
        let mut icons = vec![];
        for p in &s.procs {
            if !p.meta.path.is_empty() && icon_done.insert((p.meta.path.to_lowercase(), px)) {
                icons.push((p.meta.path.clone(), px, super::icons::icon_bgra(&p.meta.path, px).unwrap_or_default()));
            }
        }
        icons.extend(take_icon_requests(&ctl, &mut icon_done));
        more |= !icons.is_empty();
        let mut startup = None;
        if tick == 1 || tick % 60 == 0 || ctl.reload_startup.swap(false, Relaxed) {
            startup = Some(Arc::new(crate::startup::list_all()));
            more = true;
        }
        let gpus = if tick == 1 { Some(gpu_names()) } else { None };
        if tick % 10 == 2
            && let Some(h) = history_anomalies()
        {
            hist = h;
            more = true;
        }
        if more || tick % 5 == 0 {
            publish(&s, &pf, icons, &|sh| {
                sh.services = services.clone();
                sh.cmdlines = cmdlines.clone();
                sh.services_all = services_all.clone();
                sh.hist = hist.clone();
                if let Some(st) = &startup {
                    sh.startup = st.clone();
                    sh.startup_gen += 1;
                }
                if let Some(g) = &gpus {
                    sh.gpu_names = g.clone();
                }
            });
            notify(&ctl);
        }
        tick += 1;
        // wait for the next tick (or a "refresh now" / un-pause / icon request / startup reload)
        let until = t0 + Duration::from_millis(ctl.interval_ms.load(Relaxed));
        while Instant::now() < until || ctl.paused.load(Relaxed) {
            if ctl.now.swap(false, Relaxed) {
                break;
            }
            if ctl.reload_startup.swap(false, Relaxed) {
                let st = Arc::new(crate::startup::list_all());
                let mut sh = shared.lock().unwrap();
                sh.startup = st;
                sh.startup_gen += 1;
                drop(sh);
                notify(&ctl);
            }
            let req = take_icon_requests(&ctl, &mut icon_done);
            if !req.is_empty() {
                shared.lock().unwrap().icons.extend(req);
                notify(&ctl);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

fn take_icon_requests(ctl: &Ctl, done: &mut HashSet<(String, u32)>) -> Vec<(String, u32, Vec<u8>)> {
    let req: Vec<(String, u32)> = std::mem::take(&mut *ctl.icon_req.lock().unwrap());
    let mut out = vec![];
    for (path, px) in req {
        if done.insert((path.to_lowercase(), px)) {
            let px_data = super::icons::icon_bgra(&path, px).unwrap_or_default();
            out.push((path, px, px_data));
        }
    }
    out
}
