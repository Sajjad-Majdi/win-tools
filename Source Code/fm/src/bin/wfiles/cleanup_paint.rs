//! Direct2D layout for Cleanup. Sizes are in DIPs.
use super::*;
use crate::fmt;
use crate::gfx::{Align, C, Font};
use crate::motion::{self, Spring};
use crate::paint::Btn;
use crate::theme::{R_CARD, R_CTRL, R_MENU, R_SMALL, Theme};

const TILE_H: f32 = 148.0;
const TILE_GAP: f32 = 12.0;
const TILE_STEP: f32 = TILE_H + TILE_GAP;
const PANEL_W: f32 = 440.0;
const PANEL_HEAD: f32 = 156.0;
const SCROLL_PAD: f32 = 100.0;
const BIN_GLYPH: u32 = 0xE74D;

fn hit(target: Target) -> Hit {
    Hit::Cleanup(target)
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    Rect::new(x, y, (a.r().min(b.r()) - x).max(0.0), (a.b().min(b.b()) - y).max(0.0))
}

fn fraction(a: u64, b: u64) -> f32 {
    (a as f32 / b.max(1) as f32).clamp(0.0, 1.0)
}

fn count(n: usize) -> String {
    fmt::thousands(n as u64)
}

fn items(n: usize) -> String {
    format!("{} item{}", count(n), if n == 1 { "" } else { "s" })
}

fn scroll_pos(f: &mut Follow, max: f32, instant: bool) -> (f32, bool) {
    if instant {
        f.set(f.target.clamp(0.0, max));
    } else {
        f.target = f.target.clamp(0.0, max);
        f.step();
    }
    (f.value.clamp(0.0, max), !f.settled())
}

impl App {
    pub fn paint_cleanup(&mut self, c: Rect, t: &Theme) {
        if c.w < 1.0 || c.h < 1.0 {
            return;
        }
        self.cleanup.reduce_motion.get_or_insert_with(reduced_motion);
        self.gfx.clip(c);
        if self.cleanup.phase == Phase::Idle {
            self.paint_cleanup_drives(c, t);
        } else {
            self.paint_cleanup_results(c, t);
        }
        self.gfx.unclip();
    }

    fn cleanup_motion(&self, at: Option<Instant>, spring: Spring) -> f32 {
        let k = at.map_or(1.0, |at| motion::since(at, spring, self.shooting));
        if k != 1.0 {
            self.anim();
        }
        k
    }

    fn cleanup_instant(&self) -> bool {
        self.shooting || self.cleanup.reduce_motion == Some(true)
    }

    fn cleanup_label(&mut self, s: &str, r: Rect, fs: f32, weight: u32, color: C) {
        self.gfx.text(s, r, Font::Ui, fs, weight, Align::Left, color);
    }

    fn cleanup_number(&mut self, n: &str, sub: &str, r: Rect, fs: f32, color: C, t: &Theme) {
        let width = self.gfx.measure(n, Font::Display, fs, 600).min(r.w);
        self.gfx.text(n, Rect::new(r.x, r.y, width + 2.0, r.h), Font::Display, fs, 600, Align::Left, color);
        if width + 7.0 < r.w {
            self.cleanup_label(sub, Rect::new(r.x + width + 7.0, r.y + 4.0, r.w - width - 7.0, r.h), if fs >= 30.0 { 14.0 } else { 12.5 }, 400, t.text2);
        }
    }

    fn cleanup_card(&self, r: Rect, amount: f32, t: &Theme) {
        let fill = cleanup_card_fill(amount, t);
        let stroke = t.stroke.mix(t.sel_stroke, amount * if t.dark { 0.60 } else { 0.80 });
        self.gfx.fill_round(r, R_CARD, fill.a(self.panel_alpha()));
        self.gfx.stroke_round(r, R_CARD, stroke, 1.0);
    }

    fn cleanup_badge(&mut self, group: usize, x: f32, y: f32, amount: f32, t: &Theme) {
        let r = Rect::new(x, y, 36.0, 36.0);
        if amount > 0.0 {
            let alpha = if t.dark { 0.12 } else { 0.09 };
            self.gfx.fill_round(r, R_CTRL, t.accent.a(alpha * amount));
        }
        let color = t.text2.mix(t.accent, amount);
        match group {
            0 => self.gfx.glyph(0xE823, r, 16.0, color),
            1 => self.gfx.glyph(0xE81E, r, 16.0, color),
            // The icon font has separate chevrons for the code symbol.
            2 => {
                self.gfx.glyph(0xE76B, Rect::new(r.cx() - 10.0, r.y, 10.0, r.h), 9.0, color);
                self.gfx.line(r.cx() - 1.5, r.cy() + 5.0, r.cx() + 1.5, r.cy() - 5.0, color, 1.3);
                self.gfx.glyph(0xE76C, Rect::new(r.cx(), r.y, 10.0, r.h), 9.0, color);
            }
            3 => self.gfx.glyph(GROUPS[group].2, r, 16.0, color),
            4 => self.gfx.glyph(0xF158, r, 16.0, color),
            _ => {
                self.gfx.glyph(0xE8A5, r, 16.0, color);
                self.gfx.glyph(0xE74B, Rect::new(r.x, r.y + 4.0, r.w, r.h), 7.0, color);
            }
        }
    }

    fn cleanup_circle(&mut self, r: Rect, on: bool, partial: bool, hover: bool, scale: f32, t: &Theme) {
        if on || partial {
            self.gfx.ellipse(r.cx(), r.cy(), 11.0 * scale, t.accent, true, 1.0);
            if partial {
                self.gfx.fill_round(Rect::new(r.cx() - 5.0, r.cy() - 1.0, 10.0, 2.0), 1.0, t.on_accent);
            } else {
                self.gfx.glyph(0xE73E, r, 12.0 * scale, t.on_accent);
            }
        } else {
            self.gfx.ellipse(r.cx(), r.cy(), 10.25, if hover { t.text2 } else { t.field_border }, false, 1.5);
        }
    }

    fn paint_cleanup_drives(&mut self, c: Rect, t: &Theme) {
        let drives = self.scan_drives();
        let scope = self.cleanup.scope(&drives);
        let columns = if c.w >= 472.0 { 2 } else { 1 };
        let width = ((c.w - 52.0) / columns as f32 + 6.0).min(300.0);
        let grid_w = columns as f32 * (width + 12.0) - 12.0;
        let rows = drives.len().div_ceil(columns);
        let content_h = 70.0 + rows as f32 * 88.0;
        self.cleanup.scroll_max = (content_h + 16.0 - c.h).max(0.0);
        let instant = self.cleanup_instant();
        let (scroll, moving) = scroll_pos(&mut self.cleanup.scroll, self.cleanup.scroll_max, instant);
        if moving { self.anim(); }
        let x = c.cx() - grid_w * 0.5;
        let y = (c.cy() - content_h * 0.5 - 16.0).max(c.y + 12.0) - scroll;
        self.cleanup_label("Drives to scan", Rect::new(x, y, grid_w, 20.0), 12.0, 600, t.text3);
        for (i, d) in drives.iter().enumerate() {
            let r = Rect::new(x + (i % columns) as f32 * (width + 12.0), y + 30.0 + (i / columns) as f32 * 88.0, width, 76.0);
            let on = scope.contains(&d.root);
            let h = hit(Drive(d.root.chars().next().unwrap_or('?')));
            let scaled = self.press_in(r, h);
            self.cleanup_card(r, f32::from(on), t);
            self.gfx.fill_round(r, R_CARD, t.hover.a(t.hover.3 * self.hk(h)));
            self.drive_slab(r.x + 14.0, r.cy() - 14.0, &d.root, d.kind, t);
            let x = r.x + 66.0;
            let w = (r.r() - 48.0 - x).max(0.0);
            self.cleanup_label(&drive_name(d), Rect::new(x, r.y + 10.0, w, 22.0), 13.0, 400, t.text);
            if d.total > 0 {
                self.cleanup_drive_bar(Rect::new(x, r.y + 36.0, w, 6.0), d, None, t);
                let text = format!("{} free of {}", fmt::size(d.free), fmt::size(d.total));
                self.cleanup_label(&text, Rect::new(x, r.y + 46.0, w, 18.0), 12.0, 400, t.text2);
            }
            self.cleanup_circle(Rect::new(r.r() - 36.0, r.cy() - 11.0, 22.0, 22.0), on, false, self.hov(h), 1.0, t);
            if scaled { self.gfx.pop_xf(); }
            self.hit(intersect(c, r), h);
        }
        let sy = y + 26.0 + rows as f32 * 88.0 + 10.0;
        let r = Rect::new(c.cx() - 75.0, sy, 150.0, 36.0);
        if r.y >= c.y && r.b() <= c.b() {
            self.button(r, "Scan", 0, if scope.is_empty() { Btn::Disabled } else { Btn::Accent }, hit(Scan), t);
        }
    }

    fn paint_cleanup_results(&mut self, c: Rect, t: &Theme) {
        let panel = self.cleanup.panel.is_some();
        let push = panel && c.w >= 900.0;
        let left = Rect::new(c.x, c.y, c.w - if push { PANEL_W } else { 0.0 }, c.h);
        let done = self.cleanup.done.is_some() && self.cleanup.phase != Phase::Scan;
        let w = (left.w - 40.0).max(0.0);
        // The docked panel leaves room for two slightly narrower tiles at 1280 DIPs.
        let min_tile = if push { 275.0 } else { 300.0 };
        let cols = (((w + TILE_GAP) / (min_tile + TILE_GAP)) as usize).max(1);
        self.cleanup.columns = cols;
        self.paint_cleanup_header(left, done, t);
        self.paint_cleanup_progress(left, t);
        let top = if !done {
            self.paint_cleanup_segments(left, t);
            self.paint_cleanup_drive_chips(left, t)
        } else if left.w >= 650.0 { 100.0 } else { 108.0 };
        let list = Rect::new(left.x, left.y + top, left.w, (left.h - top).max(0.0));
        self.cleanup.list_height = list.h;
        let visible = self.cleanup.visible();
        let rows = visible.len().div_ceil(cols);
        let tiles_h = if rows == 0 && !done { 48.0 } else { rows as f32 * TILE_STEP };
        let show_bin = (!panel || push) && self.cleanup.phase != Phase::Scan;
        let bin_h = if done { self.cleanup_done_height() } else if show_bin { 32.0 } else { 0.0 };
        let height = 4.0 + tiles_h + bin_h + SCROLL_PAD;
        self.cleanup.scroll_max = (height - list.h).max(0.0);
        if std::mem::take(&mut self.cleanup.reveal_panel_tile) && push
            && let Some(i) = visible.iter().position(|&group| Some(group) == self.cleanup.panel)
        {
            self.cleanup_reveal_tile(i);
        }
        let instant = self.cleanup_instant();
        let (scroll, moving) = scroll_pos(&mut self.cleanup.scroll, self.cleanup.scroll_max, instant);
        if moving { self.anim(); }
        self.gfx.clip(list);
        let mut y = list.y + 4.0 - scroll;
        let x = list.x + 20.0;
        if done {
            self.paint_cleanup_done_card(Rect::new(x, y, w, bin_h), list, t);
            y += bin_h;
        }
        let tile_w = (w - (cols - 1) as f32 * TILE_GAP) / cols as f32;
        for (i, &g) in visible.iter().enumerate() {
            let r = Rect::new(x + (i % cols) as f32 * (tile_w + TILE_GAP), y + (i / cols) as f32 * TILE_STEP, tile_w, TILE_H);
            if r.intersects(&list) {
                self.paint_cleanup_tile(g, r, list, t);
            }
        }
        if visible.is_empty() && !done {
            self.cleanup_label("No cleanup items found", Rect::new(x, y + 12.0, w, 32.0), 13.0, 400, t.text2);
        }
        if !done && show_bin {
            self.paint_cleanup_bin(Rect::new(x, y + tiles_h, w, bin_h), list, t);
        }
        self.gfx.unclip();
        let selected = self.cleanup.totals(true);
        if self.cleanup.phase != Phase::Scan && (!done || selected.count > 0) && (!panel || push) {
            self.paint_cleanup_pill(left, selected, t);
        }
        if let Some(group) = self.cleanup.panel {
            let overlay_pill = !push && (!done || selected.count > 0);
            self.paint_cleanup_panel(c, group, overlay_pill, t);
            if overlay_pill {
                self.paint_cleanup_pill(c, selected, t);
            }
        }
    }

    pub(super) fn cleanup_reveal_tile(&mut self, i: usize) {
        let done = self.cleanup.done.is_some();
        let head = if done { self.cleanup_done_height() } else { 0.0 };
        let y = head + 4.0 + (i / self.cleanup.columns.max(1)) as f32 * TILE_STEP;
        let height = (self.cleanup.list_height - 80.0).max(0.0);
        let from = self.cleanup.scroll.target;
        self.cleanup.scroll.to(reveal(from, height, y, y + TILE_H).clamp(0.0, self.cleanup.scroll_max));
    }

    fn paint_cleanup_header(&mut self, c: Rect, done: bool, t: &Theme) {
        let y = c.y + 20.0;
        let x = c.x + 22.0;
        let mut right = c.r() - 20.0;
        if done && self.cleanup.phase != Phase::Review {
            self.paint_cleanup_done_head(c, t);
            return;
        }
        let p = &self.cleanup;
        let scanning = p.phase == Phase::Scan;
        let scope = p.scope(&self.drives);
        let n = scope.len();
        let totals = p.totals(false);
        let reviewed = p.review_done;
        let groups = p.tiles.iter().filter(|g| g.at.is_some()).count();
        let sub = if scanning { format!("Scanning {n} drive{} · {groups} of 6", if n == 1 { "" } else { "s" }) }
            else if p.limited { format!("{} on {n} drives · {:.1} s · scan limited", items(totals.count), p.seconds) }
            else { format!("{} on {n} drive{} · {:.1} s", items(totals.count), if n == 1 { "" } else { "s" }, p.seconds) };
        let width = match p.phase {
            Phase::Scan | Phase::Review => {
                let label = if scanning { "Cancel" } else { "Stop" };
                let w = if scanning { 66.0 } else { 54.0 };
                let r = Rect::new(right - w, y, w, 32.0);
                self.button(r, label, 0, Btn::Std, hit(Stop), t);
                right = r.x - 14.0;
                if !scanning && c.w >= 440.0 {
                    let s = format!("Reviewing {} of {}", count(reviewed), count(totals.count));
                    let sw = self.gfx.measure(&s, Font::Ui, 12.5, 400) + 2.0;
                    self.cleanup_label(&s, Rect::new(right - sw, y, sw, 32.0), 12.5, 400, t.text2);
                    self.cleanup_spinner(right - sw - 16.0, y + 16.0, t);
                    right -= sw + 30.0;
                }
                right - x
            }
            Phase::Moving => {
                self.cleanup_label("Moving items…", Rect::new(right - 118.0, y, 118.0, 32.0), 13.0, 400, t.text2);
                right - x - 120.0
            }
            _ => {
                let review_again = self.ai_on && self.cleanup.has_notes();
                if self.ai_on && totals.count > 0 {
                    let label = if review_again { "Review again" } else { "AI Review" };
                    let width = 48.0 + self.gfx.measure(label, Font::Ui, 13.0, 400);
                    let r = Rect::new(right - width, y, width, 32.0);
                    self.paint_cleanup_ai(r, label, t);
                    right = r.x - 8.0;
                }
                if !review_again {
                    let r = Rect::new(right - 32.0, y, 32.0, 32.0);
                    self.icon_btn(r, 0xE72C, 14.0, t.text2, hit(Rescan), R_CTRL, t);
                    right = r.x - 8.0;
                }
                if self.cleanup.panel.is_none() && right - x > 220.0 {
                    let all = self.scan_drives().iter().all(|d| (d.kind == "fixed") == scope.contains(&d.root));
                    let value = if all { "All".into() } else { scope.iter().map(|r| r.trim_end_matches('\\')).collect::<Vec<_>>().join(" ") };
                    let w = 75.0 + self.gfx.measure(&value, Font::Ui, 12.5, 400);
                    let r = Rect::new(right - w, y + 1.0, w, 30.0);
                    self.paint_cleanup_scope(r, &value, t);
                    right = r.x - 8.0;
                }
                right - x
            }
        };
        let instant = self.cleanup_instant();
        let (bytes, moving) = scroll_pos(&mut self.cleanup.found, f32::MAX, instant);
        if moving { self.anim(); }
        let bytes = if scanning || moving { bytes.round() as u64 } else { totals.bytes };
        self.cleanup_number(&fmt::size(bytes), if scanning { "found so far" } else { "found" },
            Rect::new(x, c.y + 16.0, width.max(90.0), 36.0), 30.0, if scanning { t.text2 } else { t.text }, t);
        let sub_w = self.gfx.measure(&sub, Font::Ui, 12.0, 400);
        self.cleanup_label(&sub, Rect::new(x, c.y + 54.0, (c.w - 40.0).max(0.0), 17.0), 12.0, 400, t.text3);
        if self.cleanup.show_notes && self.cleanup.has_notes() && self.cleanup.phase != Phase::Review && c.w >= 400.0 {
            let checks = self.cleanup.tiles.iter().map(|g| g.verdicts[0]).sum();
            let keeps = self.cleanup.tiles.iter().map(|g| g.verdicts[1]).sum();
            self.cleanup_verdicts(Rect::new(x + sub_w + 12.0, c.y + 54.0, c.w - sub_w - 55.0, 17.0), checks, keeps, false, t);
        }
    }

    fn cleanup_spinner(&mut self, x: f32, y: f32, t: &Theme) {
        self.gfx.ellipse(x, y, 6.0, t.track, false, 2.0);
        let phase = if self.cleanup_instant() { 0.0 } else { self.cleanup.scan_at.map_or(0.0, |a| a.elapsed().as_secs_f32() * 8.0) };
        for i in 0..4 {
            let a = phase + i as f32 * 0.28;
            let b = a + 0.28;
            self.gfx.line(x + a.cos() * 6.0, y + a.sin() * 6.0, x + b.cos() * 6.0, y + b.sin() * 6.0, t.accent, 2.0);
        }
        if !self.cleanup_instant() { self.anim(); }
    }

    fn paint_cleanup_ai(&mut self, r: Rect, label: &str, t: &Theme) {
        let on = self.cleanup.has_notes() && self.cleanup.show_notes;
        let h = hit(Ai);
        let scaled = self.press_in(r, h);
        self.gfx.fill_round(r, R_CTRL, if on { t.sel } else { t.card });
        self.gfx.stroke_round(r, R_CTRL, if on { t.sel_stroke } else { t.stroke }, 1.0);
        self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * self.hk(h)));
        self.gfx.glyph(0xF4A5, Rect::new(r.x + 13.0, r.y, 14.0, r.h), 13.0, t.accent_text);
        self.cleanup_label(label, Rect::new(r.x + 35.0, r.y, r.w - 43.0, r.h), 13.0, 400, t.text);
        if scaled { self.gfx.pop_xf(); }
        self.hit(r, h);
    }

    fn paint_cleanup_scope(&mut self, r: Rect, value: &str, t: &Theme) {
        let h = hit(Scope);
        self.gfx.fill_round(r, R_CTRL, t.card);
        self.gfx.stroke_round(r, R_CTRL, t.stroke, 1.0);
        self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * self.hk(h)));
        self.cleanup_label("Drives", Rect::new(r.x + 12.0, r.y, 38.0, r.h), 12.5, 400, t.text);
        self.cleanup_label(value, Rect::new(r.x + 51.0, r.y, r.w - 72.0, r.h), 12.5, 400, t.text2);
        self.gfx.glyph(0xE70D, Rect::new(r.r() - 19.0, r.y, 9.0, r.h), 9.0, t.text2);
        self.hit(r, h);
    }

    fn paint_cleanup_progress(&mut self, c: Rect, t: &Theme) {
        let instant = self.cleanup_instant();
        let p = &mut self.cleanup;
        let running = matches!(p.phase, Phase::Scan | Phase::Review);
        let fade = if running { 1.0 } else { p.progress_at.map_or(0.0, |at| 1.0 - motion::lin(at, 300.0)) };
        if fade <= 0.0 || self.shooting && !running {
            return;
        }
        let (value, moving) = scroll_pos(&mut p.progress, 1.0, instant);
        self.gfx.fill(Rect::new(c.x, c.y, c.w * value, 2.0), t.accent.a(fade));
        if moving || fade < 1.0 { self.anim(); }
    }

    fn paint_cleanup_segments(&mut self, c: Rect, t: &Theme) {
        let instant = self.cleanup_instant();
        let mut segs = [(0.0, 0.0); 6];
        let mut moving = false;
        for (i, g) in self.cleanup.tiles.iter_mut().enumerate() {
            if g.total.bytes == 0 || g.at.is_none() {
                continue;
            }
            let k = motion::since(g.at.unwrap(), Spring::Gentle, instant);
            let (fill, active) = scroll_pos(&mut g.fill, 1.0, instant);
            segs[i] = (g.total.bytes as f32 * k, fill);
            moving |= active || k < 1.0;
        }
        let n = segs.iter().filter(|s| s.0 > 0.0).count();
        if n == 0 {
            if moving { self.anim(); }
            return;
        }
        let sum: f32 = segs.iter().map(|s| s.0).sum();
        let span = (c.w - 42.0 - 3.0 * (n - 1) as f32 - 6.0 * n as f32).max(0.0);
        let mut x = c.x + 22.0;
        for (g, (bytes, fill)) in segs.into_iter().enumerate().filter(|(_, s)| s.0 > 0.0) {
            let r = Rect::new(x, c.y + 82.0, 6.0 + span * (bytes / sum), 10.0);
            self.gfx.fill_round(r, 5.0, t.track);
            self.gfx.fill_round(Rect::new(r.x, r.y, r.w * fill, r.h), 5.0, t.accent);
            if self.hov(hit(Segment(g))) || self.hov(hit(Tile(g))) || self.hov(hit(Items(g))) {
                self.gfx.stroke_round(r.inset(-3.0, -3.0), 8.0, t.text2, 1.5);
            }
            self.hit(r, hit(Segment(g)));
            x = r.r() + 3.0;
        }
        if moving { self.anim(); }
    }

    fn paint_cleanup_drive_chips(&mut self, c: Rect, t: &Theme) -> f32 {
        let drives = self.cleanup.by_drive();
        if drives.is_empty() { return 108.0; }
        let left = c.x + 22.0;
        let right = c.r() - 20.0;
        let mut x = left;
        let mut y = c.y + 102.0;
        self.gfx.clip(Rect::new(left, y, (right - left).max(0.0), (c.b() - y).max(0.0)));
        for (letter, total) in drives {
            let label = format!("{letter}:");
            let root = format!("{label}\\");
            let Some(d) = self.drives.iter().find(|d| d.root.eq_ignore_ascii_case(&root)) else { continue };
            let kind = d.kind;
            let selected = total.count > 0;
            let (frees, free) = if selected {
                let before = fmt::size_short(d.free);
                let after = fmt::size_short(d.free.saturating_add(total.bytes).min(d.total.max(d.free)));
                let before = match (before.rsplit_once(' '), after.rsplit_once(' ')) {
                    (Some((value, unit)), Some((_, next))) if unit == next => value,
                    _ => &before,
                };
                (format!("frees {}", fmt::size(total.bytes)), format!("· {before} → {after} free"))
            } else {
                ("nothing selected".into(), String::new())
            };
            let label_w = self.gfx.measure(&label, Font::Ui, 12.0, 600).ceil() + 2.0;
            let frees_w = self.gfx.measure(&frees, Font::Ui, 12.0, 400).ceil() + 2.0;
            let free_w = if selected { self.gfx.measure(&free, Font::Ui, 12.0, 400).ceil() + 6.0 } else { 0.0 };
            let width = 22.0 + label_w + 4.0 + frees_w + free_w;
            if x > left && x + width > right {
                x = left;
                y += 22.0;
            }
            self.gfx.push_scale(x, y + 4.0, 0.4, 0.0, 0.0);
            self.drive_slab(x, y + 4.0, &root, kind, t);
            self.gfx.pop_xf();
            let tx = x + 22.0;
            self.cleanup_label(&label, Rect::new(tx, y, label_w, 20.0), 12.0, 600, if selected { t.text } else { t.text3 });
            let tx = tx + label_w + 4.0;
            self.cleanup_label(&frees, Rect::new(tx, y, frees_w, 20.0), 12.0, 400, if total.bytes > 0 { t.accent_text } else { t.text3 });
            if selected {
                self.cleanup_label(&free, Rect::new(tx + frees_w + 4.0, y, free_w - 4.0, 20.0), 12.0, 400, t.text3);
            }
            x += width + 16.0;
        }
        self.gfx.unclip();
        y - c.y + 30.0
    }

    fn paint_cleanup_tile(&mut self, g: usize, r: Rect, list: Rect, t: &Theme) {
        let tile = &self.cleanup.tiles[g];
        let (total, sel, verdicts) = (tile.total, tile.selected, tile.verdicts);
        let pending = self.cleanup.phase == Phase::Scan && tile.at.is_none();
        let k = self.cleanup_motion(tile.at, Spring::Gentle);
        let tick = self.cleanup_motion(tile.tick_at, Spring::Bouncy);
        let slide = if self.cleanup.reduce_motion == Some(true) { 0.0 } else { (1.0 - k) * 8.0 };
        let h = hit(Tile(g));
        let (name, reason, _) = GROUPS[g];
        let on = sel.count > 0;
        let mixed = on && sel.count != total.count;
        let scaled = !pending && self.pk(h) > 0.0 && self.cleanup.reduce_motion != Some(true);
        if scaled { self.gfx.push_scale(r.cx(), r.cy(), 1.0 - 0.015 * self.pk(h), 0.0, 0.0); }
        if k < 1.0 {
            self.gfx.opacity_layer(r.inset(-8.0, -8.0), k.max(0.05));
            self.gfx.push_scale(r.cx(), r.cy(), 1.0, 0.0, slide);
        }
        let tick = tick.clamp(0.0, 1.0);
        let amount = if pending { 0.0 } else if on { tick } else { 1.0 - tick };
        let hover = t.hover.3 * self.hk(h);
        self.cleanup_card(r, amount, t);
        self.gfx.fill_round(r, R_CARD, t.hover.a(hover));
        self.cleanup_badge(g, r.x + 16.0, r.y + 16.0, amount, t);
        let width = (r.w - 32.0).max(0.0);
        self.cleanup_label(name, Rect::new(r.x + 16.0, r.y + 62.0, width, 20.0), 13.5, 600, if pending { t.text2 } else { t.text });
        let desc = Rect::new(r.x + 16.0, r.y + 82.0, width, 16.0);
        if self.cleanup.show_notes && verdicts.iter().sum::<usize>() == total.count && total.count > 0 {
            self.cleanup_verdicts(desc, verdicts[0], verdicts[1], true, t);
        } else if self.cleanup.phase == Phase::Review {
            self.cleanup_shimmer(Rect::new(desc.x, desc.y + 5.0, width.min(120.0), 9.0), t);
        } else {
            self.cleanup_label(reason, desc, 12.0, 400, t.text2);
        }
        if pending {
            self.cleanup_shimmer(Rect::new(r.x + 16.0, r.b() - 31.0, width.min(92.0), 9.0), t);
        } else {
            let scale = if self.cleanup.reduce_motion == Some(true) { 1.0 } else { 0.5 + 0.5 * tick };
            self.cleanup_circle(Rect::new(r.r() - 36.0, r.y + 14.0, 22.0, 22.0), on, mixed, self.hov(h), scale, t);
            let more = Rect::new(r.r() - 72.0, r.b() - 41.0, 64.0, 28.0);
            if self.cleanup.phase != Phase::Scan {
                if self.cleanup.panel == Some(g) {
                    self.gfx.stroke_round(r, R_CARD, t.accent, 2.0);
                } else if self.cleanup.keyboard && self.cleanup.focus == g {
                    self.gfx.stroke_round(r, R_CARD, t.accent, 1.0);
                }
                self.hit(intersect(list, r), h);
                self.hit(intersect(list, more), hit(Items(g)));
            }
            let mh = hit(Items(g));
            self.gfx.fill_round(more, R_SMALL, t.hover.a(t.hover.3 * self.hk(mh)));
            self.cleanup_label("Items", Rect::new(more.x + 10.0, more.y, 37.0, more.h), 12.5, 400, if self.hov(mh) { t.text } else { t.text2 });
            self.gfx.glyph(0xE76C, Rect::new(more.r() - 18.0, more.y, 9.0, more.h), 8.0, t.text3);
            let n = fmt::size(if mixed { sel.bytes } else { total.bytes });
            let sub = if mixed { format!("of {} · {}", fmt::size(total.bytes), items(total.count)) } else { items(total.count) };
            let nr = Rect::new(r.x + 16.0, r.b() - 42.0, (more.x - r.x - 18.0).max(0.0), 30.0);
            self.gfx.clip(nr);
            self.cleanup_number(&n, &sub, nr, 22.0, if on { t.text } else { t.text2 }, t);
            self.gfx.unclip();
            let tags: Vec<_> = self.cleanup.tiles[g].drives.keys().map(char::to_string).collect();
            let mut x = nr.x + self.gfx.measure(&n, Font::Display, 22.0, 600)
                + 7.0 + self.gfx.measure(&sub, Font::Ui, 12.5, 400) + 6.0;
            let widths: Vec<_> = tags.iter().map(|label| self.gfx.measure(label, Font::Ui, 11.0, 600).ceil() + 8.0).collect();
            if x + widths.iter().map(|w| w + 4.0).sum::<f32>() - 4.0 <= nr.r() {
                for (label, w) in tags.iter().zip(widths) {
                    let tag = Rect::new(x, nr.cy() - 4.0, w, 16.0);
                    self.gfx.stroke_round(tag, 4.0, t.stroke, 1.0);
                    self.gfx.text(label, tag, Font::Ui, 11.0, 600, Align::Center, t.text3);
                    x += w + 4.0;
                }
            }
        }
        if k < 1.0 {
            self.gfx.pop_xf();
            self.gfx.pop_layer();
        }
        if scaled { self.gfx.pop_xf(); }
    }

    fn cleanup_shimmer(&self, r: Rect, t: &Theme) {
        self.gfx.fill_round(r, r.h / 2.0, t.track);
        if !self.shooting && self.cleanup.reduce_motion != Some(true) {
            let p = self.cleanup.scan_at.map_or(0.0, |a| (a.elapsed().as_secs_f32() / 1.3).fract());
            let w = r.w * 0.3;
            self.gfx.fill_round(Rect::new(r.x + (r.w - w) * p, r.y, w, r.h), r.h / 2.0, t.field_border.a(0.38));
            self.anim();
        }
    }

    fn cleanup_verdicts(&mut self, r: Rect, checks: usize, keeps: usize, safe: bool, t: &Theme) {
        let mut x = r.x;
        for (n, name, color) in [(checks, "to check", t.warn), (keeps, "keep", t.danger), (usize::from(safe && checks + keeps == 0), "All safe", t.ok)] {
            if n == 0 || x + 8.0 >= r.r() { continue; }
            let s = if name == "All safe" { name.into() } else { format!("{} {name}", count(n)) };
            self.gfx.ellipse(x + 3.5, r.cy(), 3.5, color, true, 1.0);
            self.cleanup_label(&s, Rect::new(x + 12.0, r.y, r.r() - x - 12.0, r.h), 12.0, 400, if safe { color } else { t.text2 });
            x += 24.0 + self.gfx.measure(&s, Font::Ui, 12.0, 400);
        }
    }

    fn paint_cleanup_bin(&mut self, r: Rect, list: Rect, t: &Theme) {
        if !r.intersects(&list) { return; }
        let info = self.cleanup.bin;
        let label = "Recycle Bin";
        let meta = format!(" · {} · {}", fmt::size(info.bytes), items(info.count as usize));
        let label_w = self.gfx.measure(label, Font::Ui, 13.0, 400);
        let meta_w = self.gfx.measure(&meta, Font::Ui, 12.0, 400);
        self.gfx.glyph(BIN_GLYPH, Rect::new(r.x, r.y, 16.0, r.h), 16.0, t.text3);
        let x = r.x + 24.0;
        self.cleanup_label(label, Rect::new(x, r.y, label_w + 2.0, r.h), 13.0, 400, t.text2);
        self.cleanup_label(&meta, Rect::new(x + label_w, r.y, meta_w + 2.0, r.h), 12.0, 400, t.text3);
        let open_w = self.gfx.measure("Open", Font::Ui, 13.0, 400);
        let b = Rect::new(x + label_w + meta_w + 10.0, r.y, open_w + 2.0, r.h);
        self.cleanup_label("Open", b, 13.0, 400, t.accent_text);
        if self.hov(hit(Bin)) {
            self.gfx.line(b.x, b.cy() + 7.0, b.x + open_w, b.cy() + 7.0, t.accent_text, 1.0);
        }
        self.hit(intersect(b, list), hit(Bin));
    }

    fn paint_cleanup_done_bin(&mut self, r: Rect, list: Rect, t: &Theme) {
        if !r.intersects(&list) { return; }
        let info = self.cleanup.bin;
        let s = format!("{} · {}", fmt::size(info.bytes), items(info.count as usize));
        self.gfx.glyph(BIN_GLYPH, Rect::new(r.x + 12.0, r.y, 36.0, r.h), 16.0, t.text2);
        self.cleanup_label("Recycle Bin", Rect::new(r.x + 60.0, r.y + 9.0, r.w - 125.0, 20.0), 13.0, 600, t.text);
        self.cleanup_label(&s, Rect::new(r.x + 60.0, r.y + 29.0, r.w - 228.0, 16.0), 12.0, 400, t.text2);
        let b = Rect::new(r.r() - 158.0, r.y + 12.0, 146.0, 30.0);
        if b.y >= list.y && b.b() <= list.b() && info.count > 0 {
            self.button(b, "Empty Recycle Bin", 0xE74D, Btn::Std, hit(Empty), t);
        }
    }

    fn paint_cleanup_pill(&mut self, c: Rect, selected: Total, t: &Theme) {
        let instant = self.cleanup_instant();
        let p = &mut self.cleanup;
        let (bytes, bytes_moving) = scroll_pos(&mut p.selected, f32::MAX, instant);
        let (n, n_moving) = scroll_pos(&mut p.selected_count, f32::MAX, instant);
        if bytes_moving || n_moving { self.anim(); }
        let selected = if selected.count > 0 { fmt::size(bytes.round() as u64) } else { String::new() };
        let sub = if selected.is_empty() { "Nothing selected".into() } else { format!("selected · {}", items(n.round() as usize)) };
        let num_w = self.gfx.measure(&selected, Font::Ui, 14.0, 600);
        let w = (num_w + self.gfx.measure(&sub, Font::Ui, 13.0, 400) + 225.0).min(c.w - 20.0);
        let k = self.cleanup_motion(self.cleanup.pill_at, Spring::Gentle);
        let slide = if self.cleanup.reduce_motion == Some(true) { 0.0 } else { 10.0 * (1.0 - k) };
        let r = Rect::new(c.cx() - w / 2.0, c.b() - 68.0 + slide, w, 52.0);
        self.gfx.shadow(r, 26.0, 8.0, 17.0, if t.dark { 0.44 * k } else { 0.17 * k });
        self.gfx.fill_round(r, 26.0, t.glass.a(t.glass.3 * k));
        self.gfx.stroke_round(r, 26.0, t.glass_stroke.a(t.glass_stroke.3 * k), 1.0);
        let b = Rect::new(r.r() - 183.0, r.y + 8.0, 175.0, 36.0);
        self.cleanup_label(&selected, Rect::new(r.x + 20.0, r.y, num_w + 2.0, r.h), 14.0, 600, t.text.a(k));
        let x = r.x + 20.0 + if num_w > 0.0 { num_w + 4.0 } else { 0.0 };
        self.cleanup_label(&sub, Rect::new(x, r.y, (b.x - x - 6.0).max(0.0), r.h), 13.0, 400, t.text2.a(k));
        self.round_button(b, "Move to Recycle Bin", 0xE74D, if self.cleanup.can_move() { Btn::Accent } else { Btn::Disabled }, hit(Move), 18.0, t);
    }
}

impl App {
    fn paint_cleanup_panel(&mut self, c: Rect, group: usize, overlay_pill: bool, t: &Theme) {
        let w = c.w.min(PANEL_W);
        let k = self.cleanup_motion(self.cleanup.panel_at, Spring::Gentle);
        let slide = if self.cleanup.reduce_motion == Some(true) { 0.0 } else { 24.0 * (1.0 - k) };
        let r = Rect::new(c.r() - w + slide, c.y, w, c.h);
        if k < 1.0 {
            self.gfx.opacity_layer(c, k.max(0.05));
        }
        self.gfx.shadow(r, 0.0, 0.0, 27.0, if t.dark { 0.35 } else { 0.12 });
        self.gfx.fill(r, t.card);
        self.gfx.fill(Rect::new(r.x, r.y, 1.0, r.h), t.stroke);
        self.hit(intersect(c, r), Hit::Block);
        let (name, why, _) = GROUPS[group];
        let tile = &self.cleanup.tiles[group];
        let (total, selected) = (tile.total, tile.selected);
        let tick = self.cleanup_motion(tile.tick_at, Spring::Snappy).clamp(0.0, 1.0);
        self.cleanup_badge(group, r.x + 21.0, r.y + 18.0, if selected.count > 0 { tick } else { 1.0 - tick }, t);
        self.cleanup_label(name, Rect::new(r.x + 69.0, r.y + 18.0, r.w - 121.0, 20.0), 14.5, 600, t.text);
        self.cleanup_label(why, Rect::new(r.x + 69.0, r.y + 39.0, r.w - 89.0, 16.0), 12.0, 400, t.text2);
        self.icon_btn(Rect::new(r.r() - 44.0, r.y + 12.0, 32.0, 32.0), 0xE711, 10.0, t.text2, hit(PanelClose), R_CTRL, t);
        self.cleanup_number(&fmt::size(total.bytes), &items(total.count), Rect::new(r.x + 21.0, r.y + 70.0, r.w - 41.0, 30.0), 22.0, t.text, t);
        let all = Rect::new(r.x + 1.0, r.y + 118.0, r.w - 1.0, 38.0);
        self.gfx.fill(all, t.hover.a(t.hover.3 * self.hk(hit(All))));
        self.gfx.fill(Rect::new(all.x, all.y, all.w, 1.0), t.divider);
        self.gfx.fill(Rect::new(all.x, all.b() - 1.0, all.w, 1.0), t.divider);
        self.cleanup_checkbox(Rect::new(r.x + 20.0, all.y + 11.0, 16.0, 16.0), selected.count > 0, selected.count > 0 && selected.count != total.count, hit(All), t);
        self.cleanup_label(&format!("All {}", count(total.count)), Rect::new(r.x + 48.0, all.y, 120.0, all.h), 12.5, 400, t.text2);
        let selected_text = fmt::size(selected.bytes);
        self.gfx.text(&format!("{selected_text} selected"), Rect::new(r.x + 170.0, all.y, (r.w - 190.0).max(0.0), all.h), Font::Ui, 12.5, 400, Align::Right, t.text3);
        if self.cleanup.can_select() { self.hit(intersect(c, all), hit(All)); }
        let list = Rect::new(r.x + 1.0, r.y + PANEL_HEAD, r.w - 1.0, (r.h - PANEL_HEAD - if overlay_pill { 70.0 } else { 0.0 }).max(0.0));
        self.paint_cleanup_rows(list, group, t);
        if k < 1.0 { self.gfx.pop_layer(); }
    }

    // Match the tick boxes in Settings > Places to search.
    fn cleanup_checkbox(&mut self, cb: Rect, on: bool, mixed: bool, h: Hit, t: &Theme) {
        let hk = self.hk(h);
        if on && !mixed {
            self.gfx.fill_round(cb, 4.0, t.accent);
            self.gfx.glyph(0xE73E, cb, 10.0, t.on_accent);
        } else {
            self.gfx.fill_round(cb, 4.0, t.field.a(0.6 + 0.4 * hk));
            self.gfx.stroke_round(cb, 4.0, if hk > 0.0 { t.text2 } else { t.field_border }, 1.0);
            if mixed {
                self.gfx.fill_round(Rect::new(cb.x + 4.0, cb.y + 4.0, 8.0, 8.0), 2.0, t.accent);
            }
        }
    }

    fn paint_cleanup_rows(&mut self, list: Rect, group: usize, t: &Theme) {
        let instant = self.cleanup_instant();
        let p = &mut self.cleanup;
        let reviewing = p.phase == Phase::Review;
        let notes = p.show_notes;
        // Prefix heights allow direct access to the visible rows even when some rows have verdicts.
        let mut animating = false;
        if p.offsets.len() != p.order.len() + 1 || p.rows_moving {
            p.offsets.clear();
            p.offsets.push(4.0);
            for &i in &p.order {
                let data = &p.tiles[group].rows[i];
                let k = if notes { data.advice_at.map_or(f32::from(reviewing), |at| motion::since(at, Spring::Gentle, instant)) } else { 0.0 };
                animating |= notes && data.advice_at.is_some() && k < 1.0;
                let detail_line = if data.item.copy_of.is_some() || data.item.is_package_download() { 17.0 } else { 0.0 };
                p.offsets.push(p.offsets.last().copied().unwrap_or(4.0) + 52.0 + detail_line + 18.0 * k);
            }
            p.rows_moving = animating;
        }
        let height = p.offsets.last().copied().unwrap_or(0.0);
        p.panel_max = (height + 16.0 - list.h).max(0.0);
        let (scroll, moving) = scroll_pos(&mut p.panel_scroll, p.panel_max, instant);
        let first = p.offsets.partition_point(|y| *y <= scroll).saturating_sub(1);
        let last = p.offsets.partition_point(|y| *y <= scroll + list.h).min(p.order.len());
        if moving || animating { self.anim(); }
        self.gfx.clip(list);
        for slot in first..last {
            let i = self.cleanup.order[slot];
            let data = self.cleanup.tiles[group].rows[i].clone();
            let top = self.cleanup.offsets[slot];
            let r = Rect::new(list.x + 6.0, list.y + top - scroll, list.w - 12.0, self.cleanup.offsets[slot + 1] - top);
            self.paint_cleanup_row(r, list, group, i, slot, data, t);
        }
        if self.cleanup.panel_max > 0.0 {
            let h = (list.h * list.h / (height + 16.0)).clamp(12.0, list.h.max(12.0));
            let y = list.y + (list.h - h) * (scroll / self.cleanup.panel_max);
            self.gfx.fill_round(Rect::new(list.r() - 5.0, y, 4.0, h), 2.0, t.text3.a(0.45));
        }
        self.gfx.unclip();
    }

    fn paint_cleanup_row(&mut self, r: Rect, list: Rect, group: usize, i: usize, slot: usize, row: RowData, t: &Theme) {
        let h = hit(Row(i));
        self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * self.hk(h)));
        self.gfx.fill_round(r, R_CTRL, t.press.a(t.press.3 * self.pk(h)));
        if self.cleanup.keyboard && self.cleanup.row_focus == slot {
            self.gfx.stroke_round(r, R_CTRL, t.accent, 1.0);
        }
        self.cleanup_checkbox(Rect::new(r.x + 14.0, r.y + 18.0, 16.0, 16.0), row.on, false, h, t);
        let name = item_name(&row.item);
        let package = row.item.is_package_download();
        let is_dir = group == 2 && !package;
        let kind = crate::ficons::kind_of(&name, is_dir);
        self.item_icon(&row.item.path, &name, is_dir, kind, Rect::new(r.x + 40.0, r.y + 16.0, 20.0, 20.0), t);
        let x = r.x + 70.0;
        let rw = (r.w - 174.0).max(0.0);
        let nr = Rect::new(x, r.y + 8.0, rw, 18.0);
        let parent = row.item.path.parent();
        if is_dir && let Some(project) = parent.and_then(Path::file_name) {
            let project = project.to_string_lossy();
            let pw = (self.gfx.measure(&project, Font::Ui, 13.0, 400) + 2.0).min(nr.w);
            self.cleanup_label(&project, Rect::new(nr.x, nr.y, pw, nr.h), 13.0, 400, t.text);
            self.cleanup_label(&name, Rect::new(nr.x + pw + 6.0, nr.y, (nr.w - pw - 6.0).max(0.0), nr.h), 13.0, 400, t.text3);
        } else {
            self.cleanup_label(&name, nr, 13.0, 400, t.text);
        }
        if let Some(folder) = parent {
            let path = folder.to_string_lossy();
            let path = middle_ellipsis(&path, rw, |s| self.gfx.measure(s, Font::Ui, 12.0, 400));
            self.cleanup_label(&path, Rect::new(x, r.y + 26.0, rw, 17.0), 12.0, 400, t.text3);
        }
        let detail = row.item.copy_of.as_ref().map(|kept| format!("Copy of {}", kept.display()))
            .or_else(|| package.then(|| row.item.reason.to_owned()));
        let detail_line = if let Some(detail) = detail {
            let width = if package { (r.r() - x - 14.0).max(0.0) } else { rw };
            let detail = middle_ellipsis(&detail, width, |s| self.gfx.measure(s, Font::Ui, 12.0, 400));
            self.cleanup_label(&detail, Rect::new(x, r.y + 43.0, width, 17.0), 12.0, 400, t.text2);
            17.0
        } else { 0.0 };
        let size = fmt::size(row.item.size);
        let mut rr = Rect::new(r.r() - 100.0, r.y + 8.0, 86.0, 18.0);
        self.gfx.text(&size, rr, Font::Ui, 13.0, 400, Align::Right, t.text);
        rr.y += 18.0;
        self.gfx.text(&fmt::date(row.item.modified, false), rr, Font::Ui, 11.5, 400, Align::Right, t.text3);
        if self.cleanup.show_notes && r.h > 53.0 + detail_line {
            self.gfx.clip(r);
            if let Some(advice) = &row.advice {
                let k = self.cleanup_motion(row.advice_at, Spring::Gentle);
                let (verdict, color) = match advice.verdict {
                    Verdict::Safe => ("Safe", t.ok), Verdict::CheckFirst => ("Check first", t.warn), Verdict::Keep => ("Keep", t.danger),
                };
                let width = self.gfx.measure(verdict, Font::Ui, 12.0, 600) + 2.0;
                self.cleanup_label(verdict, Rect::new(x, r.y + 46.0 + detail_line, width, 17.0), 12.0, 600, color.a(k));
                self.cleanup_label(&advice.reason, Rect::new(x + width + 6.0, r.y + 46.0 + detail_line, (rw - width - 6.0).max(0.0), 17.0), 12.0, 400, t.text2.a(k));
            } else if self.cleanup.phase == Phase::Review {
                self.cleanup_shimmer(Rect::new(x, r.y + 50.0 + detail_line, rw * 0.8, 9.0), t);
            }
            self.gfx.unclip();
        }
        if self.cleanup.can_select() { self.hit(intersect(list, r), h); }
    }

    fn paint_cleanup_done_head(&mut self, c: Rect, t: &Theme) {
        let Some(done) = &self.cleanup.done else { return };
        let (moved, in_use, too_big, no_bin) = (done.moved, done.in_use, done.too_big, done.no_bin);
        let (skipped, can_undo, at) = (done.skipped, !done.records.is_empty(), done.at);
        let k = self.cleanup_motion(Some(at), Spring::Gentle);
        if k < 1.0 { self.gfx.opacity_layer(c, k.max(0.05)); }
        let narrow = c.w < 650.0;
        let button_y = c.y + if narrow { 66.0 } else { 20.0 };
        let open = Rect::new(c.r() - 168.0, button_y, 148.0, 32.0);
        let undo = Rect::new(open.x - 90.0, button_y, 82.0, 32.0);
        let moving = self.cleanup.moving();
        self.button(open, "Open Recycle Bin", 0xE74D, if moving { Btn::Disabled } else { Btn::Std }, if moving { Hit::Block } else { hit(Bin) }, t);
        self.button(undo, "Undo", 0xE7A7, if can_undo && !moving { Btn::Std } else { Btn::Disabled }, if can_undo && !moving { hit(Undo) } else { Hit::Block }, t);
        let icon = Rect::new(c.x + 22.0, c.y + 19.0, 30.0, 30.0);
        self.gfx.ellipse(icon.cx(), icon.cy(), 15.0, if moved.count > 0 { t.ok } else { t.warn }, true, 1.0);
        self.gfx.glyph(if moved.count > 0 { 0xE73E } else { 0xE7BA }, icon, 16.0, t.layer);
        let title = if moved.count > 0 { format!("{} moved to the Recycle Bin", fmt::size(moved.bytes)) } else { "No items moved".into() };
        let x = icon.r() + 16.0;
        let w = if narrow { c.r() - x - 20.0 } else { undo.x - x - 12.0 };
        self.gfx.text(&title, Rect::new(x, c.y + 16.0, w, 36.0), Font::Display, if narrow { 20.0 } else { 24.0 }, 600, Align::Left, t.text);
        let mut sub = items(moved.count);
        if moving {
            sub.push_str(" · Working…");
        } else {
            if in_use > 0 { sub.push_str(&format!(" · {} in use, skipped", count(in_use))); }
            if too_big > 0 { sub.push_str(&format!(" · {} too big for the bin", count(too_big))); }
            if no_bin > 0 { sub.push_str(&format!(" · {} on drives without a bin", count(no_bin))); }
            let other = skipped - in_use - too_big - no_bin;
            if other > 0 { sub.push_str(&format!(" · {} skipped", count(other))); }
        }
        self.cleanup_label(&sub, Rect::new(x, c.y + 54.0, w, 17.0), 12.0, 400, t.text3);
        if k < 1.0 { self.gfx.pop_layer(); }
    }

    fn cleanup_done_height(&self) -> f32 {
        102.0 + 60.0 * self.cleanup.done.as_ref().map_or(0, |d| d.by_drive.len()) as f32
    }

    fn paint_cleanup_done_card(&mut self, r: Rect, list: Rect, t: &Theme) {
        let Some(done) = &self.cleanup.done else { return };
        let k = self.cleanup_motion(Some(done.at), Spring::Gentle);
        let drives: Vec<_> = self.drives.iter().filter_map(|d| Some((d.clone(), *done.by_drive.get(&d.root)?))).collect();
        if k < 1.0 { self.gfx.opacity_layer(list, k.max(0.05)); }
        let card = Rect::new(r.x, r.y, r.w, r.h - 46.0);
        self.cleanup_card(card, 0.0, t);
        self.paint_cleanup_done_bin(Rect::new(r.x, r.y, r.w, 56.0), list, t);
        for (i, (d, bytes)) in drives.iter().enumerate() {
            let y = r.y + 56.0 + i as f32 * 60.0;
            self.gfx.fill(Rect::new(r.x, y, r.w, 1.0), t.divider);
            self.drive_slab(r.x + 12.0, y + 14.0, &d.root, d.kind, t);
            let x = r.x + 66.0;
            let split = r.w < 600.0;
            let width = (r.w - if split { 86.0 } else { 396.0 }).max(0.0);
            self.cleanup_label(&drive_name(d), Rect::new(x, y + 7.0, width, 20.0), 13.0, 400, t.text);
            self.cleanup_drive_bar(Rect::new(x, y + if split { 29.0 } else { 35.0 }, width, 6.0), d, Some(*bytes), t);
            let text = format!("{} free · +{} when emptied", fmt::size(d.free), fmt::size(*bytes));
            self.gfx.text(&text, Rect::new(r.r() - 316.0, y + if split { 36.0 } else { 20.0 }, 300.0, 20.0), Font::Ui, 12.5, 400, Align::Right, t.text2);
        }
        let found = self.cleanup.totals(false).bytes;
        self.cleanup_label(&format!("Still found · {}", fmt::size(found)), Rect::new(r.x + 2.0, card.b() + 16.0, r.w - 4.0, 20.0), 12.0, 600, t.text3);
        if k < 1.0 { self.gfx.pop_layer(); }
    }

    fn cleanup_drive_bar(&self, bar: Rect, d: &fmcore::shell::Drive, bin: Option<u64>, t: &Theme) {
        let used = d.total.saturating_sub(d.free);
        let w = bar.w * fraction(used, d.total);
        self.gfx.fill_round(bar, 3.0, t.track);
        self.gfx.fill_round(Rect::new(bar.x, bar.y, w, bar.h), 3.0, if bin.is_none() && fraction(used, d.total) > 0.9 { t.danger } else { t.accent });
        if let Some(bin) = bin {
            let b = bar.w * fraction(bin.min(used), d.total);
            let r = Rect::new(bar.x + w - b, bar.y, b, bar.h);
            self.gfx.fill(r, t.track.mix(t.accent, 0.35));
            self.gfx.clip(r);
            let mut x = r.x - 6.0;
            while x < r.r() {
                self.gfx.line(x, r.b(), x + 6.0, r.y, t.accent.a(0.40), 2.0);
                x += 6.0;
            }
            self.gfx.unclip();
        }
    }

    pub fn paint_cleanup_hidden(&mut self, r: Rect, t: &Theme) {
        self.gfx.fill(Rect::new(r.x, r.y, r.w, 1.0), t.divider);
        let label = format!("Hidden from Cleanup ({})", self.cleanup.hidden.len());
        let on = !self.cleanup.hidden.is_empty();
        let h = if on { hit(Hidden) } else { Hit::Block };
        if on { self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * self.hk(h))); }
        self.gfx.glyph(BROOM, Rect::new(r.x + 16.0, r.y, 18.0, r.h), 16.0, t.text2);
        self.cleanup_label(&label, Rect::new(r.x + 46.0, r.y, r.w - 78.0, r.h), 13.0, 400, t.text);
        if on { self.gfx.glyph(0xE70D, Rect::new(r.r() - 29.0, r.y, 12.0, r.h), 9.0, t.text2); }
        self.hit(r, h);
    }

    pub fn paint_cleanup_confirm(&mut self, t: &Theme) {
        let Some(dialog) = &self.cleanup.dialog else { return };
        let screen = Rect::new(0.0, 0.0, self.w, self.h);
        let w = 468.0f32.min(self.w - 24.0);
        let (title, groups, drive_line, focus_cancel, ready, at, warnings) = match dialog {
            Dialog::Move(c) => {
                let line = c.by_drive.iter().map(|(d, total)| format!("{d}: {}", fmt::size(total.bytes))).collect::<Vec<_>>().join(" · ");
                ("Move to Recycle Bin?", c.by_group, line, c.on_cancel, c.limits.is_some(), c.at, confirm_warnings(c))
            }
            Dialog::Empty { on_cancel, at } => ("Empty Recycle Bin?", [Total::default(); 6], String::new(), *on_cancel, true, *at, vec![]),
        };
        let empty = matches!(dialog, Dialog::Empty { .. });
        self.hit(screen, Hit::Block);
        let group_h = 30.0 * groups.iter().filter(|g| g.count > 0).count() as f32;
        let warn_h: f32 = warnings.iter().map(|(_, detail)| if detail.is_empty() { 36.0 } else { 52.0 }).sum();
        let drive_h = if drive_line.is_empty() { 0.0 } else { 22.0 };
        let h = if empty { 170.0 } else { 208.0 + drive_h + warn_h + group_h };
        let k = self.cleanup_motion(Some(at), Spring::Bouncy).min(1.0);
        self.gfx.fill(screen, t.scrim.a(t.scrim.3 * k));
        let r = Rect::new((screen.w - w) * 0.5, (screen.h - h) * 0.5, w, h);
        let pop = if self.cleanup.reduce_motion == Some(true) { 1.0 } else { 0.97 + 0.03 * k };
        let scale = ((screen.h - 20.0) / h).min(1.0) * pop;
        self.gfx.push_scale(r.cx(), r.cy(), scale, 0.0, 0.0);
        if k < 1.0 { self.gfx.opacity_layer(screen, k.max(0.05)); }
        self.card(r, R_MENU, t, true);
        let x = r.x + 24.0;
        let width = r.w - 48.0;
        self.gfx.text(title, Rect::new(x, r.y + 22.0, width, 28.0), Font::Display, 20.0, 600, Align::Left, t.text);
        if empty {
            self.gfx.para("Everything in the Recycle Bin will be deleted for good.", Rect::new(x, r.y + 58.0, width, 44.0),
                Font::Ui, 13.0, 400, Align::Left, t.text2, 20.0, false, 2);
        } else {
            let total = groups.iter().copied().fold(Total::default(), |mut a, b| { a.add(b); a });
            self.cleanup_number(&fmt::size(total.bytes), &items(total.count), Rect::new(x, r.y + 60.0, width, 38.0), 30.0, t.text, t);
            if drive_h > 0.0 {
                self.cleanup_label(&drive_line, Rect::new(x, r.y + 99.0, width, drive_h), 13.0, 400, t.text2);
            }
            let mut y = r.y + 108.0 + drive_h;
            self.gfx.fill(Rect::new(x, y, width, 1.0), t.divider);
            y += 8.0;
            for (i, g) in groups.iter().enumerate().filter(|(_, g)| g.count > 0) {
                self.gfx.glyph(GROUPS[i].2, Rect::new(x, y, 15.0, 30.0), 15.0, t.text3);
                self.cleanup_label(GROUPS[i].0, Rect::new(x + 25.0, y, (width - 185.0).max(0.0), 30.0), 13.0, 400, t.text2);
                self.gfx.text(&count(g.count), Rect::new(r.r() - 184.0, y, 70.0, 30.0), Font::Ui, 12.5, 400, Align::Right, t.text3);
                self.gfx.text(&fmt::size(g.bytes), Rect::new(r.r() - 104.0, y, 80.0, 30.0), Font::Ui, 13.0, 400, Align::Right, t.text);
                y += 30.0;
            }
            for (label, detail) in warnings {
                let ww = Rect::new(x, y + 4.0, width, if detail.is_empty() { 32.0 } else { 48.0 });
                self.gfx.fill_round(ww, R_CTRL, t.warn_fill);
                self.gfx.stroke_round(ww, R_CTRL, t.warn_stroke, 1.0);
                self.gfx.glyph(0xE7BA, Rect::new(x + 12.0, ww.y + 8.0, 16.0, 18.0), 14.0, t.warn);
                self.cleanup_label(&label, Rect::new(x + 38.0, ww.y + 7.0, width - 50.0, 19.0), 12.5, 400, t.warn);
                self.cleanup_label(&detail, Rect::new(x + 38.0, ww.y + 26.0, width - 50.0, 16.0), 12.0, 400, t.text2);
                y = ww.b();
            }
        }
        let bw = if empty { 100.0 } else { 165.0 };
        let y = r.b() - 24.0 - 32.0;
        let yes = Rect::new(r.r() - 24.0 - bw, y, bw, 32.0);
        let cancel = Rect::new(yes.x - 108.0, y, 100.0, 32.0);
        let label = if empty { "Empty" } else if ready { "Move to Recycle Bin" } else { "Checking the Recycle Bin…" };
        let yes_style = if !ready { Btn::Disabled } else if empty { Btn::Danger } else { Btn::Accent };
        let no_hits = self.no_hits;
        self.no_hits = true;
        self.button(cancel, "Cancel", 0, Btn::Std, hit(Cancel), t);
        self.button(yes, label, if !empty && ready { 0xE74D } else { 0 }, yes_style, hit(Confirm), t);
        self.no_hits = no_hits;
        if focus_cancel || !ready {
            self.gfx.stroke_round(cancel.inset(2.0, 2.0), R_CTRL - 1.0, t.accent, 1.0);
        }
        if k < 1.0 { self.gfx.pop_layer(); }
        self.gfx.pop_xf();
        let scaled = |b: Rect| Rect::new(r.cx() + (b.x - r.cx()) * scale, r.cy() + (b.y - r.cy()) * scale, b.w * scale, b.h * scale);
        self.hit(scaled(cancel), hit(Cancel));
        if ready { self.hit(scaled(yes), hit(Confirm)); }
    }
}

fn reduced_motion() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
    let mut enabled = windows::core::BOOL(1);
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some((&mut enabled as *mut windows::core::BOOL).cast()), Default::default()) }.is_ok()
        && !enabled.as_bool()
}

fn cleanup_card_fill(amount: f32, t: &Theme) -> C {
    t.card.mix(t.sel, amount * if t.dark { 0.40 } else { 0.55 })
}

fn middle_ellipsis<'a>(s: &'a str, width: f32, mut measure: impl FnMut(&str) -> f32) -> std::borrow::Cow<'a, str> {
    let full = measure(s);
    if full <= width {
        return s.into();
    }
    let chars: Vec<_> = s.chars().collect();
    if chars.len() < 8 {
        return s.into();
    }
    let mut keep = (chars.len() as f32 * width / full) as usize;
    keep = keep.clamp(6, chars.len() - 1);
    loop {
        let first = (keep / 4).max(3);
        let last = chars.len() - (keep - first);
        let cut: String = chars[..first].iter().chain(std::iter::once(&'…')).chain(&chars[last..]).collect();
        let used = measure(&cut);
        if used <= width || keep == 6 {
            return cut.into();
        }
        keep = ((keep as f32 * width / used) as usize).clamp(6, keep - 1);
    }
}

fn confirm_warnings(c: &ConfirmData) -> Vec<(String, String)> {
    let mut out = vec![];
    for (names, verdict) in c.flagged.iter().zip(["Check first", "Keep"]) {
        if !names.is_empty() {
            let n = names.len();
            let label = if n == 1 { format!("1 item marked {verdict} is included") } else { format!("{} items marked {verdict} are included", count(n)) };
            let more = if n > 2 { format!(" and {} more", count(n - 2)) } else { String::new() };
            out.push((label, format!("{}{more}", names[..n.min(2)].join(", "))));
        }
    }
    if let Some(limits) = &c.limits {
        for (why, reason) in [(Skip::TooBig, "too big for the Recycle Bin"), (Skip::NoBin, "on drives without a Recycle Bin")] {
            let mut drives: Vec<_> = limits.iter().filter(|(_, skip)| *skip == why).filter_map(|(p, _)| fmcore::recycle::drive_root(p)).collect();
            let n = limits.iter().filter(|(_, skip)| *skip == why).count();
            if n == 0 { continue; }
            drives.sort();
            drives.dedup();
            let label = format!("{} {} {reason}", items(n), if n == 1 { "is" } else { "are" });
            let detail = if drives.is_empty() { "Will be skipped".into() } else {
                format!("Will be skipped on {}", drives.iter().map(|d| d.trim_end_matches('\\')).collect::<Vec<_>>().join(", "))
            };
            out.push((label, detail));
        }
    }
    out
}
