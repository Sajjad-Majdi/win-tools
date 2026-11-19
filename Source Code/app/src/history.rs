//! Optional background sampler (`wtm sampler`): every 10 s it records CPU and memory of every process for the last
//! 30 minutes, so the window can say "high CPU for 6 min" or "+400 MB in 10 min" the moment it opens.
//! It answers on the "history" pipe: `anomalies`, `series <pid>`, `cost`.
//! Without it, `anomalies_now` gives what can be known at once (hung windows, CPU averaged over the process life).
use crate::insight;
use crate::procs::{Proc, Sampler};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const STEP_S: u64 = 10;
const KEEP: usize = 180; // 30 min

#[derive(Default)]
struct Track {
    name: String,
    label: String,
    /// (seconds since sampler start, cpu % of PC, private memory bytes)
    pts: Vec<(u32, f32, u64)>,
}

#[derive(Default)]
pub struct History {
    tracks: HashMap<(u32, i64), Track>,
    started: Option<Instant>,
    samples: u64,
    last_ms: f32,
    /// whole-PC CPU % of the last sample
    total: f32,
}

#[derive(Clone, Debug)]
pub struct Anomaly {
    pub pid: u32,
    pub name: String,
    pub label: String,
    /// "not_responding" | "high_cpu" | "memory_growing"
    pub kind: &'static str,
    pub minutes: u32,
    pub value: f64,
    pub text: String,
}

impl Anomaly {
    pub fn json(&self) -> Value {
        json!({"pid": self.pid, "name": self.name, "label": self.label, "kind": self.kind, "minutes": self.minutes, "value": self.value, "text": self.text})
    }
}

impl History {
    fn add(&mut self, procs: &[Proc], ms: f32) {
        let t = self.started.get_or_insert_with(Instant::now).elapsed().as_secs() as u32;
        self.samples += 1;
        self.last_ms = ms;
        let live: std::collections::HashSet<(u32, i64)> = procs.iter().map(|p| (p.pid, p.create_time)).collect();
        self.tracks.retain(|k, _| live.contains(k));
        for p in procs.iter().filter(|p| p.pid != 0) {
            let tr = self.tracks.entry((p.pid, p.create_time)).or_insert_with(|| Track { name: p.name.clone(), label: p.meta.description.clone(), pts: vec![] });
            tr.pts.push((t, p.cpu, p.mem));
            if tr.pts.len() > KEEP {
                tr.pts.remove(0);
            }
        }
    }

    pub fn anomalies(&self, ncpu: usize) -> Vec<Anomaly> {
        let core = 0.8 * 100.0 / ncpu as f32; // 80% of one core, as % of the PC
        let mut out = vec![];
        for ((pid, _), tr) in &self.tracks {
            let n = tr.pts.len();
            if n < 2 {
                continue;
            }
            // high CPU: trailing run of samples at >= 1 core, at least 2 minutes long
            let run = tr.pts.iter().rev().take_while(|p| p.1 >= core).count();
            if run >= 2 {
                let first = tr.pts[n - run].0;
                let mins = (tr.pts[n - 1].0 - first) / 60 + if run > 1 { 0 } else { 0 };
                let avg = tr.pts[n - run..].iter().map(|p| p.1).sum::<f32>() / run as f32;
                if tr.pts[n - 1].0 - first >= 120 {
                    out.push(Anomaly { pid: *pid, name: tr.name.clone(), label: tr.label.clone(), kind: "high_cpu", minutes: mins.max(2), value: avg as f64, text: format!("high CPU for {} min (avg {:.1}%)", mins.max(2), avg) });
                }
            }
            // memory growing: +200 MB from the lowest point in the window to now, still near its top
            let (tmin, mmin) = tr.pts.iter().map(|p| (p.0, p.2)).min_by_key(|p| p.1).unwrap();
            let (tnow, mnow) = (tr.pts[n - 1].0, tr.pts[n - 1].2);
            let mmax = tr.pts.iter().map(|p| p.2).max().unwrap();
            if mnow > mmin + 200 * 1048576 && tnow > tmin && mnow + 50 * 1048576 >= mmax {
                let mins = ((tnow - tmin) / 60).max(1);
                let grow = (mnow - mmin) as f64 / 1048576.0;
                out.push(Anomaly { pid: *pid, name: tr.name.clone(), label: tr.label.clone(), kind: "memory_growing", minutes: mins, value: grow, text: format!("memory +{grow:.0} MB in {mins} min") });
            }
        }
        out.sort_by(|a, b| b.value.total_cmp(&a.value));
        out
    }

    fn series(&self, pid: u32) -> Option<Value> {
        let (_, tr) = self.tracks.iter().find(|(k, _)| k.0 == pid)?;
        Some(json!({"pid": pid, "name": tr.name, "step_s": STEP_S, "points": tr.pts.iter().map(|p| json!([p.0, (p.1 * 10.0).round() / 10.0, p.2])).collect::<Vec<_>>()}))
    }
}

/// Instant checks, no history: hung windows and CPU averaged over each process's life.
pub fn anomalies_now(procs: &[Proc]) -> Vec<Anomaly> {
    let ncpu = crate::sys::cpu_count();
    let core = 100.0 / ncpu as f32;
    let now = insight::now_filetime();
    let mut out = vec![];
    for pid in insight::hung_pids() {
        if let Some(p) = procs.iter().find(|p| p.pid == pid) {
            out.push(Anomaly { pid, name: p.name.clone(), label: p.meta.description.clone(), kind: "not_responding", minutes: 0, value: 0.0, text: "not responding".into() });
        }
    }
    for p in procs.iter().filter(|p| p.pid > 4) {
        let age_min = ((now - p.create_time) / 600_000_000) as u32;
        let life = insight::lifetime_cpu(p, now, ncpu);
        if age_min >= 2 && life >= core && p.kind != crate::procs::Kind::Windows {
            out.push(Anomaly { pid: p.pid, name: p.name.clone(), label: p.meta.description.clone(), kind: "high_cpu", minutes: age_min, value: life as f64, text: format!("averaged {life:.1}% CPU over its {age_min} min") });
        }
    }
    out
}

/// Run the sampler until killed. Answers on the "history" pipe.
pub fn run_sampler() -> ! {
    let hist = Arc::new(Mutex::new(History::default()));
    let h2 = hist.clone();
    std::thread::spawn(move || {
        crate::ipc::serve_on("history", |line| {
            let ncpu = crate::sys::cpu_count();
            let h = h2.lock().unwrap();
            let mut it = line.split_whitespace();
            match it.next().unwrap_or("") {
                "anomalies" => json!({"ok": true, "minutes_recorded": h.started.map(|s| s.elapsed().as_secs() / 60).unwrap_or(0), "anomalies": h.anomalies(ncpu).iter().map(|a| a.json()).collect::<Vec<_>>()}).to_string(),
                "series" => match it.next().and_then(|p| p.parse::<u32>().ok()).and_then(|pid| h.series(pid)) {
                    Some(v) => v.to_string(),
                    None => json!({"ok": false, "error": "no history for that PID"}).to_string(),
                },
                "latest" => {
                    let mut m = serde_json::Map::new();
                    for ((pid, _), tr) in &h.tracks {
                        if let Some(p) = tr.pts.last() {
                            m.insert(pid.to_string(), json!((p.1 * 10.0).round() / 10.0));
                        }
                    }
                    json!({"ok": true, "cpu": m, "total": h.total}).to_string()
                }
                "cost" => {
                    let me = std::process::id();
                    let mut s = Sampler::new();
                    s.refresh(false);
                    let p = s.procs.iter().find(|p| p.pid == me).cloned();
                    json!({"ok": true, "samples": h.samples, "last_sample_ms": h.last_ms, "cpu_seconds_total": p.as_ref().map(|p| p.cpu_time as f64 / 1e7), "private_bytes": p.as_ref().map(|p| p.mem), "working_set": p.as_ref().map(|p| p.ws), "tracked": h.tracks.len()}).to_string()
                }
                _ => json!({"ok": false, "error": "anomalies | series <pid> | cost"}).to_string(),
            }
        })
    });
    let mut s = Sampler::new();
    let mut ticks = 0u64;
    loop {
        let t0 = Instant::now();
        s.refresh(ticks % 30 == 0);
        if ticks % 30 == 0 {
            s.fill_desc(usize::MAX);
        }
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        if ticks > 0 {
            let mut h = hist.lock().unwrap();
            h.add(&s.procs, ms);
            h.total = s.total_cpu;
        }
        ticks += 1;
        std::thread::sleep(Duration::from_secs(if ticks == 1 { 1 } else { STEP_S }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(pts: Vec<(u32, f32, u64)>) -> History {
        let mut h = History::default();
        h.tracks.insert((42, 1), Track { name: "t.exe".into(), label: "T".into(), pts });
        h
    }

    #[test]
    fn sustained_cpu_is_flagged_short_burst_is_not() {
        let busy: Vec<_> = (0..15).map(|i| (i * 10, 7.0, 10 << 20)).collect(); // 140 s at one core of 16
        assert!(track(busy).anomalies(16).iter().any(|a| a.kind == "high_cpu"));
        let burst: Vec<_> = (0..15).map(|i| (i * 10, if i > 12 { 50.0 } else { 0.1 }, 10 << 20)).collect();
        assert!(!track(burst).anomalies(16).iter().any(|a| a.kind == "high_cpu"));
    }

    #[test]
    fn growing_memory_is_flagged_flat_is_not() {
        let grow: Vec<_> = (0..20).map(|i| (i * 10, 0.0, (100 + i as u64 * 20) << 20)).collect();
        let a = track(grow).anomalies(16);
        assert!(a.iter().any(|a| a.kind == "memory_growing" && a.value >= 350.0));
        let flat: Vec<_> = (0..20).map(|i| (i * 10, 0.0, 500 << 20)).collect();
        assert!(track(flat).anomalies(16).is_empty());
    }
}
