//! The Performance page: a list of resources on the left (CPU, memory, every disk, every network adapter, every
//! GPU) with a mini graph each, and the selected one on the right: a large graph, "used of total" facts with
//! units, and the processes that use it most (click one to jump to it). Below 760 the list becomes chips on top.
//! Nothing here animates: it is drawn again only when a new sample arrives (or on input).
use super::app::*;
use super::gfx::{Align, C, Font, Rect};
use super::model;
use std::collections::VecDeque;

pub struct PerfItem {
    pub key: String,
    pub title: String,
    /// short line under the title (and the chip text when narrow)
    pub sub: String,
    pub color: C,
    /// the graph's top value (0 = from the history's peak)
    pub max: f32,
    pub floor: f32,
}

fn ghz(mhz: u32) -> String {
    format!("{:.2} GHz", mhz as f64 / 1000.0)
}

fn pct_of(a: u64, b: u64) -> f64 {
    a as f64 * 100.0 / b.max(1) as f64
}

impl App {
    /// The resources shown on the page, in order.
    pub fn perf_items(&self) -> Vec<PerfItem> {
        let s = &self.perf;
        let t = self.t;
        let used = s.mem_total.saturating_sub(s.mem_avail);
        let mut v = vec![
            PerfItem { key: "cpu".into(), title: "CPU".into(), sub: format!("{:.0}% · {}", s.cpu, ghz(if s.cur_mhz > 0 { s.cur_mhz } else { s.base_mhz })), color: t.cpu, max: 100.0, floor: 0.0 },
            PerfItem { key: "mem".into(), title: "Memory".into(), sub: format!("{} of {} ({:.0}%)", model::fmt_gb(used), model::fmt_gb(s.mem_total), pct_of(used, s.mem_total)), color: t.mem, max: s.mem_total as f32, floor: 0.0 },
        ];
        for d in &s.disks {
            let kind = match d.ssd {
                Some(true) => "SSD",
                Some(false) => "HDD",
                None => "Disk",
            };
            let title = if d.letters.is_empty() { d.name.clone() } else { format!("{} ({})", d.name, d.letters) };
            v.push(PerfItem { key: format!("disk:{}", d.name), title, sub: format!("{kind} · {:.0}%", d.active), color: t.disk, max: 100.0, floor: 0.0 });
        }
        for n in &s.nets {
            v.push(PerfItem { key: format!("net:{}", n.name), title: n.name.clone(), sub: format!("↓ {}  ↑ {}", model::fmt_rate(n.recv_rate), model::fmt_rate(n.send_rate)), color: t.net, max: 0.0, floor: 100.0 * 1024.0 });
        }
        if let Some(g) = &s.gpu {
            for (i, a) in g.adapters.iter().enumerate() {
                let short = a.name.replace("(TM)", "").replace("NVIDIA GeForce ", "").replace("  ", " ");
                v.push(PerfItem { key: format!("gpu:{}", a.name), title: format!("GPU {i}"), sub: format!("{} · {:.0}%", short.trim(), a.util), color: t.gpu, max: 100.0, floor: 0.0 });
            }
        }
        v
    }

    /// Add this sample to the page's histories (called on every new sample).
    pub fn push_perf_hist(&mut self) {
        let s = self.perf.clone();
        let mut add = |k: String, v: f32| {
            let q = self.perf_hist.entry(k).or_insert_with(VecDeque::new);
            q.push_back(v);
            while q.len() > model::HIST_N {
                q.pop_front();
            }
        };
        add("cpu".into(), s.cpu);
        add("mem".into(), s.mem_total.saturating_sub(s.mem_avail) as f32);
        for d in &s.disks {
            add(format!("disk:{}", d.name), d.active);
        }
        for n in &s.nets {
            add(format!("net:{}", n.name), (n.recv_rate + n.send_rate) as f32);
        }
        if let Some(g) = &s.gpu {
            for a in &g.adapters {
                add(format!("gpu:{}", a.name), a.util);
            }
        }
        for (i, c) in s.cores.iter().enumerate() {
            add(format!("core:{i}"), *c);
        }
    }

    fn hist_of(&self, key: &str) -> Vec<f32> {
        self.perf_hist.get(key).map(|q| q.iter().copied().collect()).unwrap_or_default()
    }

    fn graph_max(&self, it: &PerfItem, vals: &[f32]) -> f32 {
        if it.max > 0.0 {
            it.max
        } else {
            (vals.iter().copied().fold(0.0f32, f32::max) * 1.25).max(it.floor)
        }
    }

    /// The selected resource (the first one when nothing or something gone is selected).
    pub fn perf_index(&self, items: &[PerfItem]) -> usize {
        items.iter().position(|i| i.key == self.perf_sel).unwrap_or(0)
    }

    /// Up / Down on the page.
    pub fn perf_move(&mut self, d: i32) {
        let items = self.perf_items();
        if items.is_empty() {
            return;
        }
        let i = (self.perf_index(&items) as i32 + d).clamp(0, items.len() as i32 - 1) as usize;
        self.perf_sel = items[i].key.clone();
        self.set_scroll(2, 0.0);
    }

    pub fn paint_performance(&mut self) {
        let t = self.t;
        let layer = Rect::new(self.cx0(), TITLE_H, self.w - self.cx0() - 12.0, self.h - TITLE_H - 12.0);
        self.gfx.card(layer, 8.0, t.layer, t.layer_s);
        self.gfx.clip(layer.inset(1.0, 1.0));
        let items = self.perf_items();
        let sel = self.perf_index(&items);
        let narrow = layer.w < 760.0;
        let detail;
        if !narrow {
            // left: the resource list with a mini graph each
            let lw = 252.0;
            let mut y = layer.y + 10.0;
            for (i, it) in items.iter().enumerate() {
                let r = Rect::new(layer.x + 8.0, y, lw, 58.0);
                let h = Hit::PerfItem(i as u8);
                if i == sel {
                    self.gfx.fill_round(r, 4.0, t.sel);
                    self.gfx.fill_round(Rect::new(r.x, r.cy() - 9.0, 3.0, 18.0), 1.5, t.acc);
                } else {
                    self.hp(r, 4.0, h);
                }
                self.gfx.text(&it.title, Rect::new(r.x + 14.0, r.y + 9.0, lw - 14.0 - 88.0, 18.0), Font::Ui, 13.0, 600, Align::Left, t.t1);
                self.gfx.text(&it.sub, Rect::new(r.x + 14.0, r.y + 30.0, lw - 14.0 - 88.0, 17.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
                let vals = self.hist_of(&it.key);
                let gr = Rect::new(r.r() - 80.0, r.y + 12.0, 70.0, 34.0);
                self.gfx.stroke_round(gr, 2.0, t.line, 1.0);
                let mx = self.graph_max(it, &vals);
                self.gfx.spark(gr.inset(1.0, 1.0), &vals, mx, it.color, 0.16, 1.2);
                self.tip(r, format!("{}\n{}", it.title, it.sub));
                self.hit(r, h);
                y += 62.0;
                if y > layer.b() - 58.0 {
                    break;
                }
            }
            self.gfx.vline(layer.x + 8.0 + lw + 10.0, layer.y + 10.0, layer.b() - 10.0, t.line);
            detail = Rect::new(layer.x + lw + 38.0, layer.y, layer.r() - (layer.x + lw + 38.0) - 20.0, layer.h);
        } else {
            // narrow: the resources as chips that wrap onto more lines
            let (mut x, mut y) = (layer.x + 12.0, layer.y + 12.0);
            for (i, it) in items.iter().enumerate() {
                let label = format!("{}  {}", it.title, it.sub.rsplit(" · ").next().unwrap_or(&it.sub));
                let w = (self.gfx.measure(&label, Font::Ui, 12.5, 400) + 24.0).min(layer.w - 24.0);
                if x + w > layer.r() - 12.0 && x > layer.x + 12.0 {
                    x = layer.x + 12.0;
                    y += 36.0;
                }
                let r = Rect::new(x, y, w, 30.0);
                let h = Hit::PerfItem(i as u8);
                if i == sel {
                    self.gfx.fill_round(r, 15.0, t.acc);
                } else {
                    self.gfx.fill_round(r, 15.0, t.tag);
                    self.hp(r, 15.0, h);
                }
                self.gfx.text(&label, r.inset(12.0, 0.0), Font::Ui, 12.5, 400, Align::Center, if i == sel { t.on_acc } else { t.t1 });
                self.tip(r, format!("{}\n{}", it.title, it.sub));
                self.hit(r, h);
                x += w + 6.0;
            }
            y += 30.0 + 10.0;
            self.gfx.hline(layer.x + 12.0, layer.r() - 12.0, y, t.line);
            detail = Rect::new(layer.x + 16.0, y + 1.0, layer.w - 32.0, layer.b() - y - 1.0);
        }
        if let Some(it) = items.get(sel) {
            self.gfx.clip(detail);
            let h = self.paint_perf_detail(it, detail);
            self.gfx.unclip();
            self.content_h[2] = h;
            self.view_h[2] = detail.h;
            self.clamp_scroll();
            self.scrollbar(detail.off(16.0, 0.0), 2);
        }
        self.gfx.unclip();
    }

    /// The right side for one resource. Returns its content height (for scrolling).
    fn paint_perf_detail(&mut self, it: &PerfItem, d: Rect) -> f32 {
        let t = self.t;
        let s = self.perf.clone();
        let y0 = d.y + 18.0 - self.scroll[2];
        let mut y = y0;
        // what it is: name and device
        let (device, big, big_sub) = self.perf_head(it, &s);
        let tw = self.gfx.text_at(&it.title, d.x, y, 28.0, d.w * 0.5, Font::Display, 20.0, 600, t.t1);
        self.gfx.text_at(&device, d.x + tw + 12.0, y + 3.0, 28.0, (d.w - tw - 12.0).max(0.0), Font::Ui, 13.0, 400, t.t2);
        y += 34.0;
        let bw = self.gfx.text_at(&big, d.x, y, 36.0, d.w, Font::Display, 28.0, 600, t.t1);
        self.gfx.text_at(&big_sub, d.x + bw + 8.0, y + 6.0, 30.0, (d.w - bw - 8.0).max(0.0), Font::Ui, 13.0, 400, t.t2);
        y += 44.0;
        // the large graph: the last 60 s
        let vals = self.hist_of(&it.key);
        let mx = self.graph_max(it, &vals);
        let gh = (d.h * 0.3).clamp(120.0, 220.0);
        let gr = Rect::new(d.x, y, d.w, gh);
        for f in [0.25f32, 0.5, 0.75] {
            self.gfx.hline(gr.x, gr.r(), (gr.y + gr.h * f).round(), t.line);
        }
        self.gfx.stroke_round(gr, 0.0, t.line, 1.0);
        self.gfx.spark(gr.inset(1.0, 1.0), &vals, mx, it.color, 0.16, 1.6);
        let max_label = match it.key.as_str() {
            "mem" => model::fmt_gb(mx as u64),
            k if k.starts_with("net:") => model::fmt_rate(mx as f64),
            _ => "100%".into(),
        };
        self.gfx.text(&max_label, Rect::new(gr.r() - 104.0, gr.y + 3.0, 100.0, 14.0), Font::Ui, 11.0, 400, Align::Right, t.t3);
        self.gfx.text("60 s", Rect::new(gr.x, gr.b() + 4.0, 60.0, 16.0), Font::Ui, 11.0, 400, Align::Left, t.t3);
        self.gfx.text("now", Rect::new(gr.r() - 60.0, gr.b() + 4.0, 60.0, 16.0), Font::Ui, 11.0, 400, Align::Right, t.t3);
        y = gr.b() + 28.0;
        // CPU: one small graph per logical processor
        if it.key == "cpu" && !s.cores.is_empty() {
            let cols = ((d.w + 8.0) / 92.0).floor().clamp(2.0, 8.0) as usize;
            let cw = (d.w - (cols as f32 - 1.0) * 8.0) / cols as f32;
            for (i, _) in s.cores.iter().enumerate() {
                let r = Rect::new(d.x + (i % cols) as f32 * (cw + 8.0), y + (i / cols) as f32 * 42.0, cw, 34.0);
                self.gfx.stroke_round(r, 2.0, t.line, 1.0);
                let cv = self.hist_of(&format!("core:{i}"));
                self.gfx.spark(r.inset(1.0, 1.0), &cv, 100.0, it.color, 0.14, 1.0);
                self.gfx.text(&format!("{i}"), Rect::new(r.x + 4.0, r.y + 2.0, 30.0, 12.0), Font::Ui, 10.0, 400, Align::Left, t.t3);
            }
            y += s.cores.len().div_ceil(cols) as f32 * 42.0 + 14.0;
        }
        // GPU: the main engines with a bar each, the others in one small line
        if let Some(a) = it.key.strip_prefix("gpu:").and_then(|n| s.gpu.as_ref().and_then(|g| g.adapters.iter().find(|a| a.name == n))).cloned() {
            self.gfx.text_at("Engines", d.x, y, 20.0, 200.0, Font::Ui, 13.0, 600, t.t1);
            y += 26.0;
            for (name, v) in a.engines.iter().take(6) {
                self.gfx.text_at(name, d.x, y, 22.0, 140.0, Font::Ui, 12.5, 400, t.t2);
                let br = Rect::new(d.x + 150.0, y + 8.0, (d.w - 150.0 - 56.0).max(40.0), 6.0);
                self.gfx.fill_round(br, 3.0, t.track);
                self.gfx.fill_round(Rect::new(br.x, br.y, (br.w * (v / 100.0).clamp(0.0, 1.0)).max(3.0), 6.0), 3.0, it.color);
                self.gfx.text(&format!("{v:.0}%"), Rect::new(br.r() + 8.0, y, 48.0, 22.0), Font::Ui, 12.5, 400, Align::Right, t.t1);
                y += 26.0;
            }
            if !a.other.is_empty() {
                let o = a.other.iter().map(|(n, v)| format!("{n} {v:.0}%")).collect::<Vec<_>>().join(" · ");
                y += self.gfx.para(&format!("Also: {o}"), Rect::new(d.x, y + 2.0, d.w, 60.0), Font::Ui, 11.5, 400, Align::Left, t.t3, 16.0) + 6.0;
            }
            y += 14.0;
        }
        // facts: "used of total" with units, two columns when there is room
        let facts = self.perf_facts(it, &s);
        let two = d.w >= 520.0;
        let cw = if two { (d.w - 24.0) / 2.0 } else { d.w };
        for (i, (k, v)) in facts.iter().enumerate() {
            let (cx, cy) = if two { (d.x + (i % 2) as f32 * (cw + 24.0), y + (i / 2) as f32 * 32.0) } else { (d.x, y + i as f32 * 32.0) };
            self.gfx.hline(cx, cx + cw, cy, t.line);
            self.gfx.text(k, Rect::new(cx, cy + 8.0, cw * 0.42, 17.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
            self.gfx.text(v, Rect::new(cx + cw * 0.42, cy + 8.0, cw * 0.58, 17.0), Font::Ui, 12.5, 400, Align::Right, t.t1);
            self.tip(Rect::new(cx, cy, cw, 32.0), format!("{k}: {v}"));
        }
        let rows = if two { facts.len().div_ceil(2) } else { facts.len() };
        y += rows as f32 * 32.0 + 22.0;
        // the processes that use it most (click one to jump to it in the list)
        let (head, top) = self.perf_top(it);
        self.gfx.text_at(&head, d.x, y, 20.0, d.w, Font::Ui, 13.0, 600, t.t1);
        y += 26.0;
        self.perf_top_keys.clear();
        if top.is_empty() {
            let msg = if it.key.starts_with("net:") { "Windows has no per-app network counter without administrator rights." } else { "Nothing uses it right now." };
            self.gfx.text_at(msg, d.x, y, 20.0, d.w, Font::Ui, 12.5, 400, t.t3);
            y += 28.0;
        }
        for (i, (key, label, path, val)) in top.into_iter().enumerate() {
            let r = Rect::new(d.x - 6.0, y, d.w + 12.0, 32.0);
            let h = Hit::PerfTop(i as u8);
            self.hp(r, 4.0, h);
            self.draw_icon(&path, Rect::new(d.x, y + 8.0, 16.0, 16.0), 0xE737);
            let vw = self.gfx.measure(&val, Font::Ui, 12.5, 400);
            self.gfx.text(&label, Rect::new(d.x + 26.0, y, d.w - 26.0 - vw - 12.0, 32.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
            self.gfx.text(&val, Rect::new(d.r() - vw - 1.0, y, vw + 1.0, 32.0), Font::Ui, 12.5, 400, Align::Right, t.t1);
            self.tip(r, format!("{label}: {val}\nClick to show it in Processes"));
            self.hit(r, h);
            self.perf_top_keys.push(key);
            y += 32.0;
        }
        y - y0 + 30.0
    }

    /// (device name, big value, small text next to it)
    fn perf_head(&self, it: &PerfItem, s: &crate::perf::Snapshot) -> (String, String, String) {
        let used = s.mem_total.saturating_sub(s.mem_avail);
        match it.key.as_str() {
            "cpu" => (s.cpu_name.clone(), format!("{:.0}%", s.cpu), format!("at {}", ghz(if s.cur_mhz > 0 { s.cur_mhz } else { s.base_mhz }))),
            "mem" => (String::new(), model::fmt_gb(used), format!("of {} in use ({:.0}%)", model::fmt_gb(s.mem_total), pct_of(used, s.mem_total))),
            k if k.starts_with("disk:") => {
                let d = s.disks.iter().find(|d| format!("disk:{}", d.name) == k);
                (d.map(|d| d.model.clone()).unwrap_or_default(), format!("{:.0}%", d.map(|d| d.active).unwrap_or(0.0)), "active time".into())
            }
            k if k.starts_with("net:") => {
                let n = s.nets.iter().find(|n| format!("net:{}", n.name) == k);
                let (dn, up) = n.map(|n| (n.recv_rate, n.send_rate)).unwrap_or((0.0, 0.0));
                (n.map(|n| n.desc.clone()).unwrap_or_default(), model::fmt_rate(dn + up), format!("↓ {}  ↑ {}", model::fmt_rate(dn), model::fmt_rate(up)))
            }
            k => {
                let a = s.gpu.as_ref().and_then(|g| g.adapters.iter().find(|a| format!("gpu:{}", a.name) == k));
                (a.map(|a| a.name.clone()).unwrap_or_default(), format!("{:.0}%", a.map(|a| a.util).unwrap_or(0.0)), "busiest engine".into())
            }
        }
    }

    /// "Used of total" facts for a resource.
    fn perf_facts(&self, it: &PerfItem, s: &crate::perf::Snapshot) -> Vec<(String, String)> {
        let n = |x: u32| {
            let s = x.to_string();
            let mut out = String::new();
            for (i, ch) in s.chars().enumerate() {
                if i > 0 && (s.len() - i) % 3 == 0 {
                    out.push(',');
                }
                out.push(ch);
            }
            out
        };
        let f = |k: &str, v: String| (k.to_string(), v);
        let used = s.mem_total.saturating_sub(s.mem_avail);
        match it.key.as_str() {
            "cpu" => vec![
                f("Utilisation", format!("{:.0}%", s.cpu)),
                f("Speed", if s.cur_mhz > 0 { format!("{} (base {})", ghz(s.cur_mhz), ghz(s.base_mhz)) } else { format!("base {}", ghz(s.base_mhz)) }),
                f("Cores", format!("{} cores, {} threads", s.phys_cores, s.cores.len())),
                f("Processes", n(s.processes)),
                f("Threads", n(s.threads)),
                f("Handles", n(s.handles)),
                f("Up time", model::fmt_dur(s.uptime_s)),
            ],
            "mem" => {
                let compressed = self.procs.iter().find(|p| p.name.eq_ignore_ascii_case("Memory Compression")).map(|p| p.ws).unwrap_or(0);
                vec![
                    f("In use", format!("{} of {} ({:.0}%)", model::fmt_gb(used), model::fmt_gb(s.mem_total), pct_of(used, s.mem_total))),
                    f("Available", model::fmt_gb(s.mem_avail)),
                    f("Committed", format!("{} of {}", model::fmt_gb(s.commit_total), model::fmt_gb(s.commit_limit))),
                    f("Cached", model::fmt_gb(s.cached)),
                    f("Paged pool", model::fmt_mem(s.paged_pool)),
                    f("Non-paged pool", model::fmt_mem(s.nonpaged_pool)),
                    f("Compressed", model::fmt_mem(compressed)),
                ]
            }
            k if k.starts_with("disk:") => {
                let Some(d) = s.disks.iter().find(|d| format!("disk:{}", d.name) == k) else { return vec![] };
                vec![
                    f("Active time", format!("{:.0}%", d.active)),
                    f("Read", model::fmt_rate(d.read_rate)),
                    f("Write", model::fmt_rate(d.write_rate)),
                    f("Capacity", if d.total > 0 { format!("{} used of {}", model::fmt_gb(d.total.saturating_sub(d.free)), model::fmt_gb(d.total)) } else { "–".into() }),
                    f("Type", match d.ssd { Some(true) => "SSD".into(), Some(false) => "HDD".into(), None => "–".into() }),
                    f("Drives", if d.letters.is_empty() { "–".into() } else { d.letters.clone() }),
                ]
            }
            k if k.starts_with("net:") => {
                let Some(nt) = s.nets.iter().find(|x| format!("net:{}", x.name) == k) else { return vec![] };
                let link = if nt.link_bps >= 1_000_000_000 { format!("{:.1} Gbps", nt.link_bps as f64 / 1e9) } else { format!("{:.0} Mbps", nt.link_bps as f64 / 1e6) };
                vec![
                    f("Down", model::fmt_rate(nt.recv_rate)),
                    f("Up", model::fmt_rate(nt.send_rate)),
                    f("Link speed", link),
                    f("IPv4", if nt.ipv4.is_empty() { "–".into() } else { nt.ipv4.clone() }),
                    f("Adapter", nt.desc.clone()),
                ]
            }
            k => {
                let Some(a) = s.gpu.as_ref().and_then(|g| g.adapters.iter().find(|a| format!("gpu:{}", a.name) == k)) else { return vec![] };
                vec![
                    f("Utilisation", format!("{:.0}%", a.util)),
                    f("Dedicated memory", format!("{} of {}", model::fmt_mem(a.dedicated_used), model::fmt_gb(a.dedicated_total))),
                    f("Shared memory", format!("{} of {}", model::fmt_mem(a.shared_used), model::fmt_gb(a.shared_total))),
                    f("Driver", if a.driver.is_empty() { "–".into() } else { a.driver.clone() }),
                ]
            }
        }
    }

    /// The heading and the top 5 processes for a resource: (group key, label, exe path, value text).
    fn perf_top(&self, it: &PerfItem) -> (String, Vec<(String, String, String, String)>) {
        let k = it.key.as_str();
        let (head, val): (&str, fn(&model::Group) -> f64) = if k == "cpu" {
            ("Top CPU use", |g| g.cpu as f64)
        } else if k == "mem" {
            ("Top memory use", |g| g.mem as f64)
        } else if k.starts_with("disk:") {
            ("Top I/O (files, pipes and devices, all disks)", |g| g.io)
        } else if k.starts_with("gpu:") {
            ("Top GPU use (all adapters)", |g| g.gpu as f64)
        } else {
            return ("Top network use".into(), vec![]);
        };
        let mut v: Vec<&model::Group> = self.groups.iter().filter(|g| val(g) > 0.0).collect();
        v.sort_by(|a, b| val(b).total_cmp(&val(a)));
        let fmt = |g: &model::Group| -> String {
            let x = val(g);
            if k == "mem" {
                model::fmt_mem(x as u64)
            } else if k.starts_with("disk:") {
                model::fmt_rate(x)
            } else {
                model::fmt_pct(x as f32)
            }
        };
        (
            head.into(),
            v.into_iter()
                .take(5)
                .map(|g| {
                    let label = if g.members.len() > 1 { format!("{} ({})", g.label, g.members.len()) } else { g.label.clone() };
                    (g.key.clone(), label, g.path.clone(), fmt(g))
                })
                .collect(),
        )
    }
}
