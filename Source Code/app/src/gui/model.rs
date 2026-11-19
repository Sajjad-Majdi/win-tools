//! Turns the flat process sample into what the Processes page shows: merged rows (same exe + same product),
//! children named by what they run, impact order with "Not responding" pinned on top, search results grouped
//! into Processes / Startup apps / Services, and short per-row history for sparklines and attention marks.
use crate::procs::{self, Kind, Proc};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Impact,
    Cpu,
    Mem,
    Disk,
    Net,
    Gpu,
    Name,
}

impl SortKey {
    pub const MENU: [SortKey; 6] = [SortKey::Impact, SortKey::Cpu, SortKey::Mem, SortKey::Disk, SortKey::Gpu, SortKey::Name];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Impact => "Impact",
            SortKey::Cpu => "CPU",
            SortKey::Mem => "Memory",
            // read + write of files, pipes and devices (Windows has no per-app disk-only counter without ETW)
            SortKey::Disk => "I/O",
            SortKey::Net => "Network",
            SortKey::Gpu => "GPU",
            SortKey::Name => "Name",
        }
    }
    pub fn parse(s: &str) -> Option<SortKey> {
        Some(match s.to_lowercase().as_str() {
            "impact" => SortKey::Impact,
            "cpu" => SortKey::Cpu,
            "memory" | "mem" => SortKey::Mem,
            "disk" | "io" | "i/o" => SortKey::Disk,
            "network" | "net" => SortKey::Net,
            "gpu" => SortKey::Gpu,
            "name" => SortKey::Name,
            _ => return None,
        })
    }
}

/// Metric columns of the list.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Res {
    Cpu,
    Mem,
    Disk,
    Net,
    Gpu,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub pid: u32,
    pub ppid: u32,
    pub create_time: i64,
    pub name: String,
    /// what tells this child apart: window title, script, --type, ...
    pub sub: String,
    pub cpu: f32,
    pub mem: u64,
    pub io: f64,
    pub gpu: f32,
    pub cpu_time: u64,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub key: String,
    pub label: String,
    /// exe name of the group ("node.exe")
    pub exe: String,
    pub path: String,
    pub company: String,
    pub kind: Kind,
    /// protected Windows process (lock glyph)
    pub sys: bool,
    pub hung: bool,
    pub members: Vec<Member>,
    pub cpu: f32,
    pub mem: u64,
    pub io: f64,
    pub gpu: f32,
    pub cpu_time: u64,
    pub score: f32,
    pub driver: &'static str,
}

impl Group {
    pub fn pids(&self) -> Vec<u32> {
        self.members.iter().map(|m| m.pid).collect()
    }
    /// The member whose parent is not in the group (the one to restart); else the oldest.
    pub fn root(&self) -> u32 {
        let ids: Vec<u32> = self.pids();
        self.members.iter().filter(|m| !ids.contains(&m.ppid)).min_by_key(|m| m.create_time).or_else(|| self.members.iter().min_by_key(|m| m.create_time)).map(|m| m.pid).unwrap_or(0)
    }
    pub fn value(&self, r: Res, net: f64) -> f64 {
        match r {
            Res::Cpu => self.cpu as f64,
            Res::Mem => self.mem as f64,
            Res::Disk => self.io,
            Res::Net => net,
            Res::Gpu => self.gpu as f64,
        }
    }
    pub fn type_label(&self) -> &'static str {
        match self.kind {
            Kind::App => "App",
            Kind::Background => "Background",
            Kind::Windows => "Windows",
        }
    }
}

const PROTECTED: &[&str] = &["system", "registry", "memory compression", "secure system", "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "lsaiso.exe", "dwm.exe", "msmpeng.exe", "fontdrvhost.exe", "svchost.exe"];

fn label_for(p: &Proc, services: &HashMap<u32, Vec<String>>) -> String {
    if p.name.eq_ignore_ascii_case("svchost.exe") {
        return match services.get(&p.pid) {
            Some(s) if s.len() == 1 => format!("Service Host: {}", s[0]),
            Some(s) if !s.is_empty() => format!("Service Host: {} (+{})", s[0], s.len() - 1),
            _ => "Service Host".into(),
        };
    }
    if !p.meta.description.is_empty() {
        return p.meta.description.clone();
    }
    p.name.clone()
}

/// Merge processes by exe name + product (description); svchost stays one row per process.
pub fn groups(procs: &[Proc], services: &HashMap<u32, Vec<String>>, cmdlines: &HashMap<u32, String>, gpu: &HashMap<u32, f32>, hung: &[u32], merge: bool, ram_total: u64) -> Vec<Group> {
    let mut map: HashMap<String, Group> = HashMap::new();
    let mut order: Vec<String> = vec![];
    // an exe without a description joins the one product of the same name (bash.exe -> "Git for Windows"), so
    // one program is one row; names with several products (claude.exe: Claude, Claude Code) stay apart
    let mut products: HashMap<String, Vec<String>> = HashMap::new();
    for p in procs.iter().filter(|p| p.pid != 0 && !p.meta.description.is_empty()) {
        let v = products.entry(p.name.to_lowercase()).or_default();
        let l = label_for(p, services);
        if !v.contains(&l) {
            v.push(l);
        }
    }
    for p in procs.iter().filter(|p| p.pid != 0) {
        let mut label = label_for(p, services);
        if p.meta.description.is_empty()
            && let Some([only]) = products.get(&p.name.to_lowercase()).map(|v| v.as_slice())
        {
            label = only.clone();
        }
        let svc_host = p.name.eq_ignore_ascii_case("svchost.exe");
        let key = if !merge || svc_host { format!("{}#{}", p.name.to_lowercase(), p.pid) } else { format!("{}|{}", p.name.to_lowercase(), label.to_lowercase()) };
        let m = Member {
            pid: p.pid,
            ppid: p.ppid,
            create_time: p.create_time,
            name: p.name.clone(),
            sub: crate::insight::child_label(p, cmdlines.get(&p.pid).map(|s| s.as_str())),
            cpu: p.cpu,
            mem: p.mem,
            io: p.io_rate,
            gpu: gpu.get(&p.pid).copied().unwrap_or(0.0),
            cpu_time: p.cpu_time,
        };
        let g = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            let lname = p.name.to_lowercase();
            Group {
                key,
                label: label.clone(),
                exe: p.name.clone(),
                path: p.meta.path.clone(),
                company: p.meta.company.clone(),
                kind: p.kind,
                sys: p.kind == Kind::Windows && (p.meta.path.is_empty() || PROTECTED.contains(&lname.as_str())),
                hung: false,
                members: vec![],
                cpu: 0.0,
                mem: 0,
                io: 0.0,
                gpu: 0.0,
                cpu_time: 0,
                score: 0.0,
                driver: "memory",
            }
        });
        if p.kind == Kind::App {
            g.kind = Kind::App;
        }
        if g.path.is_empty() {
            g.path = p.meta.path.clone();
        }
        if hung.contains(&p.pid) {
            g.hung = true;
        }
        g.cpu += m.cpu;
        g.mem += m.mem;
        g.io += m.io;
        g.gpu = (g.gpu + m.gpu).min(100.0);
        g.cpu_time += m.cpu_time;
        g.members.push(m);
    }
    let mut v: Vec<Group> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    for g in &mut v {
        // children keep a fixed order (by PID), so expanded groups never shuffle
        g.members.sort_by_key(|m| m.pid);
        let i = crate::insight::impact(g.cpu, g.mem, g.io, g.gpu, ram_total);
        g.score = i.score;
        g.driver = i.driver;
    }
    v
}

/// The value a sort ranks by (bigger first); Name has none.
pub fn sort_val(g: &Group, key: SortKey) -> f64 {
    match key {
        SortKey::Impact | SortKey::Net => g.score as f64,
        SortKey::Cpu => g.cpu as f64,
        SortKey::Mem => g.mem as f64,
        SortKey::Disk => g.io,
        SortKey::Gpu => g.gpu as f64,
        SortKey::Name => 0.0,
    }
}

/// The smallest gap that may move a row past its neighbour: (relative, absolute). Smaller changes are noise.
fn min_gap(key: SortKey) -> (f64, f64) {
    match key {
        SortKey::Impact | SortKey::Net => (0.35, 1.0),
        SortKey::Cpu => (0.35, 1.5),
        SortKey::Mem => (0.15, 32.0 * 1048576.0),
        SortKey::Disk => (0.50, 512.0 * 1024.0),
        SortKey::Gpu => (0.35, 2.0),
        SortKey::Name => (0.0, 0.0),
    }
}

/// A calm order for one list (one section, or the flat list): keep the previous order and let a row pass its
/// neighbour only when it is clearly bigger (both gaps of `min_gap`); ties and small changes never move a row.
/// New rows go where their value puts them. `rev` = smallest first. Name is an exact A-Z (or Z-A) order.
/// `smooth`: the sort value averaged over the last samples (so a one-second burst does not move a row).
pub fn calm_order(mut v: Vec<Group>, prev: &HashMap<String, usize>, key: SortKey, rev: bool, smooth: &HashMap<String, f64>) -> Vec<Group> {
    if key == SortKey::Name {
        v.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()).then(a.key.cmp(&b.key)));
        if rev {
            v.reverse();
        }
        return v;
    }
    let sign = if rev { -1.0 } else { 1.0 };
    let val = |g: &Group| smooth.get(&g.key).copied().unwrap_or_else(|| sort_val(g, key)) * sign;
    // the plain order by value (ties by key, so it is the same every time)
    v.sort_by(|a, b| val(b).total_cmp(&val(a)).then(a.key.cmp(&b.key)));
    if prev.is_empty() {
        return v;
    }
    let (old, new): (Vec<Group>, Vec<Group>) = v.into_iter().partition(|g| prev.contains_key(&g.key));
    let mut list = old;
    list.sort_by_key(|g| prev[&g.key]);
    for g in new {
        let at = list.iter().position(|x| val(&g) > val(x)).unwrap_or(list.len());
        list.insert(at, g);
    }
    let (rel, abs) = min_gap(key);
    let clearly = |a: f64, b: f64| a - b > abs.max(a.abs().max(b.abs()) * rel);
    for _ in 0..list.len() {
        let mut moved = false;
        for i in 0..list.len().saturating_sub(1) {
            if clearly(val(&list[i + 1]), val(&list[i])) {
                list.swap(i, i + 1);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    list
}

pub fn sort_groups(v: &mut [Group], key: SortKey) {
    match key {
        SortKey::Impact => v.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.mem.cmp(&a.mem))),
        SortKey::Cpu => v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem))),
        SortKey::Mem => v.sort_by(|a, b| b.mem.cmp(&a.mem)),
        SortKey::Disk => v.sort_by(|a, b| b.io.total_cmp(&a.io).then(b.mem.cmp(&a.mem))),
        SortKey::Gpu => v.sort_by(|a, b| b.gpu.total_cmp(&a.gpu).then(b.mem.cmp(&a.mem))),
        SortKey::Net => v.sort_by(|a, b| b.score.total_cmp(&a.score)),
        SortKey::Name => v.sort_by_key(|a| a.label.to_lowercase()),
    }
}

/// Keep the old order unless a row moved 2+ places (so the list does not jump on small changes).
pub fn stable_order(v: &mut Vec<Group>, prev: &HashMap<String, usize>) {
    if prev.is_empty() {
        return;
    }
    let mut keyed: Vec<(f32, usize, Group)> = std::mem::take(v)
        .into_iter()
        .enumerate()
        .map(|(new, g)| {
            let k = match prev.get(&g.key) {
                Some(&old) if (old as i64 - new as i64).abs() <= 3 => old as f32,
                _ => new as f32 - 0.5,
            };
            (k, new, g)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    *v = keyed.into_iter().map(|x| x.2).collect();
}

// ------------------------------------------------------------------------------------------------ search
pub fn tokens(q: &str) -> Vec<String> {
    procs::tokens(q)
}

fn hay_match(fields: &[&str], toks: &[String]) -> bool {
    toks.iter().all(|t| fields.iter().any(|f| f.to_lowercase().contains(t.as_str())))
}

/// Does a member match the search? (name, title/sub, description, company, path, command line, pid)
pub fn member_matches(p: &Proc, cmd: Option<&str>, toks: &[String]) -> bool {
    procs::match_rank(p, toks, cmd).is_some()
}

pub fn startup_matches(e: &crate::startup::Entry, toks: &[String]) -> bool {
    hay_match(&[&e.name, &e.command], toks)
}

pub fn service_matches(s: &crate::services::Service, toks: &[String]) -> bool {
    hay_match(&[&s.name, &s.display], toks)
}

// ------------------------------------------------------------------------------------------------ history
pub const HIST_N: usize = 60;

#[derive(Default, Clone)]
pub struct Hist {
    pub cpu: VecDeque<f32>,
    pub mem: VecDeque<f32>,
    pub io: VecDeque<f32>,
    pub gpu: VecDeque<f32>,
    /// since when the row uses >= 10% of the PC
    pub busy_since: Option<Instant>,
    /// memory every ~10 s for the last 10 min
    pub mem_slow: VecDeque<(Instant, u64)>,
    pub seen: u64,
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    q.push_back(v);
    while q.len() > HIST_N {
        q.pop_front();
    }
}

impl Hist {
    pub fn add(&mut self, cpu: f32, mem: u64, io: f64, gpu: f32, now: Instant, gn: u64) {
        push(&mut self.cpu, cpu);
        push(&mut self.mem, mem as f32);
        push(&mut self.io, io as f32);
        push(&mut self.gpu, gpu);
        self.seen = gn;
        if cpu >= 10.0 {
            self.busy_since.get_or_insert(now);
        } else {
            self.busy_since = None;
        }
        if self.mem_slow.back().map(|(t, _)| now.duration_since(*t).as_secs() >= 10).unwrap_or(true) {
            self.mem_slow.push_back((now, mem));
            while self.mem_slow.len() > 61 {
                self.mem_slow.pop_front();
            }
        }
    }

    pub fn series(&self, r: Res) -> Vec<f32> {
        let q = match r {
            Res::Cpu => &self.cpu,
            Res::Mem => &self.mem,
            Res::Disk => &self.io,
            Res::Gpu => &self.gpu,
            Res::Net => return vec![],
        };
        q.iter().copied().collect()
    }

    /// Minutes of >= 10% CPU so far (0 = not busy).
    pub fn busy_min(&self, now: Instant) -> u32 {
        self.busy_since.map(|t| (now.duration_since(t).as_secs() / 60) as u32).unwrap_or(0)
    }

    /// (+bytes, minutes) when memory grew on >= 80% of samples by >= 200 MB (or 25%) over the window.
    pub fn growth(&self) -> Option<(u64, u32)> {
        let n = self.mem_slow.len();
        if n < 6 {
            return None;
        }
        let first = self.mem_slow.front()?;
        let last = self.mem_slow.back()?;
        let ups = self.mem_slow.iter().zip(self.mem_slow.iter().skip(1)).filter(|(a, b)| b.1 >= a.1).count();
        let grew = last.1.saturating_sub(first.1);
        let mins = (last.0.duration_since(first.0).as_secs() / 60) as u32;
        if ups * 5 >= (n - 1) * 4 && (grew >= 200 << 20 || (first.1 > 0 && grew * 4 >= first.1 && grew >= 50 << 20)) {
            Some((grew, mins.max(1)))
        } else {
            None
        }
    }
}

// ------------------------------------------------------------------------------------------------ formatting
pub fn fmt_mem(b: u64) -> String {
    let mb = b as f64 / 1048576.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if b == 0 {
        "0 MB".into()
    } else if b < 1048576 {
        format!("{} KB", (b / 1024).max(1))
    } else {
        format!("{:.0} MB", mb.max(0.1))
    }
}

pub fn fmt_rate(b: f64) -> String {
    let kb = b / 1024.0;
    if b < 1.0 {
        "0 KB/s".into()
    } else if kb >= 1000.0 {
        format!("{:.1} MB/s", kb / 1024.0)
    } else if kb < 10.0 {
        format!("{:.1} KB/s", kb.max(0.1))
    } else {
        format!("{kb:.0} KB/s")
    }
}

pub fn fmt_pct(v: f32) -> String {
    if v < 0.05 { "0%".into() } else { format!("{v:.1}%") }
}

pub fn fmt_gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1073741824.0)
}

pub fn fmt_dur(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}
