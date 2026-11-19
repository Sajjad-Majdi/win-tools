//! Painting of the title bar and the Processes page (totals strip, attention chips, command bar, the list),
//! plus the small widgets every page uses (buttons, pills, toggles, segmented controls, scrollbars).
use super::app::*;
use super::gfx::{Align, C, Font, Rect};
use super::model::{self, Res, SortKey};
use super::motion;
use crate::procs::Kind;
use std::time::Instant;

pub struct Chip {
    pub key: String,
    pub text: String,
    pub glyph: u32,
    pub hung: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Std,
    Accent,
    Crit,
    Subtle,
}

/// Column of the process list: kind, x, width.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Col {
    Name,
    Type,
    Pid,
    M(Res),
}

pub const RES_ALL: [Res; 5] = [Res::Cpu, Res::Mem, Res::Disk, Res::Net, Res::Gpu];

impl App {
    pub fn res_color(&self, r: Res) -> C {
        match r {
            Res::Cpu => self.t.cpu,
            Res::Mem => self.t.mem,
            Res::Disk => self.t.disk,
            Res::Net => self.t.net,
            Res::Gpu => self.t.gpu,
        }
    }

    // ------------------------------------------------------------------ widgets
    /// Width of a button with an optional glyph and label.
    pub fn btn_w(&self, label: &str, glyph: u32, small: bool) -> f32 {
        let (pad, gs, fs, gap) = if small { (10.0, 11.0, 12.0, 6.0) } else { (12.0, 13.0, 13.0, 8.0) };
        let lw = self.gfx.measure(label, Font::Ui, fs, 400);
        let mut w = pad * 2.0 + lw;
        if glyph != 0 {
            w += gs + if label.is_empty() { 0.0 } else { gap };
        }
        w.ceil()
    }

    /// Standard / accent / destructive / subtle button. `enabled = false` draws the disabled look and no hit.
    pub fn button(&mut self, r: Rect, label: &str, glyph: u32, style: Btn, small: bool, enabled: bool, h: Hit) {
        let t = self.t;
        let (gs, fs, gap) = if small { (11.0, 12.0, 6.0) } else { (13.0, 13.0, 8.0) };
        // hover fades in, press dims and springs the button down to 0.97
        let hv = if enabled { self.hov(h) } else { 0.0 };
        let pd = if enabled { self.press_depth(h).clamp(0.0, 1.0) } else { 0.0 };
        let (fill, border, bottom, fg) = match (style, enabled) {
            (_, false) => (t.ctl, t.ctl_s, t.ctl_s, t.t3),
            (Btn::Std, _) => (t.ctl.mix(t.ctl_h, hv).mix(t.prs, pd), t.ctl_s, t.ctl_b, t.t1.mix(t.t2, pd)),
            (Btn::Accent, _) => {
                let c = t.acc.mix(t.acc_h, hv).fade(1.0 - 0.15 * pd);
                (c, c, c, t.on_acc.fade(1.0 - 0.2 * pd))
            }
            (Btn::Crit, _) => (t.crit_btn.fade(1.0 - 0.1 * hv - 0.1 * pd), t.crit_btn, t.crit_btn, t.on_crit),
            (Btn::Subtle, _) => (t.hov.fade(hv).mix(t.prs, pd), C(0.0, 0.0, 0.0, 0.0), C(0.0, 0.0, 0.0, 0.0), t.t1),
        };
        let pressed = self.press_begin(r, h);
        let g = &self.gfx;
        g.fill_round(r, 4.0, fill);
        if border.3 > 0.0 {
            g.stroke_round(r, 4.0, border, 1.0);
            if bottom != border {
                g.fill(Rect::new(r.x + 3.0, r.b() - 1.0, r.w - 6.0, 1.0), bottom);
            }
        }
        let lw = g.measure(label, Font::Ui, fs, 400);
        let cw = lw + if glyph != 0 { gs + if label.is_empty() { 0.0 } else { gap } } else { 0.0 };
        let mut x = r.x + (r.w - cw) / 2.0;
        if glyph != 0 {
            g.glyph(glyph, Rect::new(x, r.y, gs, r.h), gs, fg);
            x += gs + gap;
        }
        if !label.is_empty() {
            g.text(label, Rect::new(x, r.y, lw + 2.0, r.h), Font::Ui, fs, 400, Align::Left, fg);
        }
        self.press_end(pressed);
        if enabled {
            self.hit(r, h);
        }
    }

    /// Command-bar button (subtle, 32 high): glyph 14 + 8 + label 13. Returns its width.
    pub fn cmd_btn(&mut self, x: f32, y: f32, label: &str, glyph: u32, chevron: bool, on: bool, enabled: bool, h: Hit) -> f32 {
        let t = self.t;
        let lw = self.gfx.measure(label, Font::Ui, 13.0, 400);
        let w = 10.0 + 14.0 + if label.is_empty() { 0.0 } else { 8.0 + lw } + if chevron { 6.0 + 9.0 } else { 0.0 } + 10.0;
        let r = Rect::new(x, y, w.ceil(), 32.0);
        let on_a = self.fade(hid(1, h), on);
        if on_a > 0.0 {
            self.gfx.fill_round(r, 4.0, t.hov.fade(on_a));
        }
        let pressed = enabled && self.press_begin(r, h);
        if enabled {
            self.hp(r, 4.0, h);
            self.hit(r, h);
        }
        let fg = if enabled { t.t1 } else { t.t3 };
        let g = &self.gfx;
        g.glyph(glyph, Rect::new(x + 10.0, y, 14.0, 32.0), 14.0, fg);
        if !label.is_empty() {
            g.text(label, Rect::new(x + 32.0, y, lw + 2.0, 32.0), Font::Ui, 13.0, 400, Align::Left, fg);
        }
        if chevron {
            g.glyph(0xE70D, Rect::new(x + 32.0 + lw + 6.0, y + 1.0, 9.0, 32.0), 9.0, t.t2);
        }
        self.press_end(pressed);
        r.w
    }

    /// Pill (type tag, flag, verdict). Returns its width.
    pub fn pill(&self, x: f32, cy: f32, text: &str, glyph: u32, fg: C, bg: C, h: f32, size: f32, weight: u32) -> f32 {
        let g = &self.gfx;
        let tw = g.measure(text, Font::Ui, size, weight);
        let pad = if h >= 24.0 { 10.0 } else { 8.0 };
        let gw = if glyph != 0 { size - 1.0 + 5.0 } else { 0.0 };
        let w = (pad * 2.0 + gw + tw).ceil();
        let r = Rect::new(x, cy - h / 2.0, w, h);
        g.fill_round(r, h / 2.0, bg);
        let mut tx = x + pad;
        if glyph != 0 {
            g.glyph(glyph, Rect::new(tx, r.y, size - 1.0, h), size - 1.5, fg);
            tx += gw;
        }
        g.text(text, Rect::new(tx, r.y, tw + 2.0, h), Font::Ui, size, weight, Align::Left, fg);
        w
    }

    /// "On"/"Off" label + toggle, right edge at `right`. Returns its full width.
    pub fn toggle(&mut self, right: f32, cy: f32, on: bool, h: Hit, label: bool) -> f32 {
        let t = self.t;
        let r = Rect::new(right - 40.0, cy - 10.0, 40.0, 20.0);
        let hv = self.hov(h);
        let (k, kv) = self.glide(hid(3, h), on as u8 as f32, motion::GLIDE);
        let k = k.clamp(-0.1, 1.1);
        let pressed = self.press_begin(r, h);
        let g = &self.gfx;
        let kc = k.clamp(0.0, 1.0);
        g.fill_round(r, 10.0, t.hov.fade(hv * (1.0 - kc)));
        g.fill_round(r, 10.0, t.acc.mix(t.acc_h, hv).fade(kc));
        g.stroke_round(r, 10.0, t.t2.fade(1.0 - kc), 1.0);
        // the knob stretches a little while it travels
        let stretch = (kv.abs() * 0.6).min(4.0);
        let kx = r.x + 9.0 + 20.0 * k;
        let kr = 5.0 + kc;
        g.fill_round(Rect::new(kx - kr - stretch / 2.0, cy - kr, kr * 2.0 + stretch, kr * 2.0), kr, t.t2.mix(t.on_acc, kc));
        self.press_end(pressed);
        let g = &self.gfx;
        let mut w = 40.0;
        if label {
            g.text(if on { "On" } else { "Off" }, Rect::new(r.x - 12.0 - 24.0, cy - 10.0, 24.0, 20.0), Font::Ui, 13.0, 400, Align::Right, t.t1);
            w += 36.0;
        }
        self.hit(Rect::new(right - w, cy - 14.0, w, 28.0), h);
        w
    }

    /// Segmented control; returns its width. `key` animates the selection pill.
    pub fn segmented(&mut self, right: f32, cy: f32, items: &[&str], sel: usize, group: u8) -> f32 {
        let t = self.t;
        let ws: Vec<f32> = items.iter().map(|s| (self.gfx.measure(s, Font::Ui, 13.0, 400) + 24.0).ceil()).collect();
        let total = ws.iter().sum::<f32>() + 2.0 * (items.len() as f32 - 1.0) + 6.0;
        let r = Rect::new(right - total, cy - 16.0, total, 32.0);
        self.gfx.fill_round(r, 6.0, t.inp);
        self.gfx.stroke_round(r, 6.0, t.ctl_s, 1.0);
        let mut x = r.x + 3.0;
        let mut xs = vec![];
        for w in &ws {
            xs.push(x);
            x += w + 2.0;
        }
        let sx = self.slide(10 + group, xs[sel]);
        let sw = ws[sel];
        let sr = Rect::new(sx, r.y + 3.0, sw, 26.0);
        self.gfx.fill_round(sr, 4.0, t.card);
        self.gfx.stroke_round(sr, 4.0, t.card_s, 1.0);
        self.gfx.fill_round(Rect::new(sr.cx() - 6.0, sr.b() - 5.0, 12.0, 3.0), 1.5, t.acc);
        for (i, s) in items.iter().enumerate() {
            let ir = Rect::new(xs[i], r.y + 3.0, ws[i], 26.0);
            let h = Hit::Seg(group, i as u8);
            if i != sel {
                self.hp(ir, 4.0, h);
            }
            let pressed = self.press_begin(ir, h);
            self.gfx.text(s, ir, Font::Ui, 13.0, 400, Align::Center, if i == sel { t.t1 } else { t.t2 });
            self.press_end(pressed);
            self.hit(ir, h);
        }
        total
    }

    /// Border of a text field: one even, subtle line all round at rest; a clean accent ring when focused
    /// (it fades in). No bottom bar.
    pub fn input_ring(&self, r: Rect, h: Hit, focus: bool) {
        let t = self.t;
        let f = self.fade(hid(7, h), focus);
        self.gfx.stroke_round(r, 4.0, t.inp_s.fade(1.0 - f), 1.0);
        if f > 0.0 {
            self.gfx.stroke_round(r, 4.0, t.acc.fade(f), 1.5);
        }
    }

    /// Selection highlight for a list: `base` is the list's origin row rect (y = top of content at scroll),
    /// `y` the selected row's content y. A new selection fades in; moving the selection glides the highlight
    /// with a spring and the accent pill stretches with its speed. `key` names the spring (one per list).
    pub fn sel_highlight(&mut self, base: Rect, id: u64, y: f32, key: u64) {
        let t = self.t;
        let now = Instant::now();
        let sel_at = {
            let mut mo = self.mo.borrow_mut();
            // a new selection, or a far jump (search, Page Down), fades in where it is instead of gliding
            let far = mo.springs.get(&key).map(|s| (s.x - y).abs() > base.h * 12.0).unwrap_or(true);
            let at = match mo.sel_prev {
                Some((_, at)) if !far => at,
                _ => {
                    mo.springs.insert(key, motion::Spring::new(y, 0.2));
                    now
                }
            };
            mo.sel_prev = Some((id, at));
            at
        };
        let a = self.tween_in(sel_at, motion::NORMAL, motion::decel);
        let (gy, v) = self.glide(key, y, motion::GLIDE);
        let r = Rect::new(base.x, base.y + gy, base.w, base.h);
        self.gfx.fill_round(r, 4.0, t.sel.fade(a));
        let ph = (16.0 * motion::decel(a) + (v.abs() * 0.012).min(10.0)).min(base.h - 8.0);
        self.gfx.fill_round(Rect::new(r.x, r.cy() - ph / 2.0, 3.0, ph), 1.5, t.acc.fade(a));
    }

    /// Exe icon or a glyph stand-in.
    pub fn draw_icon(&mut self, path: &str, r: Rect, fallback: u32) {
        if let Some(b) = self.icon(path, r.w) {
            self.gfx.bitmap(&b, r, 1.0);
        } else {
            let t = self.t;
            self.gfx.glyph(fallback, r, r.w * 0.85, t.t2);
        }
    }

    /// Overlay scrollbar for a scroll area `k`.
    pub fn scrollbar(&mut self, view: Rect, k: u8) {
        let ch = self.content_h[k as usize];
        if ch <= view.h + 1.0 {
            return;
        }
        let th = (view.h * view.h / ch).max(30.0);
        let ty = view.y + (view.h - th) * (self.scroll[k as usize] / (ch - view.h));
        let hot = self.hov(Hit::Scroll(k));
        let w = 3.0 + 3.0 * hot;
        let t = self.t;
        self.gfx.fill_round(Rect::new(view.r() - 4.0 - w, ty + 2.0, w, th - 4.0), w / 2.0, t.t3.fade(0.5 + 0.3 * hot));
        self.hit(Rect::new(view.r() - 12.0, view.y, 12.0, view.h), Hit::Scroll(k));
    }

    /// Text with the search tokens marked (acc-soft fill + accent underline). Returns the drawn width.
    pub fn text_marked(&self, s: &str, x: f32, y: f32, h: f32, max_w: f32, size: f32, c: C) -> f32 {
        let g = &self.gfx;
        if self.searching() {
            let low = s.to_lowercase();
            if low.len() == s.len() {
                for tok in model::tokens(&self.query) {
                    let mut from = 0;
                    while let Some(p) = low[from..].find(&tok) {
                        let pos = from + p;
                        let start = s[..pos].encode_utf16().count() as u32;
                        let len = s[pos..pos + tok.len()].encode_utf16().count() as u32;
                        // marks go under the text; hit-testing keeps them right for right-to-left runs
                        for (px, pw) in g.range_x(s, Font::Ui, size, 400, start, len) {
                            if px + pw <= max_w {
                                let r = Rect::new(x + px - 1.0, y + (h - size - 6.0) / 2.0, pw + 2.0, size + 6.0);
                                g.fill_round(r, 2.0, self.t.acc_soft);
                                g.fill(Rect::new(r.x, r.b() - 1.5, r.w, 1.5), self.t.acc);
                            }
                        }
                        from = pos + tok.len().max(1);
                    }
                }
            }
        }
        g.text_at(s, x, y, h, max_w, Font::Ui, size, 400, c)
    }

    // ------------------------------------------------------------------ navigation sidebar
    /// Left edge of the page content (right of the sidebar).
    pub fn cx0(&self) -> f32 {
        // the sidebar opened over a small window does not push the page
        if self.nav_auto() { NAV_W + 8.0 } else { self.nav_w + 8.0 }
    }

    /// Is the content area narrow (fewer columns, no sparklines in the summary)?
    pub fn narrow(&self) -> bool {
        self.w - self.cx0() < 960.0
    }

    /// Is the sidebar expanded (the user's choice, and the window is wide enough)?
    pub fn nav_open(&self) -> bool {
        if self.nav_auto() { self.nav_peek } else { self.prefs.nav_open }
    }

    /// The sidebar is the user's choice (expanded or collapsed, remembered) at any width; only a window with less
    /// than about 600 of content collapses it by itself, and then the toggle opens it over the page.
    pub fn nav_auto(&self) -> bool {
        self.w - NAV_W_OPEN - 20.0 < 600.0
    }

    /// Left sidebar: pane toggle, Processes, Startup apps; Settings at the bottom. Expanded 200 (icon + label),
    /// collapsed 48 (icons, tooltip on hover). The accent pill at the left edge springs to the selected page.
    pub fn paint_nav(&mut self) {
        let t = self.t;
        let target = if self.nav_open() { NAV_W_OPEN } else { NAV_W };
        self.nav_w = self.glide(0x6500, target, motion::GLIDE).0.clamp(NAV_W - 4.0, NAV_W_OPEN + 4.0);
        let w = self.nav_w;
        let ih = if self.prefs.density == 0 { 32.0 } else { 36.0 };
        let gap = 4.0;
        let label_a = ((w - NAV_W) / (NAV_W_OPEN - NAV_W)).clamp(0.0, 1.0);
        // narrow window: the opened sidebar is a panel over the page (drawn after it), with a shadow
        if self.nav_auto() && w > NAV_W + 1.0 {
            let pr = Rect::new(0.0, TITLE_H - 4.0, w, self.h - TITLE_H + 4.0);
            self.gfx.shadow(pr, 0.0, 0.0, 24.0, 0.2 * t.sh);
            self.gfx.fill(pr, t.fly);
            self.gfx.vline(pr.r(), pr.y, pr.b(), t.fly_s);
            self.hit(pr, Hit::Block);
        }
        // the pane toggle
        let tr = Rect::new(4.0, TITLE_H + 2.0, 40.0, ih);
        self.hp(tr, 4.0, Hit::NavToggle);
        let pressed = self.press_begin(tr, Hit::NavToggle);
        // a side-panel glyph (not a list), so it never looks like the Processes item under it
        self.gfx.glyph(if self.nav_open() { 0xE8A0 } else { 0xE89F }, Rect::new(tr.x + 12.0, tr.y, 16.0, tr.h), 15.0, t.t1);
        self.press_end(pressed);
        self.hit(tr, Hit::NavToggle);
        if self.nav_open() {
            self.tip(tr, "Collapse sidebar");
        }
        // (glyph, label, y, page): Processes, Performance, Startup apps; Settings at the bottom
        let items: [(u32, &str, f32, u8); 4] = [
            (0xE8FD, "Processes", tr.b() + gap + 4.0, 0),
            (0xE9D2, "Performance", tr.b() + gap + 4.0 + ih + gap, 3),
            (0xE7E8, "Startup apps", tr.b() + gap + 4.0 + 2.0 * (ih + gap), 1),
            (0xE713, "Settings", self.h - 12.0 - ih, 2),
        ];
        let mut sel_y = None;
        for (glyph, label, y, page) in items.iter() {
            let r = Rect::new(4.0, *y, w - 8.0, ih);
            let h = Hit::Tab(*page);
            let on = self.page == *page;
            if on {
                sel_y = Some(r.cy());
                self.gfx.fill_round(r, 4.0, t.hov.fade(self.fade(hid(1, h), true)));
            } else {
                self.fade(hid(1, h), false);
            }
            self.hp(r, 4.0, h);
            let pressed = self.press_begin(r, h);
            self.gfx.glyph(*glyph, Rect::new(r.x + 12.0, r.y, 16.0, r.h), 15.0, if on { t.t1 } else { t.t2.mix(t.t1, self.hov(h)) });
            if label_a > 0.01 {
                self.gfx.clip(Rect::new(r.x, r.y, r.w, r.h));
                self.gfx.text_at(label, r.x + 40.0, r.y, r.h, (r.w - 48.0).max(0.0), Font::Ui, 13.0, if on { 600 } else { 400 }, t.t1.fade(label_a));
                self.gfx.unclip();
            }
            self.press_end(pressed);
            self.hit(r, h);
        }
        if let Some(cy) = sel_y {
            let (py, v) = self.glide(0x6501, cy, motion::GLIDE);
            let ph = 16.0 + (v.abs() * 0.01).min(12.0);
            self.gfx.fill_round(Rect::new(4.0, py - ph / 2.0, 3.0, ph), 1.5, t.acc);
        }
        // collapsed: the name of the hovered item in a small tooltip
        if label_a < 0.5
            && matches!(self.hover, Hit::Tab(_) | Hit::NavToggle)
        {
            let (label, y) = match self.hover {
                Hit::Tab(p) => items.iter().find(|x| x.3 == p).map(|x| (x.1, x.2)).unwrap_or(("", tr.y)),
                _ => (if self.nav_open() { "Collapse sidebar" } else { "Expand sidebar" }, tr.y),
            };

            self.nav_tip = Some((label, y + ih / 2.0));
        } else {
            self.nav_tip = None;
        }
    }

    /// The collapsed sidebar's tooltip (drawn last, over the page).
    pub fn paint_nav_tip(&mut self) {
        let t = self.t;
        let Some((label, cy)) = self.nav_tip else { return };
        let a = self.fade(hid(8, self.hover), true);
        let tw = self.gfx.measure(label, Font::Ui, 12.0, 400) + 20.0;
        let r = Rect::new(self.nav_w + 2.0 - (1.0 - a) * 4.0, cy - 14.0, tw, 28.0);
        let old = self.gfx.push_alpha(a);
        self.gfx.shadow(r, 4.0, 4.0, 8.0, 0.12 * t.sh);
        self.gfx.card(r, 4.0, t.fly, t.fly_s);
        self.gfx.text(label, r, Font::Ui, 12.0, 400, Align::Center, t.t1);
        self.gfx.pop_alpha(old);
    }

    /// The keyboard focus ring around the focused control (not for the list, the search box or text fields:
    /// they show focus themselves).
    pub fn paint_focus(&mut self) {
        let Some(f) = self.focus else { return };
        if matches!(f, Hit::List | Hit::Search | Hit::KeyField | Hit::HexField) || self.dialog.is_some() || self.menu.is_some() {
            return;
        }
        if let Some(r) = self.hits.iter().rev().find(|(_, h)| *h == f).map(|(r, _)| *r) {
            self.gfx.stroke_round(r.inset(-2.0, -2.0), 6.0, self.t.t1, 2.0);
        }
    }

    /// The tooltip under the pointer, once it has rested 500 ms (not while a menu or dialog is open).
    pub fn paint_tip(&mut self) {
        let t = self.t;
        if self.menu.is_some() || self.dialog.is_some() || self.pressed != Hit::None || self.rest_at.elapsed() < std::time::Duration::from_millis(500) {
            return;
        }
        let (mx, my) = self.mouse;
        let Some((_, text)) = self.tips.iter().rev().find(|(r, _)| r.has(mx, my)).cloned() else { return };
        let maxw = 420.0f32.min(self.w - 16.0);
        let tw = (self.gfx.measure(&text, Font::Ui, 12.0, 400) + 20.0).min(maxw);
        let th = self.gfx.para_h(&text, tw - 20.0, Font::Ui, 12.0, 400, 16.0) + 12.0;
        let mut x = (mx - 8.0).clamp(8.0, self.w - tw - 8.0);
        let mut y = my + 22.0;
        if y + th > self.h - 8.0 {
            y = my - th - 10.0;
        }
        x = x.round();
        y = y.round();
        let r = Rect::new(x, y, tw, th);
        self.gfx.shadow(r, 4.0, 4.0, 8.0, 0.12 * t.sh);
        self.gfx.card(r, 4.0, t.fly, t.fly_s);
        self.gfx.para(&text, Rect::new(r.x + 10.0, r.y + 6.0, tw - 20.0, th), Font::Ui, 12.0, 400, Align::Left, t.t1, 16.0);
    }

    // ------------------------------------------------------------------ title bar
    pub fn paint_title(&mut self) {
        let t = self.t;
        let narrow = self.w < 1000.0;
        let mut x = 16.0 + 16.0 + 18.0;
        // logo
        let g = &self.gfx;
        // the app icon (installer/wintask.ico, tools/make_icons.py "light"): a light-gray rounded tile with three
        // dark rounded bars, drawn from the same 1024-unit geometry scaled to 16
        let lr = Rect::new(16.0, 16.0, 16.0, 16.0);
        let u = 16.0 / 1024.0;
        let tile = Rect::new(lr.x + 48.0 * u, lr.y + 48.0 * u, 928.0 * u, 928.0 * u);
        // a hairline keeps the light tile visible on a light title bar (the icon itself has none)
        g.fill_round_exact(tile.inset(-0.5, -0.5), 230.0 * u + 0.5, C(0.0, 0.0, 0.0, if t.dark { 0.0 } else { 0.10 }));
        g.fill_round_exact(tile, 230.0 * u, C::hex(0xE1E2E5));
        for (i, bh) in [220.0f32, 430.0, 310.0].iter().enumerate() {
            let x = 262.0 + i as f32 * 190.0;
            g.fill_round_exact(Rect::new(lr.x + x * u, lr.y + (740.0 - bh) * u, 120.0 * u, bh * u), 60.0 * u, C::hex(0x484C53));
        }
        if !narrow {
            let nw = g.text_at("Rust Task Manager", lr.r() + 10.0, 0.0, TITLE_H, 150.0, Font::Ui, 12.0, 400, t.t2);
            x = lr.r() + 10.0 + nw + 18.0;
        }
        // search, centred on the window
        let sw = if narrow { 280.0 } else { 340.0 };
        // centred on the window, but never closer than 16 to the last tab
        let sr = Rect::new(((self.w - sw) / 2.0).max(x + 16.0).round(), 8.0, sw, 32.0);
        let focus = self.search_focus || self.searching();
        let shv = self.hov(Hit::Search) * if focus { 0.0 } else { 1.0 };
        {
            let g = &self.gfx;
            g.fill_round(sr, 4.0, t.inp.mix(t.ctl_h, shv));
        }
        self.input_ring(sr, Hit::Search, focus);
        {
            let g = &self.gfx;
            g.glyph(0xE721, Rect::new(sr.r() - 10.0 - 12.0, sr.y, 12.0, sr.h), 12.0, t.t2);
            let tx = Rect::new(sr.x + 12.0, sr.y, sr.w - 12.0 - 60.0, sr.h);
            if self.query.is_empty() {
                g.text("Search", tx, Font::Ui, 13.0, 400, Align::Left, t.t3);
                if focus {
                    g.fill(Rect::new(tx.x, sr.y + 8.0, 1.0, 16.0), t.t1);
                }
            } else {
                // the text scrolls so the caret stays in view (no "..." over a long query)
                let (cx, full) = g.caret_x(&self.query, self.qcaret(), Font::Ui, 13.0, 400);
                let off = if full <= tx.w - 2.0 { 0.0 } else { (cx - (tx.w - 4.0)).clamp(0.0, full - tx.w + 4.0) };
                g.clip(Rect::new(tx.x - 2.0, sr.y, tx.w + 4.0, sr.h));
                if self.q_all && self.search_focus {
                    g.fill(Rect::new(tx.x - 1.0 - off, sr.y + 7.0, full + 2.0, 18.0), t.acc.a(0.30));
                }
                g.text(&self.query, Rect::new(tx.x - off, sr.y, full + 20.0, sr.h), Font::Ui, 13.0, 400, Align::Left, t.t1);
                if focus && !self.q_all {
                    g.fill(Rect::new(tx.x + cx - off, sr.y + 8.0, 1.0, 16.0), t.t1);
                }
                g.unclip();
            }
        }
        self.hit(sr, Hit::Search);
        if !self.query.is_empty() {
            let cr = Rect::new(sr.r() - 10.0 - 12.0 - 14.0 - 22.0, sr.y + 4.0, 24.0, 24.0);
            self.hp(cr, 4.0, Hit::SearchClear);
            self.gfx.glyph(0xE711, cr, 10.0, t.t2);
            self.hit(cr, Hit::SearchClear);
            self.tip(cr, "Clear search (Esc)");
        }
        // caption buttons (Settings is the third tab, so nothing else sits next to them)
        let cx = self.w - CAP_W * 3.0;
        self.paint_caption(cx);
    }

    /// Windows 11 caption buttons: 46 x 32, Segoe Fluent Icons at 10, hover and press fills that fade,
    /// close turns #C42B1C with a white glyph; glyphs dim while the window is inactive.
    fn paint_caption(&mut self, cx: f32) {
        let t = self.t;
        let maxed = unsafe { windows::Win32::UI::WindowsAndMessaging::IsZoomed(self.hwnd).as_bool() };
        let caps = [(Hit::Min, 0xE921u32), (Hit::Max, if maxed { 0xE923 } else { 0xE922 }), (Hit::Close, 0xE8BB)];
        let ink = if t.dark { C(1.0, 1.0, 1.0, 1.0) } else { C(0.0, 0.0, 0.0, 1.0) };
        let active = self.is_active();
        let base = if active { t.t1 } else { ink.a(if t.dark { 0.37 } else { 0.36 }) };
        for (i, (h, gl)) in caps.iter().enumerate() {
            let r = Rect::new(cx + i as f32 * CAP_W, 0.0, CAP_W, CAP_H);
            let hv = self.fade(hid(5, *h), self.nc_hover == *h);
            let pv = self.fade(hid(6, *h), self.nc_hover == *h && self.nc_down == *h);
            let fg;
            if *h == Hit::Close {
                // the window's top-right corner is rounded by DWM; the fill follows it
                let red = C::hex(0xC42B1C);
                self.gfx.fill(r, red.fade(hv * (1.0 - 0.1 * pv)));
                fg = base.mix(C(1.0, 1.0, 1.0, 1.0), hv).fade(1.0 - 0.3 * pv);
            } else {
                // Fluent subtle fills: hover 6%, pressed 4% (pressed is lighter than hover)
                let a = 0.06 * hv + (0.04 - 0.06 * hv) * pv;
                self.gfx.fill(r, ink.a(a.max(0.0)));
                fg = base.mix(t.t1, hv).fade(1.0 - 0.38 * pv);
            }
            self.gfx.glyph(*gl, r, 10.0, fg);
            self.hit(r, *h);
        }
    }

    // ------------------------------------------------------------------ attention
    /// Attention mark for a group: (glyph, text) — not responding, high CPU for minutes, memory still growing.
    pub fn flag(&self, gi: usize) -> Option<(u32, String)> {
        let g = self.groups.get(gi)?;
        if g.hung {
            return Some((0xE7BA, "Not responding".into()));
        }
        let now = Instant::now();
        let h = self.hist.get(&g.key);
        let pids = g.pids();
        let sampler_cpu = self.hist_anoms.iter().filter(|a| a.kind == "high_cpu" && pids.contains(&a.pid)).map(|a| a.minutes).max();
        let busy = h.map(|h| h.busy_min(now)).unwrap_or(0);
        let mins = sampler_cpu.unwrap_or(0).max(if busy >= 3 { busy } else { 0 });
        if mins >= 3 && g.cpu >= 1.0 {
            return Some((0xE916, format!("High CPU for {mins} min")));
        }
        let grow = self.hist_anoms.iter().filter(|a| a.kind == "memory_growing" && pids.contains(&a.pid)).map(|a| ((a.value * 1048576.0) as u64, a.minutes)).max_by_key(|x| x.0).or_else(|| h.and_then(|h| h.growth()));
        if let Some((b, m)) = grow {
            return Some((0xE70E, format!("+{} in {m} min", model::fmt_mem(b))));
        }
        None
    }

    pub fn chips(&self) -> Vec<Chip> {
        let mut v = vec![];
        for (gi, g) in self.groups.iter().enumerate() {
            if let Some((glyph, text)) = self.flag(gi) {
                let short = g.label.clone();
                let text = match glyph {
                    0xE7BA => format!("{short} is not responding"),
                    0xE916 => format!("{short}: {}", text.replacen("High CPU", "high CPU", 1)),
                    _ => format!("{short}: memory {text}"),
                };
                v.push(Chip { key: g.key.clone(), text, glyph, hung: glyph == 0xE7BA });
            }
        }
        v.sort_by_key(|c| if c.hung { 0 } else if c.glyph == 0xE916 { 1 } else { 2 });
        v
    }

    // ------------------------------------------------------------------ Processes page
    pub fn paint_processes(&mut self) {
        let t = self.t;
        let mut y = TITLE_H;
        y -= 8.0;
        let chips = if self.searching() || !self.prefs.chips { vec![] } else { self.chips() };
        // the chips row opens and closes with motion (height 0 <-> 30, fade)
        let ch = self.glide(0x6001, if chips.is_empty() { 0.0 } else { 8.0 + CHIPS_H }, motion::GLIDE).0.max(0.0);
        if !chips.is_empty() && ch > 1.0 {
            let a = (ch / (8.0 + CHIPS_H)).clamp(0.0, 1.0);
            let old = self.gfx.push_alpha(a);
            self.paint_chips(&chips, y + 8.0 - (8.0 + CHIPS_H - ch) * 0.5);
            self.gfx.pop_alpha(old);
        }
        y += ch;
        let layer = Rect::new(self.cx0(), y + 8.0, self.w - self.cx0() - 12.0, self.h - y - 8.0 - 12.0);
        self.gfx.card(layer, 8.0, t.layer, t.layer_s);
        self.gfx.clip(layer.inset(1.0, 1.0));
        self.paint_cmdbar(layer);
        // a small window shows the pane over the list (the list keeps its full width)
        let over = self.pane_overlay();
        let pane_w = self.pane.as_ref().map(|p| self.pane_w(p.kind)).unwrap_or(0.0).min(layer.w - 40.0);
        let list = Rect::new(layer.x, layer.y + CMD_H, layer.w - if over { 0.0 } else { pane_w }, layer.h - CMD_H);
        self.paint_list(list);
        if self.focus == Some(Hit::List) {
            self.gfx.stroke_round(list.inset(2.0, 2.0), 4.0, t.acc, 2.0);
        }
        if pane_w > 0.0 {
            let pr = Rect::new(layer.r() - pane_w, list.y, pane_w, list.h);
            if over {
                self.gfx.shadow(pr, 0.0, 0.0, 24.0, 0.18 * t.sh);
                // the list must not show through the pane laid over it
                self.gfx.fill(pr, t.layer.a(1.0));
            }
            self.paint_pane(pr, None);
        } else {
            self.pane_rect = Rect::default();
            if let Some((p, w, closed)) = { self.mo.borrow().ghost_pane.clone() } {
                let w = w.min(layer.w - 40.0);
                let pr = Rect::new(layer.r() - w, list.y, w, list.h);
                let saved = self.scroll[3];
                self.ghost(|a| {
                    a.pane = Some(p);
                    a.paint_pane(pr, Some(closed));
                    a.pane = None;
                });
                self.scroll[3] = saved;
                self.pane_rect = Rect::default();
            }
        }
        self.gfx.unclip();
        self.paint_toast(layer);
    }

    #[allow(dead_code)]
    fn paint_tiles(&mut self) {
        let t = self.t;
        let narrow = self.narrow();
        let card = Rect::new(self.cx0(), TITLE_H, self.w - self.cx0() - 12.0, TILES_H);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let n = 5.0;
        let sw = (card.w - 2.0) / n;
        // sparklines only where a tile has room for them next to "9.7 GB of 16 GB" (else the text is cut)
        let narrow = narrow || sw < 215.0;
        let s = &self.perf;
        let used = s.mem_total.saturating_sub(s.mem_avail);
        let disk_rate: f64 = s.disks.iter().map(|d| d.read_rate + d.write_rate).sum();
        let disk_pct = s.disks.iter().map(|d| d.active).fold(0.0f32, f32::max);
        let net_rate: f64 = s.nets.iter().map(|x| x.recv_rate + x.send_rate).sum();
        let busiest = s.nets.iter().max_by(|a, b| (a.recv_rate + a.send_rate).total_cmp(&(b.recv_rate + b.send_rate))).map(|x| x.name.clone()).unwrap_or_default();
        let (down, up): (f64, f64) = (s.nets.iter().map(|x| x.recv_rate).sum(), s.nets.iter().map(|x| x.send_rate).sum());
        let gpu = s.gpu.as_ref().map(|g| g.util).unwrap_or(0.0);
        let top = |f: &dyn Fn(&model::Group) -> f64| self.groups.iter().max_by(|a, b| f(a).total_cmp(&f(b))).filter(|g| f(g) > 0.0).map(|g| (g.label.clone(), f(g)));
        let tcpu = top(&|g| g.cpu as f64);
        let tmem = top(&|g| g.mem as f64);
        let tdisk = top(&|g| g.io);
        let tgpu = top(&|g| g.gpu as f64);
        let cpu_first = self.tot[0].len() < 2;
        // numbers change in place (no gliding: a calm, readable list)
        let cpu_now = if cpu_first { self.groups.iter().map(|g| g.cpu).sum::<f32>().min(100.0) } else { s.cpu };
        let items: [(Res, &str, String, String, (String, String), f32, f32); 5] = [
            (Res::Cpu, "CPU", format!("{:.0}%", cpu_now), String::new(), tcpu.map(|(l, v)| (l, model::fmt_pct(v as f32))).unwrap_or_default(), 0.0, 10.0),
            (Res::Mem, "Memory", model::fmt_gb(used), format!("of {}", model::fmt_gb(s.mem_total)), tmem.map(|(l, v)| (l, model::fmt_mem(v as u64))).unwrap_or_default(), 0.0, s.mem_total as f32),
            (Res::Disk, "Disk", format!("{disk_pct:.0}%"), if disk_rate < 1.0 { "0 KB/s".to_string() } else { model::fmt_rate(disk_rate) }, tdisk.map(|(l, v)| (l, model::fmt_rate(v))).unwrap_or_default(), 0.0, 1048576.0),
            (Res::Net, "Network", if net_rate < 1.0 { "0 KB/s".into() } else { model::fmt_rate(net_rate) }, busiest, (String::new(), String::new()), 0.0, 102400.0),
            (Res::Gpu, "GPU", format!("{gpu:.0}%"), String::new(), tgpu.map(|(l, v)| (l, if v < 1.0 { format!("{v:.1}%") } else { format!("{v:.0}%") })).unwrap_or_default(), 0.0, 10.0),
        ];
        let on = self.flyout.map(|f| f.0);
        for (i, (res, label, value, small, (tn, tv), _, floor)) in items.iter().enumerate() {
            let r = Rect::new(card.x + 1.0 + sw * i as f32, card.y + 1.0, sw, card.h - 2.0);
            let h = Hit::Tile(i as u8);
            let is_on = on == Some(i as u8);
            let on_a = self.fade(hid(1, h), is_on);
            if on_a > 0.0 {
                self.gfx.fill_round(r, 7.0, t.hov.fade(on_a));
            }
            if !is_on {
                self.hp(r, 7.0, h);
            }
            if i > 0 {
                // the divider fades where a neighbour is lit
                let lit = on_a.max(self.hov(h)).max(self.hov(Hit::Tile(i as u8 - 1))).max(self.fade(hid(1, Hit::Tile(i as u8 - 1)), on == Some(i as u8 - 1)));
                self.gfx.vline(r.x, card.y + 14.0, card.b() - 14.0, t.line.fade(1.0 - lit));
            }
            // the tile's full text, also when it is cut
            let tip = match res {
                Res::Net => format!("Network {value} {small}\nDown {} · Up {}", model::fmt_rate(down), model::fmt_rate(up)),
                _ if tn.is_empty() => format!("{label} {value} {small}"),
                Res::Disk => format!("{label} {value} {small}\nTop I/O: {tn} {tv}"),
                _ => format!("{label} {value} {small}\nTop: {tn} {tv}"),
            };
            self.tip(r, format!("{}\nClick for details", tip.replace("  ", " ")));
            let pressed = self.press_begin(r, h);
            let x = r.x + 16.0;
            let c = self.res_color(*res);
            let g = &self.gfx;
            g.ellipse(x + 3.0, card.y + 18.0, 3.0, c, true, 0.0);
            g.text_at(label, x + 12.0, card.y + 10.0, 16.0, r.w - 40.0, Font::Ui, 12.0, 400, t.t2);
            let vw = g.text_at(value, x, card.y + 26.0, 26.0, r.w - 32.0, Font::Display, 20.0, 600, t.t1);
            // very small tiles keep only the name and the value (their details are in the flyout)
            let tiny = sw < 150.0;
            if !small.is_empty() && !tiny {
                g.text_at(small, x + vw + 4.0, card.y + 28.0, 26.0, (r.w - 32.0 - vw - 4.0 - if narrow { 0.0 } else { 70.0 }).max(10.0), Font::Ui, 12.0, 400, t.t2);
            }
            // line 3: "Top: <name> <value>" (network: down / up, there is no per-app network counter)
            let ly = card.y + 52.0;
            let maxw = r.w - 32.0 - 4.0;
            if tiny {
            } else if *res == Res::Net {
                let s3 = format!("Down {} · Up {}", model::fmt_rate(down), model::fmt_rate(up));
                g.text_at(&s3, x, ly, 16.0, maxw, Font::Ui, 12.0, 400, t.t2);
            } else if !tn.is_empty() {
                // the per-app number under Disk is all I/O (files, pipes, devices), so it says so
                let w1 = g.text_at(if *res == Res::Disk { "Top I/O: " } else { "Top: " }, x, ly, 16.0, maxw, Font::Ui, 12.0, 400, t.t2);
                let vw2 = g.measure(tv, Font::Ui, 12.0, 400);
                let nw = g.measure(tn, Font::Ui, 12.0, 400).min(maxw - w1 - vw2 - 4.0).max(0.0);
                g.text(tn, Rect::new(x + w1, ly, nw + 2.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t1);
                g.text_at(tv, x + w1 + nw + 4.0, ly, 16.0, (maxw - w1 - nw - 4.0).max(0.0), Font::Ui, 12.0, 400, t.t2);
            }
            if !narrow {
                let vals: Vec<f32> = self.tot[i].iter().copied().collect();
                let peak = vals.iter().copied().fold(0.0f32, f32::max);
                let max = if *res == Res::Mem { *floor } else { (peak * 1.25).max(*floor) };
                g.spark(Rect::new(r.r() - 14.0 - 64.0, card.y + 16.0, 64.0, 30.0), &vals, max, c, 0.14, 1.5);
            }
            if on_a > 0.0 {
                let bw = 24.0 * motion::decel(on_a);
                g.fill_round(Rect::new(r.cx() - bw / 2.0, r.b() - 3.0, bw, 3.0), 1.5, t.acc);
            }
            self.press_end(pressed);
            self.hit(r, h);
        }
    }

    fn paint_chips(&mut self, chips: &[Chip], y: f32) {
        let t = self.t;
        let mut x = self.cx0();
        let max_x = self.w - 12.0;
        for (i, c) in chips.iter().enumerate().take(3) {
            let tw = self.gfx.measure(&c.text, Font::Ui, 12.5, 400);
            let lw = if c.hung { self.gfx.measure("End", Font::Ui, 12.5, 600) + 12.0 } else { 0.0 };
            let w = (10.0 + 13.0 + 8.0 + tw + lw + 12.0).ceil();
            if x + w > max_x - 90.0 && i > 0 {
                break;
            }
            let r = Rect::new(x, y, w, CHIPS_H);
            let h = Hit::Chip(i as u8);
            let (bg, bd) = if c.hung { (t.warn_bg, t.warn_s) } else { (t.card, t.card_s) };
            let pressed = self.press_begin(r, h);
            self.gfx.card(r, 15.0, bg, bd);
            let hv = self.hov(h) + 0.6 * self.press_depth(h).clamp(0.0, 1.0);
            if hv > 0.0 {
                self.gfx.fill_round(r.inset(1.0, 1.0), 14.0, if t.dark { C(1.0, 1.0, 1.0, 0.05 * hv) } else { C(0.0, 0.0, 0.0, 0.035 * hv) });
            }
            let g = &self.gfx;
            g.glyph(c.glyph, Rect::new(x + 10.0, y, 13.0, CHIPS_H), 13.0, t.warn);
            g.text_at(&c.text, x + 31.0, y, CHIPS_H, tw + 1.0, Font::Ui, 12.5, 400, t.t1);
            self.hit(r, h);
            if c.hung {
                let er = Rect::new(x + 31.0 + tw + 4.0, y + 3.0, lw, CHIPS_H - 6.0);
                let ehv = self.hov(Hit::ChipEnd(i as u8));
                if ehv > 0.0 {
                    self.gfx.fill_round(er, 4.0, t.acc_soft.fade(ehv));
                }
                self.gfx.text("End", er, Font::Ui, 12.5, 600, Align::Center, t.acc_t);
                self.hit(er, Hit::ChipEnd(i as u8));
            }
            self.press_end(pressed);
            x += w + 8.0;
        }
        if chips.len() > 3 {
            let s = format!("+{} more", chips.len() - 3);
            let w = (self.gfx.measure(&s, Font::Ui, 12.5, 400) + 24.0).ceil();
            let r = Rect::new(x, y, w, CHIPS_H);
            self.gfx.card(r, 15.0, t.card, t.card_s);
            self.gfx.text(&s, r, Font::Ui, 12.5, 400, Align::Center, t.t1);
            self.hit(r, Hit::ChipMore);
        }
    }

    fn paint_cmdbar(&mut self, layer: Rect) {
        let t = self.t;
        let y = layer.y + 8.0;
        let has = self.sel_group().is_some();
        let mut x = layer.x + 8.0;
        // a small window shows the command bar as icons (each has a tooltip)
        let compact = layer.w < 820.0;
        let lab = |s: &'static str| if compact { "" } else { s };
        let end_label = if self.shift && has { "End process" } else { "End task" };
        let ew = self.cmd_btn(x, y, lab(end_label), 0xE711, false, false, has, Hit::EndTask);
        if compact && has {
            self.tip(Rect::new(x, y, ew, 32.0), format!("{end_label} (Del)"));
        }
        x += ew + 2.0;
        let x_end = x;
        let can_restart = self.sel_group().map(|g| g.kind != Kind::Windows).unwrap_or(false);
        let rw = self.cmd_btn(x, y, lab("Restart"), 0xE72C, false, false, can_restart, Hit::Restart);
        if compact && can_restart {
            self.tip(Rect::new(x, y, rw, 32.0), "Restart");
        }
        // disabled buttons say why
        if !has {
            self.tip(Rect::new(layer.x + 8.0, y, x - layer.x - 8.0 + rw, 32.0), "Select a process first");
        } else if !can_restart {
            self.tip(Rect::new(x, y, rw, 32.0), "Windows processes are not restarted here");
        }
        x += rw + 2.0;
        let det_on = self.pane.as_ref().map(|p| p.kind == PaneKind::Details).unwrap_or(false);
        let dw = self.cmd_btn(x, y, lab("Details"), 0xE946, false, det_on, has, Hit::Details);
        if compact && has {
            self.tip(Rect::new(x, y, dw, 32.0), "Details (F1)");
        }
        x += dw + 2.0;
        let mw = self.cmd_btn(x, y, "", 0xE712, false, false, has, Hit::More);
        self.tip(Rect::new(x, y, mw, 32.0), if has { "More actions" } else { "Select a process first" });
        let _ = x_end;
        let left_end = x + mw;
        // right side
        let sort = if compact { self.sort.label().to_string() } else { format!("Sort: {}", self.sort.label()) };
        let sw = 10.0 + 14.0 + 8.0 + self.gfx.measure(&sort, Font::Ui, 13.0, 400) + 6.0 + 9.0 + 10.0;
        let mut rx = layer.r() - 8.0 - sw.ceil();
        let menu_sort = self.menu.as_ref().map(|m| m.kind == MenuKind::Sort).unwrap_or(false);
        self.cmd_btn(rx, y, &sort, 0xE8CB, true, menu_sort, true, Hit::Sort);
        let fw = if compact { 34.0 } else { 10.0 + 14.0 + 8.0 + self.gfx.measure("Free up memory", Font::Ui, 13.0, 400) + 10.0 };
        rx -= fw.ceil() + 2.0;
        let free_on = self.pane.as_ref().map(|p| p.kind == PaneKind::Free).unwrap_or(false);
        self.cmd_btn(rx, y, lab("Free up memory"), 0xEA99, false, free_on, true, Hit::Free);
        if compact {
            self.tip(Rect::new(rx, y, fw, 32.0), "Free up memory");
        }
        let count = if self.searching() {
            let n = self.rows.iter().filter(|r| matches!(r.k, RowK::Group(_) | RowK::Child(..) | RowK::Startup(_) | RowK::Service(_))).count();
            let n = n - self.rows.iter().filter(|r| matches!(r.k, RowK::Group(gi) if self.groups[gi].members.len() > 1)).count();
            if n == 1 { "1 match".to_string() } else { format!("{n} matches") }
        } else {
            format!("{} processes", self.procs.len().saturating_sub(1))
        };
        let cw = self.gfx.measure(&count, Font::Ui, 12.0, 400);
        // the count only where it fits between the two button groups
        if rx - 8.0 - cw - 2.0 > left_end + 12.0 {
            self.gfx.text(&count, Rect::new(rx - 8.0 - cw - 2.0, y, cw + 2.0, 32.0), Font::Ui, 12.0, 400, Align::Right, t.t3);
        }
    }

    /// List columns for a list of width `w` (content box x0..x1).
    /// Is the side pane drawn over the list (content narrower than 800)?
    pub fn pane_overlay(&self) -> bool {
        self.w - self.cx0() - 12.0 < 800.0
    }

    pub fn columns(&self, x0: f32, x1: f32) -> Vec<(Col, f32, f32)> {
        let pane = self.pane.is_some() && !self.pane_overlay();
        let narrow = self.narrow();
        let mut right: Vec<(Col, f32)> = vec![];
        let res: Vec<Res> = if pane { vec![Res::Cpu, Res::Mem] } else if narrow { vec![Res::Cpu, Res::Mem, Res::Disk] } else { RES_ALL.to_vec() };
        // a narrow list with the pane open drops the Type column so names stay readable
        if self.prefs.col_type && !(pane && x1 - x0 < 560.0) {
            right.push((Col::Type, 100.0));
        }
        if self.prefs.show_pid {
            right.push((Col::Pid, 70.0));
        }
        let res: Vec<Res> = res
            .into_iter()
            .filter(|r| match r {
                Res::Disk => self.prefs.col_disk,
                // no per-app network counter without admin / ETW: the column is hidden until there is one
                Res::Net => false,
                Res::Gpu => self.prefs.col_gpu,
                _ => true,
            })
            .collect();
        for r in res {
            right.push((
                Col::M(r),
                match r {
                    Res::Cpu => 104.0,
                    Res::Mem => 124.0,
                    Res::Disk | Res::Net => 112.0,
                    Res::Gpu => 84.0,
                },
            ));
        }
        // a small window drops columns (GPU, PID, I/O, then Type) before the name gets too narrow
        for drop in [Col::M(Res::Gpu), Col::Pid, Col::M(Res::Disk), Col::Type] {
            if x1 - x0 - right.iter().map(|c| c.1).sum::<f32>() >= 200.0 {
                break;
            }
            right.retain(|c| c.0 != drop);
        }
        let fixed: f32 = right.iter().map(|c| c.1).sum();
        let mut out = vec![(Col::Name, x0, (x1 - x0 - fixed).max(120.0))];
        let mut x = x0 + out[0].2;
        for (c, w) in right {
            out.push((c, x, w));
            x += w;
        }
        out
    }

    fn sort_col(&self) -> Option<Col> {
        match self.sort {
            SortKey::Name => Some(Col::Name),
            SortKey::Cpu => Some(Col::M(Res::Cpu)),
            SortKey::Mem => Some(Col::M(Res::Mem)),
            SortKey::Disk => Some(Col::M(Res::Disk)),
            SortKey::Net => Some(Col::M(Res::Net)),
            SortKey::Gpu => Some(Col::M(Res::Gpu)),
            SortKey::Impact => None,
        }
    }

    fn paint_list(&mut self, list: Rect) {
        let t = self.t;
        let cols = self.columns(list.x + 8.0, list.r() - 12.0);
        // header
        let hy = list.y;
        self.gfx.hline(list.x, list.r(), hy, t.line);
        self.gfx.hline(list.x, list.r(), hy + HEAD_H - 1.0, t.line);
        let sorted = self.sort_col();
        for (c, x, w) in &cols {
            let (label, right, idx) = match c {
                Col::Name => ("Name", false, 0u8),
                Col::Type => ("Type", false, 255),
                Col::Pid => ("PID", true, 255),
                Col::M(Res::Cpu) => ("CPU", true, 1),
                Col::M(Res::Mem) => ("Memory", true, 2),
                Col::M(Res::Disk) => ("I/O", true, 3),
                Col::M(Res::Net) => ("Network", true, 4),
                Col::M(Res::Gpu) => ("GPU", true, 5),
            };
            let on = sorted == Some(*c);
            let fg = if on { t.t1 } else { t.t2 };
            let lw = self.gfx.measure(label, Font::Ui, 12.0, 400);
            let r = Rect::new(*x, hy + 1.0, *w, HEAD_H - 2.0);
            if idx != 255 {
                let h = Hit::Header(idx);
                let hv = self.hov(h);
                if hv > 0.0 {
                    self.gfx.fill(r, t.hov.fade(0.6 * hv));
                }
                self.hit(r, h);
            }
            let chev = if on { 12.0 } else { 0.0 };
            let lx = if right { x + w - lw - chev } else if *c == Col::Name { x + 46.0 } else { *x };
            self.gfx.text(label, Rect::new(lx, hy, lw + 2.0, HEAD_H), Font::Ui, 12.0, 400, Align::Left, fg);
            if on {
                self.gfx.glyph(if self.sort_rev { 0xE70E } else { 0xE70D }, Rect::new(lx + lw + 4.0, hy + 1.0, 8.0, HEAD_H), 8.0, t.t1);
            }
            // "since boot" until the second CPU sample
            if *c == Col::M(Res::Cpu) && self.tot[0].len() < 2 && !self.procs.is_empty() {
                let sb = "since boot";
                let sbw = self.gfx.measure(sb, Font::Ui, 12.0, 400);
                self.gfx.text(sb, Rect::new(lx - sbw - 4.0, hy, sbw + 2.0, HEAD_H), Font::Ui, 12.0, 400, Align::Left, t.t3);
            }
        }
        // rows
        let view = Rect::new(list.x, hy + HEAD_H, list.w, list.h - HEAD_H);
        self.list_rect = view;
        // the list as one keyboard stop (rows drawn after it take the clicks)
        self.hit(view, Hit::List);
        self.view_h[0] = view.h;
        self.clamp_scroll();
        if std::mem::take(&mut self.want_scroll_sel)
            && let Some(i) = self.rows.iter().position(|r| self.row_sel(r.k))
        {
            // centre the selected row
            let v = self.rows[i].y - view.h / 2.0;
            self.set_scroll(0, v);
        }
        self.gfx.clip(view);
        // the list blocks what is under it, but its rows (added after this) take the clicks
        self.hit(view, Hit::Block);
        let rows = std::mem::take(&mut self.rows);
        let maxes = self.col_max();
        // selection highlight: drawn under the rows, it glides (spring) from the old row to the new one
        if let Some(row) = rows.iter().find(|r| self.row_sel(r.k)) {
            let id = self.row_id(row.k);
            let (ry, _) = self.row_motion(id, row.y);
            self.sel_highlight(Rect::new(view.x + 4.0, view.y - self.scroll[0], view.w - 8.0, row.h), id, ry, 0x6100);
        } else {
            self.mo.borrow_mut().sel_prev = None;
        }
        for (i, row) in rows.iter().enumerate() {
            let (ry, a) = self.row_motion(self.row_id(row.k), row.y);
            let y = view.y + ry - self.scroll[0];
            if y + row.h < view.y || y > view.b() {
                continue;
            }
            let r = Rect::new(view.x + 4.0, y, view.w - 8.0, row.h);
            let old = self.gfx.push_alpha(a);
            self.paint_row(i, row.k, r, &cols, &maxes);
            self.gfx.pop_alpha(old);
        }
        self.painted = rows.iter().map(|r| self.row_id(r.k)).collect();
        self.rows = rows;
        if self.rows.is_empty() && !self.procs.is_empty() && self.searching() {
            let msg = format!("Nothing matches \"{}\"", self.query.trim());
            self.gfx.text(&msg, Rect::new(view.x, view.y + 30.0, view.w, 24.0), Font::Ui, 13.0, 400, Align::Center, t.t2);
            self.gfx.text("Search looks in names, descriptions, paths, command lines, startup apps and services.", Rect::new(view.x, view.y + 54.0, view.w, 20.0), Font::Ui, 12.0, 400, Align::Center, t.t3);
        }
        self.gfx.unclip();
        self.scrollbar(view, 0);
    }

    /// Column maxima (bars are scaled to the biggest value in their column).
    fn col_max(&self) -> [f64; 5] {
        let mut m = [0.0f64; 5];
        for g in &self.groups {
            for (i, r) in RES_ALL.iter().enumerate() {
                m[i] = m[i].max(g.value(*r, 0.0));
            }
        }
        m
    }

    fn lead(&self, cpu: f32, mem: u64, io: f64, gpu: f32) -> Option<Res> {
        let total = self.perf.mem_total.max(1) as f64;
        let shares = [(Res::Cpu, cpu as f64 / 100.0), (Res::Mem, mem as f64 / total), (Res::Disk, io / (20.0 * 1048576.0)), (Res::Gpu, gpu as f64 / 100.0)];
        let (r, s) = shares.iter().copied().fold((Res::Mem, 0.0015), |a, b| if b.1 > a.1 { b } else { a });
        if s > 0.0015 { Some(r) } else { None }
    }

    fn paint_row(&mut self, i: usize, k: RowK, r: Rect, cols: &[(Col, f32, f32)], maxes: &[f64; 5]) {
        let t = self.t;
        match k {
            RowK::PinSep => {
                self.gfx.hline(r.x + 8.0, r.r() - 8.0, r.y + 4.0, t.line);
                return;
            }
            RowK::Sec(si, n) => {
                let label = ["Processes", "Startup apps", "Services"][si as usize];
                let w = self.gfx.text_at(label, r.x + 12.0, r.y, r.h, 200.0, Font::Ui, 12.0, 600, t.t2);
                self.gfx.text_at(&n.to_string(), r.x + 12.0 + w + 8.0, r.y, r.h, 60.0, Font::Ui, 12.0, 400, t.t3);
                return;
            }
            RowK::Kind(si, n) => return self.paint_section(si, n, r, cols),
            RowK::More(si, n) => {
                let h = Hit::Row(i);
                let label = if n > 0 { format!("Show all ({n})") } else { "Show fewer".to_string() };
                let x = cols[0].1 + 46.0;
                let lw = self.gfx.measure(&label, Font::Ui, 12.5, 600);
                let br = Rect::new(x - 8.0, r.y + 3.0, lw + 16.0 + 16.0, r.h - 6.0);
                self.hp(br, 4.0, h);
                let pressed = self.press_begin(br, h);
                self.gfx.text(&label, Rect::new(x, r.y, lw + 2.0, r.h), Font::Ui, 12.5, 600, Align::Left, t.acc_t);
                self.gfx.glyph(if n > 0 { 0xE70D } else { 0xE70E }, Rect::new(x + lw + 6.0, r.y + 1.0, 9.0, r.h), 9.0, t.acc_t);
                self.press_end(pressed);
                self.hit(br, h);
                let _ = si;
                return;
            }
            RowK::Startup(si) => return self.paint_search_startup(i, si, r),
            RowK::Service(si) => return self.paint_search_service(i, si, r),
            _ => {}
        }
        let h = Hit::Row(i);
        if !self.row_sel(k) {
            self.hp(r, 4.0, h);
        }
        self.hit(r, h);
        let (gi, child) = match k {
            RowK::Group(gi) => (gi, None),
            RowK::Child(gi, mi) => (gi, Some(mi)),
            _ => return,
        };
        let g = self.groups[gi].clone();
        let (cpu, mem, io, gpu, pid) = match child {
            Some(mi) => {
                let m = &g.members[mi];
                (m.cpu, m.mem, m.io, m.gpu, m.pid)
            }
            None => (g.cpu, g.mem, g.io, g.gpu, g.members.first().map(|m| m.pid).unwrap_or(0)),
        };
        // numbers change in place; only the small bars ease (150 ms)
        let rid = self.row_id(k);
        let flag = if child.is_none() { self.flag(gi) } else { None };
        // ---- name cell
        let (nc_x, nc_w) = (cols[0].1, cols[0].2);
        let mut x = nc_x;
        let multi = g.members.len() > 1;
        if child.is_none() && multi {
            let open = self.expanded.contains(&g.key) || (self.searching() && self.rows.get(i + 1).map(|r| matches!(r.k, RowK::Child(..))).unwrap_or(false));
            let cr = Rect::new(x, r.y, 20.0, r.h);
            self.gfx.glyph(if open { 0xE70D } else { 0xE76C }, cr, 9.0, t.t2);
            self.hit(Rect::new(r.x, r.y, 24.0 + (x - r.x), r.h), Hit::Chevron(i));
        }
        x += 20.0;
        if child.is_some() {
            x += 20.0;
        }
        let path = g.path.clone();
        self.draw_icon(&path, Rect::new(x, r.cy() - 8.0, 16.0, 16.0), 0xE737);
        x += 16.0 + 10.0;
        let limit = nc_x + nc_w - 12.0;
        let show_exe = child.is_none() && self.pane.is_none() && !self.narrow() && flag.is_none() && !g.exe.eq_ignore_ascii_case(&g.label) && !g.label.starts_with("Service Host");
        // fixed parts after the name
        let count = if child.is_none() && multi { format!("({})", g.members.len()) } else { String::new() };
        let cw = if count.is_empty() { 0.0 } else { 5.0 + self.gfx.measure(&count, Font::Ui, 13.0, 400) };
        let lockw = if child.is_none() && g.sys { 7.0 + 11.0 } else { 0.0 };
        let flagw = flag.as_ref().map(|(_, s)| 10.0 + 16.0 + 5.0 + 9.5 + self.gfx.measure(s, Font::Ui, 11.5, 400)).unwrap_or(0.0);
        let (name, sub) = match child {
            Some(mi) => (g.members[mi].name.clone(), g.members[mi].sub.clone()),
            None => (g.label.clone(), if show_exe { g.exe.clone() } else { String::new() }),
        };
        let name_max = (limit - x - cw - lockw - flagw - if sub.is_empty() { 0.0 } else { 40.0 }).max(24.0);
        let nw = self.text_marked(&name, x, r.y, r.h, name_max, 13.0, t.t1);
        // cut text gets a tooltip (the drawn width reaching the room means it was cut; no extra measuring)
        let name_x = x;
        let mut cut = nw >= name_max - 0.5;
        x += nw;
        if !count.is_empty() {
            self.gfx.text_at(&count, x + 5.0, r.y, r.h, 60.0, Font::Ui, 13.0, 400, t.t2);
            x += cw;
        }
        if lockw > 0.0 {
            self.gfx.glyph(0xE72E, Rect::new(x + 7.0, r.y, 11.0, r.h), 11.0, t.t3);
            x += lockw;
        }
        if !sub.is_empty() && limit - x - 8.0 > 20.0 {
            let room = limit - x - 8.0 - flagw;
            let sw = self.text_marked(&sub, x + 8.0, r.y + 0.5, r.h, room, 12.0, t.t3);
            cut |= sw >= room - 0.5;
            x += 8.0 + sw;
        } else if !sub.is_empty() {
            cut = true;
        }
        if cut {
            let full = if sub.is_empty() || child.is_some() { name.clone() } else { format!("{name} · {sub}") };
            self.tip(Rect::new(name_x, r.y, limit - name_x, r.h), full);
        }
        if let Some((gl, s)) = &flag {
            self.pill(x + 10.0, r.cy(), s, *gl, t.warn, t.warn_bg, 20.0, 11.5, 400);
        }
        // ---- other cells
        let sorted = self.sort_col();
        let lead = self.lead(cpu, mem, io, gpu);
        let growing = flag.as_ref().map(|f| f.0 == 0xE70E).unwrap_or(false) || (child.is_some() && self.flag(gi).map(|f| f.0 == 0xE70E).unwrap_or(false));
        let hist = match child {
            Some(mi) => self.child_hist.get(&g.members[mi].pid).cloned(),
            None => self.hist.get(&g.key).cloned(),
        };
        for (c, cx, cw2) in cols.iter().skip(1) {
            let cell = Rect::new(*cx, r.y, *cw2, r.h);
            match c {
                Col::Type => {
                    if child.is_none() {
                        // on the selected row the tag needs more contrast than the usual tint
                        let bg = if self.row_sel(k) { t.t1.a(if t.dark { 0.14 } else { 0.10 }) } else { t.tag };
                        self.pill(cell.x, r.cy(), g.type_label(), 0, t.t2, bg, 20.0, 11.5, 400);
                    }
                }
                Col::Pid => {
                    let s = if child.is_some() || !multi { pid.to_string() } else { String::new() };
                    self.gfx.text(&s, cell, Font::Ui, 13.0, 400, Align::Right, t.t2);
                }
                Col::M(res) => {
                    let v = match res {
                        Res::Cpu => cpu as f64,
                        Res::Mem => mem as f64,
                        Res::Disk => io,
                        Res::Gpu => gpu as f64,
                        Res::Net => -1.0,
                    };
                    let (txt, zero) = match res {
                        Res::Cpu => (model::fmt_pct(cpu), cpu < 0.05),
                        Res::Mem => (model::fmt_mem(mem), mem == 0),
                        Res::Disk => (model::fmt_rate(io), io < 1.0),
                        Res::Gpu => (format!("{:.0}%", gpu), gpu < 0.5),
                        Res::Net => ("–".to_string(), true),
                    };
                    let vw = self.gfx.measure(&txt, Font::Ui, 13.0, 400);
                    self.gfx.text(&txt, cell, Font::Ui, 13.0, 400, Align::Right, if zero { t.t3 } else { t.t1 });
                    if zero {
                        continue;
                    }
                    let color = self.res_color(*res);
                    let moving = match res {
                        Res::Cpu => cpu >= 1.0,
                        Res::Gpu => gpu >= 2.0,
                        Res::Disk => io >= 500.0 * 1024.0,
                        Res::Mem => growing,
                        Res::Net => false,
                    };
                    let spark = match sorted {
                        Some(Col::M(s)) => s == *res,
                        Some(_) => false,
                        None => lead == Some(*res) && moving,
                    };
                    let series = hist.as_ref().map(|h| h.series(*res)).unwrap_or_default();
                    let vr = cell.r() - vw - 8.0;
                    if spark && series.len() >= 2 {
                        let peak = series.iter().copied().fold(0.0f32, f32::max);
                        self.gfx.spark(Rect::new(vr - 44.0, r.cy() - 8.0, 44.0, 16.0), &series, peak * 1.15, color, 0.18, 1.4);
                    } else {
                        // scaled to the column's biggest value (with a floor, so 0.4% CPU is not a full bar)
                        let floor = match res {
                            Res::Cpu | Res::Gpu => 5.0,
                            Res::Mem => 256.0 * 1048576.0,
                            _ => 1048576.0,
                        };
                        let mx = maxes[RES_ALL.iter().position(|x| x == res).unwrap()].max(floor);
                        let f = self.tv(rid, 10 + RES_ALL.iter().position(|x| x == res).unwrap() as u8, ((v / mx) as f32).clamp(0.06, 1.0));
                        let br = Rect::new(vr - 36.0, r.cy() - 1.5, 36.0, 3.0);
                        self.gfx.fill_round(br, 1.5, t.track);
                        self.gfx.fill_round(Rect::new(br.x, br.y, (36.0 * f).max(3.0), 3.0), 1.5, color);
                    }
                }
                Col::Name => {}
            }
        }
    }

    /// Section header of the grouped list: name + count, and the section's total CPU and memory in their columns.
    fn paint_section(&mut self, si: u8, n: usize, r: Rect, cols: &[(Col, f32, f32)]) {
        let t = self.t;
        let kind = [Kind::App, Kind::Background, Kind::Windows][si as usize % 3];
        let label = ["Apps", "Background processes", "Windows processes"][si as usize % 3];
        let x = cols[0].1 + 20.0;
        let w = self.gfx.text_at(label, x, r.y + 4.0, r.h - 4.0, 300.0, Font::Ui, 13.0, 600, t.t1);
        self.gfx.text_at(&format!("({n})"), x + w + 6.0, r.y + 4.0, r.h - 4.0, 80.0, Font::Ui, 13.0, 400, t.t2);
        let (cpu, mem) = self.groups.iter().filter(|g| g.kind == kind).fold((0.0f32, 0u64), |a, g| (a.0 + g.cpu, a.1 + g.mem));
        for (c, cx, cw) in cols.iter().skip(1) {
            let s = match c {
                Col::M(Res::Cpu) => model::fmt_pct(cpu),
                Col::M(Res::Mem) => model::fmt_mem(mem),
                _ => continue,
            };
            self.gfx.text(&s, Rect::new(*cx, r.y + 4.0, *cw, r.h - 4.0), Font::Ui, 12.5, 600, Align::Right, t.t2);
        }
        self.gfx.hline(r.x + 8.0, r.r() - 8.0, r.b() - 1.0, t.line);
    }

    fn paint_search_startup(&mut self, i: usize, si: usize, r: Rect) {
        let t = self.t;
        let Some(e) = self.startup.get(si).cloned() else { return };
        let h = Hit::Row(i);
        if !self.row_sel(RowK::Startup(si)) {
            self.hp(r, 4.0, h);
        }
        self.hit(r, h);
        let mut x = r.x + 24.0;
        let exe = exe_of(&e.command);
        self.draw_icon(&exe, Rect::new(x, r.cy() - 8.0, 16.0, 16.0), if e.source == "Service" { 0xE713 } else { 0xE8A5 });
        x += 26.0;
        let right = r.r() - 12.0;
        let tog_w = 76.0;
        let src_w = 140.0;
        let nw = self.text_marked(&e.name, x, r.y, r.h, 230.0, 13.0, t.t1);
        x += nw + 10.0;
        let room = right - tog_w - 16.0 - src_w - x;
        if room > 30.0 {
            let keep = keep_from(&e.command);
            let s = self.gfx.elide_mid(&e.command, keep, room, Font::Ui, 12.0, 400);
            self.text_marked(&s, x, r.y + 0.5, r.h, room, 12.0, t.t3);
        }
        self.gfx.text(source_label(e.source), Rect::new(right - tog_w - 16.0 - src_w, r.y, src_w, r.h), Font::Ui, 12.0, 400, Align::Right, t.t2);
        if e.source != "Service" {
            self.toggle(right, r.cy(), e.enabled, Hit::SrStartupToggle(si), true);
        }
    }

    fn paint_search_service(&mut self, i: usize, si: usize, r: Rect) {
        let t = self.t;
        let Some(s) = self.services_all.get(si).cloned() else { return };
        let h = Hit::Row(i);
        if !self.row_sel(RowK::Service(si)) {
            self.hp(r, 4.0, h);
        }
        self.hit(r, h);
        let mut x = r.x + 24.0;
        self.gfx.glyph(0xE713, Rect::new(x, r.y, 16.0, r.h), 15.0, t.t2);
        x += 26.0;
        let right = r.r() - 12.0;
        let nw = self.text_marked(&s.display, x, r.y, r.h, 300.0, 13.0, t.t1);
        x += nw + 10.0;
        let btn_x = right - 64.0;
        let start_x = btn_x - 16.0 - 140.0;
        let state_x = start_x - 96.0;
        if state_x - x > 30.0 {
            self.text_marked(&s.name, x, r.y + 0.5, r.h, state_x - x - 10.0, 12.0, t.t3);
        }
        let state = match s.state {
            "running" => "Running",
            "stopped" => "Stopped",
            other => other,
        };
        self.gfx.text(state, Rect::new(state_x, r.y, 96.0, r.h), Font::Ui, 12.0, 400, Align::Left, t.t2);
        let st = self.service_start_type(&s.name);
        self.gfx.text(&st, Rect::new(start_x, r.y, 140.0, r.h), Font::Ui, 12.0, 400, Align::Left, t.t3);
        let label = if s.state == "running" { "Stop" } else { "Start" };
        self.button(Rect::new(btn_x, r.cy() - 13.0, 64.0, 26.0), label, 0, Btn::Std, true, true, Hit::SrServiceBtn(si));
    }

    fn service_start_type(&self, name: &str) -> String {
        thread_local! {
            static CACHE: std::cell::RefCell<std::collections::HashMap<String, String>> = Default::default();
        }
        CACHE.with(|c| {
            if let Some(v) = c.borrow().get(name) {
                return v.clone();
            }
            let v = match crate::services::details(name).map(|d| d.0) {
                Some(s) if s == "auto" => "Automatic".to_string(),
                Some(s) if s == "manual" => "Manual".to_string(),
                Some(s) if s == "disabled" => "Disabled".to_string(),
                Some(s) => s,
                None => String::new(),
            };
            c.borrow_mut().insert(name.to_string(), v.clone());
            v
        })
    }
}

/// The program a startup command runs ("C:\x\y.exe" -a -> C:\x\y.exe).
pub fn exe_of(cmd: &str) -> String {
    let c = cmd.trim();
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("").to_string();
    }
    let low = c.to_lowercase();
    if let Some(i) = low.find(".exe") {
        return c[..i + 4].to_string();
    }
    c.to_string()
}

/// Where the part that must stay visible starts: the file name of the program and its arguments.
pub fn keep_from(cmd: &str) -> usize {
    let exe = exe_of(cmd);
    let file_start = exe.rfind(['\\', '/']).map(|i| i + 1).unwrap_or(0);
    let off = if cmd.trim_start().starts_with('"') { 1 } else { 0 };
    cmd[..(file_start + off).min(cmd.len())].chars().count()
}
