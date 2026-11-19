//! Things drawn on top: the performance flyout (opened from the totals strip), menus, the confirm dialog, toasts.
use super::app::*;
use super::gfx::{Align, C, Font, Rect};
use super::model::{self, SortKey};
use super::motion;
use super::paint::Btn;
use std::time::Instant;

/// How long closing overlays take to fade out.
const LEAVE_MS: f32 = 140.0;

impl App {
    /// Notice overlays that were just closed and keep drawing them while they fade out.
    pub fn track_ghosts(&mut self) {
        if self.shooting {
            return;
        }
        let now = Instant::now();
        let mut started = false;
        {
            let mut mo = self.mo.borrow_mut();
            match (self.flyout, mo.last_flyout) {
                (None, Some((i, at))) => {
                    mo.ghost_flyout = Some((i, at, now));
                    started = true;
                }
                (Some(_), _) => mo.ghost_flyout = None,
                _ => {}
            }
            mo.last_flyout = self.flyout;
            if self.menu.is_none() {
                if let Some(m) = mo.last_menu.take() {
                    mo.ghost_menu = Some((m, now));
                    started = true;
                }
            } else {
                mo.ghost_menu = None;
                mo.last_menu = self.menu.clone();
            }
            if self.toast.is_none() {
                if let Some(t) = mo.last_toast.take() {
                    mo.ghost_toast = Some((t, now));
                    started = true;
                }
            } else {
                mo.ghost_toast = None;
                mo.last_toast = self.toast.clone();
            }
            if self.dialog.is_none() {
                if let Some(d) = mo.last_dialog.take() {
                    mo.ghost_dialog = Some((d, now));
                    started = true;
                }
            } else {
                mo.ghost_dialog = None;
                mo.last_dialog = self.dialog.clone();
            }
            if self.pane.is_none() {
                if let Some((p, w)) = mo.last_pane.take() {
                    mo.ghost_pane = Some((p, w, now));
                    started = true;
                }
            } else {
                mo.ghost_pane = None;
                let w = self.pane.as_ref().map(|p| self.pane_w(p.kind)).unwrap_or(0.0);
                mo.last_pane = self.pane.clone().map(|p| (p, w));
            }
        }
        if started {
            self.animate(LEAVE_MS as u64);
        }
    }

    /// Progress 0..1 of a closing overlay (None when it is gone).
    pub fn leaving(&self, at: Instant) -> Option<f32> {
        let p = at.elapsed().as_secs_f32() * 1000.0 / LEAVE_MS;
        if p >= 1.0 {
            return None;
        }
        self.animate(30);
        Some(motion::accel(p))
    }

    pub fn paint_overlays(&mut self) {
        if self.page == 0 {
            if let Some((i, at)) = self.flyout {
                self.paint_flyout(i, at, None);
            } else if let Some((i, at, closed)) = { self.mo.borrow().ghost_flyout } {
                self.ghost(|a| a.paint_flyout(i, at, Some(closed)));
            }
        }
        if self.page != 0 {
            let layer = Rect::new(12.0, TITLE_H, self.w - 24.0, self.h - TITLE_H - 12.0);
            self.paint_toast(layer);
        }
        if self.menu.is_some() {
            self.paint_menu(None);
        } else if let Some((m, closed)) = { self.mo.borrow().ghost_menu.clone() } {
            self.ghost(|a| {
                a.menu = Some(m);
                a.paint_menu(Some(closed));
                a.menu = None;
            });
        }
        if self.dialog.is_some() {
            self.paint_dialog(None);
        } else if let Some((d, closed)) = { self.mo.borrow().ghost_dialog.clone() } {
            self.ghost(|a| {
                a.dialog = Some(d);
                a.paint_dialog(Some(closed));
                a.dialog = None;
            });
        }
    }

    /// Draw something that is only fading out: it takes no clicks.
    pub fn ghost(&mut self, f: impl FnOnce(&mut App)) {
        let n = self.hits.len();
        let (hover, pressed) = (self.hover, self.pressed);
        self.hover = Hit::None;
        self.pressed = Hit::None;
        f(self);
        self.hover = hover;
        self.pressed = pressed;
        self.hits.truncate(n);
    }

    /// Opacity and vertical offset of a popup: opening = fade in + spring down from -8 px; closing = fade out + lift.
    fn pop_motion(&self, at: Instant, closed: Option<Instant>) -> Option<(f32, f32)> {
        match closed {
            None => {
                let a = self.tween_in(at, motion::NORMAL, motion::decel);
                let p = self.spring_in(at, motion::POP);
                Some((a, -(1.0 - p) * 8.0))
            }
            Some(c) => {
                let q = self.leaving(c)?;
                Some((1.0 - q, -q * 4.0))
            }
        }
    }

    fn fly_card(&self, r: Rect) {
        let t = self.t;
        self.gfx.shadow(r, 8.0, 8.0, 16.0, 0.13 * t.sh);
        self.gfx.shadow(r, 8.0, 1.0, 3.0, 0.06 * t.sh);
        self.gfx.card(r, 8.0, t.fly, t.fly_s);
    }

    // ------------------------------------------------------------------ performance flyout
    fn paint_flyout(&mut self, i: u8, at: Instant, closed: Option<Instant>) {
        let Some((a, dy)) = self.pop_motion(at, closed) else {
            self.mo.borrow_mut().ghost_flyout = None;
            return;
        };
        let old = self.gfx.push_alpha(a);
        self.gfx.push_move(0.0, dy);
        self.paint_flyout_body(i);
        self.gfx.pop_xf();
        self.gfx.pop_alpha(old);
    }

    fn paint_flyout_body(&mut self, i: u8) {
        let t = self.t;
        let top = TITLE_H + if self.prefs.tiles { TILES_H } else { 0.0 } + 6.0;
        let r = Rect::new(12.0, top, self.w - 24.0, 332.0);
        self.hit(r, Hit::Block);
        self.fly_card(r);
        let inner = r.inset(20.0, 18.0);
        let right_w = 232.0;
        let mid_w = 218.0;
        let left = Rect::new(inner.x, inner.y, inner.w - right_w - mid_w - 56.0, inner.h);
        let mid = Rect::new(left.r() + 28.0, inner.y, mid_w, inner.h);
        let right = Rect::new(mid.r() + 28.0, inner.y, right_w, inner.h);
        self.gfx.vline(right.x, inner.y, inner.b(), t.line);
        let right = Rect::new(right.x + 24.0, right.y, right.w - 24.0, right.h);
        let s = self.perf.clone();
        let used = s.mem_total.saturating_sub(s.mem_avail);
        let series: Vec<f32> = self.tot[i as usize].iter().copied().collect();
        let res = super::paint::RES_ALL[i as usize];
        let color = self.res_color(res);
        // left: title, big value, graph
        let (title, sub, big, small, max, max_label): (&str, String, String, String, f32, String) = match i {
            0 => ("CPU", s.cpu_name.clone(), format!("{:.0}%", s.cpu), "now".into(), 100.0, "100%".into()),
            1 => ("Memory", format!("{} usable", model::fmt_gb(s.mem_total)), model::fmt_gb(used), format!("in use · {:.0}%", used as f64 * 100.0 / s.mem_total.max(1) as f64), s.mem_total as f32, model::fmt_gb(s.mem_total)),
            2 => {
                let peak = series.iter().copied().fold(0.0f32, f32::max);
                let m = (peak * 1.25).max(1048576.0);
                let act = s.disks.iter().map(|d| d.active).fold(0.0f32, f32::max);
                ("Disk", format!("{} disk{}", s.disks.len(), if s.disks.len() == 1 { "" } else { "s" }), format!("{act:.0}%"), format!("active · {}", model::fmt_rate(series.last().copied().unwrap_or(0.0) as f64)), m, model::fmt_rate(m as f64))
            }
            3 => {
                let peak = series.iter().copied().fold(0.0f32, f32::max);
                let m = (peak * 1.25).max(100.0 * 1024.0);
                let now = series.last().copied().unwrap_or(0.0) as f64;
                let desc = s.nets.iter().max_by(|a, b| (a.recv_rate + a.send_rate).total_cmp(&(b.recv_rate + b.send_rate))).map(|n| n.desc.clone()).unwrap_or_default();
                ("Network", desc, if now < 1.0 { "0 KB/s".into() } else { model::fmt_rate(now) }, "now".into(), m, model::fmt_rate(m as f64))
            }
            _ => ("GPU", self.gpu_names.join(" · "), format!("{:.0}%", s.gpu.as_ref().map(|g| g.util).unwrap_or(0.0)), "now".into(), 100.0, "100%".into()),
        };
        let g = &self.gfx;
        let tw = g.text_at(title, left.x, left.y, 20.0, 120.0, Font::Ui, 14.0, 600, t.t1);
        g.text_at(&sub, left.x + tw + 10.0, left.y + 1.0, 20.0, left.w - tw - 10.0, Font::Ui, 12.0, 400, t.t2);
        let bw = g.text_at(&big, left.x, left.y + 22.0, 36.0, 200.0, Font::Display, 28.0, 600, t.t1);
        g.text_at(&small, left.x + bw + 6.0, left.y + 26.0, 36.0, 260.0, Font::Ui, 13.0, 400, t.t2);
        let gw = left.w.min(520.0);
        let gr = Rect::new(left.x, left.y + 66.0, gw, 190.0);
        for f in [0.25f32, 0.5, 0.75] {
            g.hline(gr.x, gr.r(), (gr.y + gr.h * f).round(), t.line);
        }
        g.stroke_round(gr, 0.0, t.line, 1.0);
        g.spark(gr, &series, max, color, 0.14, 1.5);
        // scale labels
        g.text(&max_label, Rect::new(gr.r() - 104.0, gr.y + 2.0, 100.0, 14.0), Font::Ui, 11.0, 400, Align::Right, t.t3);
        g.text("0", Rect::new(gr.r() - 104.0, gr.b() - 16.0, 100.0, 14.0), Font::Ui, 11.0, 400, Align::Right, t.t3);
        g.text("60 s", Rect::new(gr.x, gr.b() + 4.0, 60.0, 16.0), Font::Ui, 11.0, 400, Align::Left, t.t3);
        g.text("now", Rect::new(gr.r() - 60.0, gr.b() + 4.0, 60.0, 16.0), Font::Ui, 11.0, 400, Align::Right, t.t3);
        // middle
        let mut stats: Vec<(String, String)> = vec![];
        let mut my = mid.y + 2.0;
        match i {
            0 => {
                let n = s.cores.len();
                self.gfx.text_at(&format!("{n} logical processors"), mid.x, my, 16.0, mid.w, Font::Ui, 12.0, 400, t.t2);
                my += 24.0;
                let per_row = 8usize;
                let rows = n.div_ceil(per_row).max(1);
                let ch = if rows > 2 { (102.0 - (rows as f32 - 1.0) * 6.0) / rows as f32 } else { 46.0 };
                for (k, v) in s.cores.iter().enumerate() {
                    let cr = Rect::new(mid.x + (k % per_row) as f32 * 28.0, my + (k / per_row) as f32 * (ch + if rows > 2 { 6.0 } else { 10.0 }), 22.0, ch);
                    self.gfx.fill_round(cr, 3.0, t.track);
                    let fh = cr.h * (v / 100.0).clamp(0.0, 1.0);
                    if fh > 0.5 {
                        self.gfx.clip(cr);
                        self.gfx.fill_round(Rect::new(cr.x, cr.b() - fh, cr.w, fh), 3.0, t.cpu);
                        self.gfx.fill(Rect::new(cr.x, cr.b() - fh, cr.w, (fh - 3.0).max(0.0).min(3.0)), t.cpu);
                        self.gfx.unclip();
                    }
                }
                my += rows as f32 * ch + (rows as f32 - 1.0) * if rows > 2 { 6.0 } else { 10.0 } + 18.0;
                let life: f64 = {
                    let busy: u64 = self.procs.iter().filter(|p| p.pid != 0).map(|p| p.cpu_time).sum();
                    busy as f64 / (s.uptime_s.max(1) as f64 * 1e7 * n.max(1) as f64) * 100.0
                };
                stats.push(("Up time".into(), model::fmt_dur(s.uptime_s)));
                stats.push(("Processes".into(), s.processes.to_string()));
                stats.push(("Since boot".into(), format!("avg {life:.0}%")));
            }
            1 => {
                self.gfx.text_at("Composition", mid.x, my, 16.0, mid.w, Font::Ui, 12.0, 400, t.t2);
                my += 24.0;
                let bar = Rect::new(mid.x, my, mid.w, 10.0);
                self.gfx.fill_round(bar, 5.0, t.track);
                let tot = s.mem_total.max(1) as f32;
                let uw = mid.w * used as f32 / tot;
                let cw = (mid.w * s.cached as f32 / tot).min(mid.w - uw - 2.0).max(0.0);
                self.gfx.fill_round(Rect::new(bar.x, bar.y, uw, 10.0), 5.0, t.mem);
                self.gfx.fill(Rect::new(bar.x + uw - 5.0, bar.y, 5.0, 10.0), t.mem);
                if cw > 1.0 {
                    self.gfx.fill(Rect::new(bar.x + uw + 2.0, bar.y, cw, 10.0), t.mem.fade(0.4));
                }
                my += 18.0;
                let mut lx = mid.x;
                for (label, c) in [("In use", t.mem), ("Cached", t.mem.fade(0.4)), ("Free", t.track)] {
                    self.gfx.fill_round(Rect::new(lx, my + 4.0, 8.0, 8.0), 2.0, c);
                    let w = self.gfx.text_at(label, lx + 13.0, my, 16.0, 80.0, Font::Ui, 11.5, 400, t.t2);
                    lx += 13.0 + w + 14.0;
                }
                my += 30.0;
                let compressed = self.procs.iter().find(|p| p.name.eq_ignore_ascii_case("Memory Compression")).map(|p| p.ws).unwrap_or(0);
                stats.push(("Available".into(), model::fmt_gb(s.mem_avail)));
                stats.push(("Cached".into(), model::fmt_gb(s.cached)));
                stats.push(("Reserved for apps".into(), format!("{:.0} of {:.0} GB", s.commit_total as f64 / 1073741824.0, s.commit_limit as f64 / 1073741824.0)));
                stats.push(("Compressed".into(), model::fmt_mem(compressed)));
            }
            2 => {
                let rd: f64 = s.disks.iter().map(|d| d.read_rate).sum();
                let wr: f64 = s.disks.iter().map(|d| d.write_rate).sum();
                stats.push(("Read".into(), if rd < 1.0 { "0 KB/s".into() } else { model::fmt_rate(rd) }));
                stats.push(("Write".into(), if wr < 1.0 { "0 KB/s".into() } else { model::fmt_rate(wr) }));
                for d in s.disks.iter().take(4) {
                    stats.push((format!("{} active", d.name), format!("{:.0}%", d.active)));
                }
            }
            3 => {
                let (dn, up): (f64, f64) = (s.nets.iter().map(|x| x.recv_rate).sum(), s.nets.iter().map(|x| x.send_rate).sum());
                stats.push(("Down".into(), if dn < 1.0 { "0 KB/s".into() } else { model::fmt_rate(dn) }));
                stats.push(("Up".into(), if up < 1.0 { "0 KB/s".into() } else { model::fmt_rate(up) }));
                if let Some(n) = s.nets.iter().max_by_key(|n| n.link_bps) {
                    stats.push(("Link speed".into(), if n.link_bps >= 1_000_000_000 { format!("{:.1} Gbps", n.link_bps as f64 / 1e9) } else { format!("{:.0} Mbps", n.link_bps as f64 / 1e6) }));
                }
                stats.push(("Adapters".into(), s.nets.len().to_string()));
            }
            _ => {
                if let Some(gp) = &s.gpu {
                    for (name, v) in gp.engines.iter().take(3) {
                        stats.push((name.clone(), format!("{v:.0}%")));
                    }
                    stats.push(("Dedicated memory".into(), model::fmt_mem(gp.dedicated_used)));
                    stats.push(("Shared memory".into(), model::fmt_mem(gp.shared_used)));
                } else {
                    stats.push(("GPU counters".into(), "Starting…".into()));
                }
            }
        }
        for (k, v) in &stats {
            self.gfx.hline(mid.x, mid.r(), my, t.line);
            self.gfx.text(k, Rect::new(mid.x, my + 7.0, mid.w - 90.0, 17.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
            self.gfx.text(v, Rect::new(mid.r() - 120.0, my + 7.0, 120.0, 17.0), Font::Ui, 12.5, 400, Align::Right, t.t1);
            my += 31.0;
        }
        // right: top now (+ since boot for CPU)
        let mut ry = right.y + 2.0;
        if i == 3 {
            self.gfx.text_at("Adapters", right.x, ry, 16.0, right.w, Font::Ui, 12.0, 400, t.t2);
            ry += 24.0;
            for n in s.nets.iter().take(5) {
                self.gfx.glyph(0xE839, Rect::new(right.x, ry, 16.0, 30.0), 14.0, t.t2);
                let v = model::fmt_rate(n.recv_rate + n.send_rate);
                let vw = self.gfx.measure(&v, Font::Ui, 13.0, 400);
                self.gfx.text(&n.name, Rect::new(right.x + 26.0, ry, right.w - 26.0 - vw - 8.0, 30.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
                self.gfx.text(&v, Rect::new(right.r() - vw - 1.0, ry, vw + 1.0, 30.0), Font::Ui, 13.0, 400, Align::Right, t.t1);
                ry += 30.0;
            }
        } else {
            self.gfx.text_at("Top now", right.x, ry, 16.0, right.w, Font::Ui, 12.0, 400, t.t2);
            ry += 24.0;
            let mut gs: Vec<(String, String, f64, usize)> = self
                .groups
                .iter()
                .map(|g| {
                    let v = match i {
                        0 => g.cpu as f64,
                        1 => g.mem as f64,
                        2 => g.io,
                        _ => g.gpu as f64,
                    };
                    (g.label.clone(), g.path.clone(), v, g.members.len())
                })
                .filter(|x| x.2 > 0.0)
                .collect();
            gs.sort_by(|a, b| b.2.total_cmp(&a.2));
            for (label, path, v, n) in gs.into_iter().take(4) {
                let vs = match i {
                    0 => model::fmt_pct(v as f32),
                    1 => model::fmt_mem(v as u64),
                    2 => model::fmt_rate(v),
                    _ => format!("{v:.0}%"),
                };
                let name = if n > 1 { format!("{label} ({n})") } else { label };
                self.trow(right, ry, &path, &name, &vs);
                ry += 30.0;
            }
            if i == 0 {
                ry += 16.0;
                self.gfx.text_at("Top since boot", right.x, ry, 16.0, right.w, Font::Ui, 12.0, 400, t.t2);
                ry += 24.0;
                let mut gs: Vec<(String, String, u64, usize)> = self.groups.iter().map(|g| (g.label.clone(), g.path.clone(), g.cpu_time, g.members.len())).collect();
                gs.sort_by(|a, b| b.2.cmp(&a.2));
                for (label, path, ct, n) in gs.into_iter().filter(|x| !x.0.eq_ignore_ascii_case("System")).take(2) {
                    let name = if n > 1 { format!("{label} ({n})") } else { label };
                    self.trow(right, ry, &path, &name, &format!("{} min", ct / 600_000_000));
                    ry += 30.0;
                }
            }
            if i == 1 {
                let br = Rect::new(right.x, right.b() - 32.0, right.w, 32.0);
                self.button(br, "Free up memory", 0xEA99, Btn::Std, false, true, Hit::FlyFree);
            }
        }
    }

    fn trow(&mut self, right: Rect, y: f32, path: &str, name: &str, v: &str) {
        let t = self.t;
        self.draw_icon(path, Rect::new(right.x, y + 7.0, 16.0, 16.0), 0xE737);
        let vw = self.gfx.measure(v, Font::Ui, 13.0, 400);
        self.gfx.text(name, Rect::new(right.x + 26.0, y, right.w - 26.0 - vw - 8.0, 30.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
        self.gfx.text(v, Rect::new(right.r() - vw - 1.0, y, vw + 1.0, 30.0), Font::Ui, 13.0, 400, Align::Right, t.t1);
    }

    // ------------------------------------------------------------------ menus
    fn paint_menu(&mut self, closed: Option<Instant>) {
        let t = self.t;
        let Some(m) = &self.menu else { return };
        let (kind, mx, my, at) = (m.kind, m.x, m.y, m.at);
        // (glyph, label, shortcut, separator before)
        let items: Vec<(u32, String, String, bool, bool)> = match kind {
            MenuKind::Row => {
                let sys = self.sel_group().map(|g| g.kind == crate::procs::Kind::Windows).unwrap_or(false);
                vec![
                    (0xE711, "End task".into(), "Del".into(), false, true),
                    (0, "End all with this name".into(), String::new(), false, true),
                    (0xE72C, "Restart".into(), String::new(), false, !sys),
                    (0xE9CE, "What is this?".into(), "F1".into(), true, true),
                    (0xE8FD, "Why is this running?".into(), String::new(), false, true),
                    (0xE838, "Open file location".into(), String::new(), true, true),
                    (0xE946, "Properties".into(), "Alt+Enter".into(), false, true),
                    (0xE756, "Copy as command".into(), "Ctrl+Shift+C".into(), false, true),
                ]
            }
            MenuKind::Sort => SortKey::MENU.iter().map(|s| (if *s == self.sort { 0xE73E } else { 0 }, s.label().to_string(), String::new(), false, true)).collect(),
            MenuKind::StartupSort => super::pages::STARTUP_SORT.iter().enumerate().map(|(i, s)| (if i as u8 == self.startup_sort { 0xE73E } else { 0 }, s.to_string(), String::new(), false, true)).collect(),
            MenuKind::Chips => self.chips().into_iter().skip(3).map(|c| (c.glyph, c.text, String::new(), false, true)).collect(),
            MenuKind::DefSort => {
                let cur = model::SortKey::parse(&self.prefs.sort);
                SortKey::MENU.iter().map(|s| (if Some(*s) == cur { 0xE73E } else { 0 }, s.label().to_string(), String::new(), false, true)).collect()
            }
        };
        let mw = match kind {
            MenuKind::Row => 248.0,
            MenuKind::Chips => 320.0,
            _ => 180.0,
        };
        let mh = 8.0 + items.iter().map(|i| if i.3 { 9.0 + 32.0 } else { 32.0 }).sum::<f32>();
        let x = mx.min(self.w - mw - 8.0).max(8.0);
        let y = if my + mh > self.h - 8.0 { (my - mh).max(8.0) } else { my };
        let Some((alpha, dy)) = self.pop_motion(at, closed) else {
            self.mo.borrow_mut().ghost_menu = None;
            return;
        };
        let old = self.gfx.push_alpha(alpha);
        // menus grow from the point they were opened at
        let up = my + mh > self.h - 8.0;
        let r = Rect::new(x, y + if up { -dy } else { dy }, mw, mh);
        self.fly_card(r);
        let mut iy = r.y + 4.0;
        for (i, (gl, label, short, sep, enabled)) in items.iter().enumerate() {
            if *sep {
                self.gfx.hline(r.x + 1.0, r.r() - 1.0, iy + 4.0, t.line);
                iy += 9.0;
            }
            let ir = Rect::new(r.x + 4.0, iy, r.w - 8.0, 32.0);
            let h = Hit::MenuItem(i as u8);
            if *enabled {
                self.hp(ir, 4.0, h);
                self.hit(ir, h);
            }
            let fg = if *enabled { t.t1 } else { t.t3 };
            if *gl != 0 {
                let gc = if kind == MenuKind::Chips { t.warn } else { fg };
                self.gfx.glyph(*gl, Rect::new(ir.x + 12.0, ir.y, 16.0, 32.0), 14.0, gc);
            }
            self.gfx.text(label, Rect::new(ir.x + 40.0, ir.y, ir.w - 40.0 - 10.0, 32.0), Font::Ui, 13.0, 400, Align::Left, fg);
            if !short.is_empty() {
                self.gfx.text(short, Rect::new(ir.x + 120.0, ir.y, ir.w - 130.0, 32.0), Font::Ui, 12.0, 400, Align::Right, t.t3);
            }
            iy += 32.0;
        }
        self.hit(r, Hit::Block);
        self.gfx.pop_alpha(old);
    }

    // ------------------------------------------------------------------ confirm dialog
    fn paint_dialog(&mut self, closed: Option<Instant>) {
        let t = self.t;
        let Some(d) = &self.dialog else { return };
        let (title, body, job, at) = (d.title.clone(), d.body.clone(), d.job.clone(), d.at);
        // opening: smoke fades in, the dialog springs from 1.05 to 1; closing: both fade, the dialog shrinks a little
        let (a, s) = match closed {
            None => (self.tween_in(at, motion::NORMAL, motion::decel), 1.05 - 0.05 * self.spring_in(at, motion::POP)),
            Some(c) => match self.leaving(c) {
                Some(q) => (1.0 - q, 1.0 - 0.03 * q),
                None => {
                    self.mo.borrow_mut().ghost_dialog = None;
                    return;
                }
            },
        };
        self.gfx.fill(Rect::new(0.0, 0.0, self.w, self.h), t.smoke.fade(a));
        let old = self.gfx.push_alpha(a);
        self.hit(Rect::new(0.0, 0.0, self.w, self.h), Hit::Block);
        let w = 448.0;
        let bh = self.gfx.para_h(&body, w - 48.0, Font::Ui, 14.0, 400, 20.0);
        let ch = 24.0 + 28.0 + 12.0 + bh + 14.0 + 18.0 + 22.0;
        let h = ch + 80.0;
        let r = Rect::new(((self.w - w) / 2.0).round(), ((self.h - h) / 2.0).round(), w, h);
        self.gfx.push_scale(r.cx(), r.cy(), s);
        self.gfx.shadow(r, 8.0, 32.0, 64.0, 0.22 * t.sh);
        self.gfx.shadow(r, 8.0, 2.0, 21.0, 0.12 * t.sh);
        self.gfx.fill_round(r, 8.0, t.fly);
        // footer band
        let fr = Rect::new(r.x, r.y + ch, r.w, 80.0);
        self.gfx.clip(fr);
        self.gfx.fill_round(r, 8.0, t.foot);
        self.gfx.unclip();
        self.gfx.hline(r.x, r.r(), fr.y, t.line);
        self.gfx.stroke_round(r, 8.0, t.fly_s, 1.0);
        let x = r.x + 24.0;
        let mut y = r.y + 24.0;
        self.gfx.text(&title, Rect::new(x, y, w - 48.0, 28.0), Font::Display, 20.0, 600, Align::Left, t.t1);
        y += 28.0 + 12.0;
        self.gfx.para(&body, Rect::new(x, y, w - 48.0, 200.0), Font::Ui, 14.0, 400, Align::Left, t.t1, 20.0);
        y += bh + 14.0;
        let mut fx = x;
        if !job.icon.is_empty() {
            self.draw_icon(&job.icon, Rect::new(fx, y + 2.0, 14.0, 14.0), 0xE737);
            fx += 20.0;
        }
        let n = format!("{} process{}", job.count, if job.count == 1 { "" } else { "es" });
        let nw = self.gfx.text_at(&n, fx, y, 18.0, 200.0, Font::Ui, 12.5, 400, t.t2);
        if job.mem > 0 {
            self.gfx.text_at(&format!("Frees about {}", model::fmt_mem(job.mem)), fx + nw + 16.0, y, 18.0, 200.0, Font::Ui, 12.5, 400, t.t2);
        }
        let bw = (w - 48.0 - 8.0) / 2.0;
        let by = fr.y + 24.0;
        self.button(Rect::new(x, by, bw, 32.0), "End task", 0, Btn::Crit, false, true, Hit::DlgEnd);
        self.button(Rect::new(x + bw + 8.0, by, bw, 32.0), "Wait", 0, Btn::Std, false, true, Hit::DlgWait);
        // default focus on "Wait"
        self.gfx.stroke_round(Rect::new(x + bw + 8.0 - 3.0, by - 3.0, bw + 6.0, 38.0), 6.0, t.t1.fade(0.0), 2.0);
        self.gfx.pop_xf();
        self.gfx.pop_alpha(old);
    }

    // ------------------------------------------------------------------ toasts
    pub fn paint_toast(&mut self, layer: Rect) {
        if self.toast.is_some() {
            self.paint_toast_at(layer, None);
        } else if let Some((ts, closed)) = { self.mo.borrow().ghost_toast.clone() } {
            self.ghost(|a| {
                a.toast = Some(ts);
                a.paint_toast_at(layer, Some(closed));
                a.toast = None;
            });
        }
    }

    fn paint_toast_at(&mut self, layer: Rect, closed: Option<Instant>) {
        let t = self.t;
        let Some(ts) = &self.toast else { return };
        let (text, code, link, ok, at) = (ts.text.clone(), ts.code.clone(), ts.link.as_ref().map(|l| l.0.clone()), ts.ok, ts.at);
        // rises 12 px with a spring and fades in; leaves by sinking 6 px and fading out
        let (alpha, off) = match closed {
            None => (self.tween_in(at, motion::NORMAL, motion::decel), (1.0 - self.spring_in(at, motion::POP)) * 12.0),
            Some(c) => match self.leaving(c) {
                Some(q) => (1.0 - q, q * 6.0),
                None => {
                    self.mo.borrow_mut().ghost_toast = None;
                    return;
                }
            },
        };
        let old = self.gfx.push_alpha(alpha);
        let g = &self.gfx;
        let tw = g.measure(&text, Font::Ui, 13.0, 400);
        let cw = if code.is_empty() { 0.0 } else { g.measure(&code, Font::Mono, 12.0, 400) + 12.0 };
        let lw = link.as_ref().map(|l| g.measure(l, Font::Ui, 13.0, 600) + 12.0).unwrap_or(0.0);
        let w = (14.0 + 16.0 + 12.0 + tw + cw + lw + 12.0 + 1.0 + 6.0 + 32.0 + 6.0).ceil().min(layer.w - 40.0);
        let r = Rect::new((layer.cx() - w / 2.0).round(), layer.b() - 28.0 - 44.0 + off, w, 44.0);
        self.fly_card(r);
        self.hit(r, Hit::Block);
        let g = &self.gfx;
        g.glyph(if ok { 0xE930 } else { 0xE7BA }, Rect::new(r.x + 14.0, r.y, 16.0, 44.0), 16.0, if ok { t.ok } else { t.warn });
        let mut x = r.x + 14.0 + 16.0 + 12.0;
        let maxw = r.r() - 6.0 - 32.0 - 13.0 - lw - cw - x;
        let w1 = g.text_at(&text, x, r.y, 44.0, maxw, Font::Ui, 13.0, 400, t.t1);
        x += w1 + 12.0;
        if !code.is_empty() {
            g.text_at(&code, x, r.y + 1.0, 44.0, cw, Font::Mono, 12.0, 400, t.t2);
            x += cw;
        }
        if let Some(l) = &link {
            let lr = Rect::new(x - 4.0, r.y + 8.0, lw - 4.0, 28.0);
            let hv = self.hov(Hit::ToastLink);
            if hv > 0.0 {
                self.gfx.fill_round(lr, 4.0, t.acc_soft.fade(hv));
            }
            self.gfx.text(l, lr, Font::Ui, 13.0, 600, Align::Center, t.acc_t);
            self.hit(lr, Hit::ToastLink);
            x += lw;
        }
        let _ = x;
        self.gfx.vline(r.r() - 6.0 - 32.0 - 7.0, r.y + 12.0, r.y + 32.0, t.line);
        let cr = Rect::new(r.r() - 6.0 - 32.0, r.y + 6.0, 32.0, 32.0);
        self.hp(cr, 4.0, Hit::ToastClose);
        self.gfx.glyph(0xE711, cr, 10.0, t.t2);
        self.hit(cr, Hit::ToastClose);
        let _ = C(0.0, 0.0, 0.0, 0.0);
        self.gfx.pop_alpha(old);
    }
}
