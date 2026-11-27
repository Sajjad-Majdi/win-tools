//! Quick settings: the panel under the sliders button at the right of the address bar (toolbar "compact" mode).
//! The most used settings in one place, set without leaving the folder: Layout (Details / Grid tiles, and a
//! continuous size slider for Grid tiles), Sort by
//! (one segmented row plus a direction button), the Details pane, hidden items and file name extensions (switches),
//! and "All settings". A change applies at once and the panel stays open; Esc or a click outside closes it.
//! Arrow keys and Tab move between the controls, Enter or Space uses one.
use crate::app::*;
use crate::gfx::{Align, Font, Rect};
use crate::model::*;
use crate::look::{GAP_MAX, GAP_MIN, GRID_MAX, GRID_MIN, LIST_MAX, LIST_MIN};
use crate::theme::{R_CTRL, R_MENU, Theme};
use std::time::Instant;

/// The panel's state while it is open.
pub struct Quick {
    pub at: Instant,
    /// the control the keyboard is on (shown with a focus ring once a key was used)
    pub focus: u8,
    pub kb: bool,
}

/// Control ids (Hit::Qs): layout tiles, sort fields, sort direction, switches, All settings.
pub const Q_DETAILS: u8 = 0;
pub const Q_GRID: u8 = 1;
pub const Q_SIZE: u8 = 2;
pub const Q_GAP: u8 = 3;
pub const Q_SORT: u8 = 10; // 10..=13: Name, Date, Type, Size
pub const Q_DIR: u8 = 14;
pub const Q_PANE: u8 = 20;
pub const Q_HIDDEN: u8 = 21;
pub const Q_EXT: u8 = 22;
pub const Q_ALL: u8 = 30;
const ORDER: [u8; 13] = [Q_DETAILS, Q_GRID, Q_SIZE, Q_GAP, Q_SORT, Q_SORT + 1, Q_SORT + 2, Q_SORT + 3, Q_DIR, Q_PANE, Q_HIDDEN, Q_EXT, Q_ALL];
const SORTS: [(&str, SortCol); 4] = [("Name", SortCol::Name), ("Date", SortCol::Date), ("Type", SortCol::Type), ("Size", SortCol::Size)];
const W: f32 = 300.0;
/// the size slider's row
pub const SIZE_H: f32 = 28.0;
const PAD: f32 = 12.0;

impl App {
    /// Settings and Clean up have no layout; This PC, Settings and Clean up keep their own order.
    fn qs_layout_on(&self) -> bool {
        !matches!(self.pane().loc, Loc::Settings | Loc::Cleanup)
    }
    fn qs_sort_on(&self) -> bool {
        !matches!(self.pane().loc, Loc::Settings | Loc::Cleanup | Loc::ThisPc)
    }
    /// The size and spacing sliders work in a folder (This PC keeps its own tiles), for the layout shown: Grid and
    /// Details each keep their own values.
    pub fn qs_size_on(&self) -> bool {
        self.qs_layout_on() && self.pane().loc != Loc::ThisPc
    }

    /// (smallest, largest, current) value of slider `k` (Q_SIZE or Q_GAP) for the layout shown.
    fn slider_range(&self, k: u8) -> (f32, f32, f32) {
        let grid = self.tab().view == View::Grid;
        match (k, grid) {
            (Q_GAP, true) => (GAP_MIN, GAP_MAX, self.look.grid_gap),
            (Q_GAP, false) => (GAP_MIN, GAP_MAX, self.look.list_gap),
            (_, true) => (GRID_MIN, GRID_MAX, self.look.grid),
            _ => (LIST_MIN, LIST_MAX, self.look.list),
        }
    }

    /// Slider `k` (Q_SIZE: item size, Q_GAP: space between items) of the layout shown (Grid and Details apart): a
    /// small and a large sign at the ends of a continuous track, with a mark at the standard value (the knob snaps
    /// to it). Used by Quick settings (`ring`: its keyboard focus ring) and by the Command bar's View menu.
    pub fn paint_slider(&mut self, k: u8, row: Rect, ring: bool, t: &Theme) {
        let on = self.qs_size_on();
        let h = Hit::Qs(k);
        let hk = if on { self.hk(h) } else { 0.0 };
        let fg = if on { t.text2 } else { t.text_dis };
        if k == Q_GAP {
            // two small squares: close together at the left end, apart at the right end
            let cy = row.cy();
            for (x, gap) in [(row.x + 2.0, 2.0), (row.r() - 18.0, 8.0)] {
                self.gfx.fill_round(Rect::new(x, cy - 2.5, 5.0, 5.0), 1.0, fg);
                self.gfx.fill_round(Rect::new(x + 5.0 + gap, cy - 2.5, 5.0, 5.0), 1.0, fg);
            }
        } else {
            let sg = if self.tab().view == View::Grid { 0xF0E2 } else { 0xE8FD };
            self.gfx.glyph(sg, Rect::new(row.x, row.y, 16.0, SIZE_H), 10.0, fg);
            self.gfx.glyph(sg, Rect::new(row.r() - 18.0, row.y, 18.0, SIZE_H), 16.0, fg);
        }
        let track = Rect::new(row.x + 26.0, row.cy() - 2.0, row.w - 26.0 - 28.0, 4.0);
        let (lo, hi, cur) = self.slider_range(k);
        let f = ((cur - lo) / (hi - lo)).clamp(0.0, 1.0);
        let kx = track.x + f * track.w;
        self.gfx.fill_round(track, 2.0, if t.dark { t.text.a(0.16) } else { t.field_bottom.a(0.45) });
        if on {
            self.gfx.fill_round(Rect::new(track.x, track.y, kx - track.x, track.h), 2.0, t.accent);
        }
        let one = track.x + (1.0 - lo) / (hi - lo) * track.w;
        self.gfx.fill_round(Rect::new(one - 1.0, track.y - 5.0, 2.0, track.h + 10.0), 1.0, fg);
        // the knob: a ring in the page colour with an accent dot that grows under the pointer or while dragged
        let grab = self.qs_drag == Some(k) || hk > 0.5;
        self.gfx.ellipse(kx, track.cy(), 9.0, t.layer2, true, 1.0);
        self.gfx.ellipse(kx, track.cy(), 9.0, t.stroke, false, 1.0);
        self.gfx.ellipse(kx, track.cy(), if grab { 6.0 } else { 4.5 }, if on { t.accent } else { t.text_dis }, true, 1.0);
        if ring {
            self.qs_ring(k, Rect::new(track.x - 10.0, row.y + 2.0, track.w + 20.0, SIZE_H - 4.0), R_CTRL, t);
        }
        if k == Q_GAP {
            self.gap_track = track;
        } else {
            self.size_track = track;
        }
        if on {
            self.hit(Rect::new(track.x - 10.0, row.y, track.w + 20.0, SIZE_H), h);
        }
    }

    /// Slider `k` from a point on its track (a press or a drag), live. Near the standard mark it snaps to it, so the
    /// default is easy to get back.
    pub fn slider_at(&mut self, k: u8, x: f32) {
        let track = if k == Q_GAP { self.gap_track } else { self.size_track };
        if !self.qs_size_on() || track.w <= 0.0 {
            return;
        }
        let (lo, hi, _) = self.slider_range(k);
        let f = ((x - track.x) / track.w).clamp(0.0, 1.0);
        let z = lo + f * (hi - lo);
        let one = track.x + (1.0 - lo) / (hi - lo) * track.w;
        self.set_slider(k, if (x - one).abs() <= 8.0 { 1.0 } else { z });
    }

    /// Set slider `k` for the layout shown (clamped); the folder re-flows at once. Saved by the caller when the
    /// change ends (`grid_size_done`).
    pub fn set_slider(&mut self, k: u8, z: f32) {
        let (lo, hi, cur) = self.slider_range(k);
        let z = z.clamp(lo, hi);
        if (z - cur).abs() > f32::EPSILON {
            let grid = self.tab().view == View::Grid;
            *match (k, grid) {
                (Q_GAP, true) => &mut self.look.grid_gap,
                (Q_GAP, false) => &mut self.look.list_gap,
                (_, true) => &mut self.look.grid,
                _ => &mut self.look.list,
            } = z;
            self.invalidate();
        }
    }

    fn qs_enabled(&self, id: u8) -> bool {
        match id {
            Q_DETAILS | Q_GRID => self.qs_layout_on(),
            Q_SIZE | Q_GAP => self.qs_size_on(),
            Q_SORT..=Q_DIR => self.qs_sort_on(),
            _ => true,
        }
    }

    pub fn qs_toggle(&mut self) {
        if self.qs.is_some() {
            self.qs = None;
        } else {
            self.close_popups();
            let first = ORDER.iter().copied().find(|i| self.qs_enabled(*i)).unwrap_or(Q_PANE);
            self.qs = Some(Quick { at: Instant::now(), focus: first, kb: false });
        }
        self.anim();
        self.invalidate();
    }

    /// The panel's rectangle (under its button, inside the window).
    pub fn qs_rect(&self) -> Rect {
        let btn = self.hits.iter().rev().find(|h| h.1 == Hit::QuickBtn).map(|h| h.0).unwrap_or(Rect::new(self.w - 44.0, TITLE_H + 8.0, 34.0, 32.0));
        let w = W.min(self.w - 16.0).max(200.0);
        let x = (btn.r() - w).clamp(8.0, (self.w - w - 8.0).max(8.0));
        Rect::new(x, btn.b() + 6.0, w, self.qs_height())
    }

    fn qs_height(&self) -> f32 {
        // Layout (tiles + size slider), Sort by, three switches, the footer
        let layout = PAD + 18.0 + 64.0 + 8.0 + SIZE_H * 2.0 + PAD;
        let sort = PAD + 18.0 + 32.0 + PAD;
        let switches = 6.0 + 3.0 * 36.0 + 6.0;
        let footer = 4.0 + 36.0 + 4.0;
        layout + 1.0 + sort + 1.0 + switches + 1.0 + footer
    }

    /// Use a control (click, Enter or Space). The panel stays open, except for "All settings".
    pub fn qs_use(&mut self, id: u8) {
        if !self.qs_enabled(id) {
            return;
        }
        if let Some(q) = self.qs.as_mut() {
            q.focus = id;
        }
        match id {
            Q_DETAILS => self.set_view(View::Details),
            Q_GRID => self.set_view(View::Grid),
            Q_DIR => {
                let pane = self.pane_mut();
                pane.sort.1 = !pane.sort.1;
                pane.sort_items();
            }
            k if (Q_SORT..Q_SORT + 4).contains(&k) => {
                let col = SORTS[(k - Q_SORT) as usize].1;
                let pane = self.pane_mut();
                pane.sort = (col, pane.sort.1);
                pane.sort_items();
            }
            Q_PANE => self.act(Act::DetailsPane),
            Q_HIDDEN => self.act(Act::ToggleHidden),
            Q_EXT => {
                self.ui.ext = !self.ui.ext;
                self.save_ui();
            }
            Q_ALL => {
                self.qs = None;
                self.act(Act::Settings);
            }
            _ => {}
        }
        self.invalidate();
    }

    /// Keys while the panel is open: true when used.
    pub fn qs_key(&mut self, vk: u16, shift: bool) -> bool {
        let Some(focus) = self.qs.as_ref().map(|q| q.focus) else { return false };
        // Left / Right on the size slider change the size (Up / Down move on)
        if matches!(focus, Q_SIZE | Q_GAP) && matches!(vk, 0x25 | 0x27) {
            let step = if vk == 0x25 { -0.1 } else { 0.1 };
            let cur = self.slider_range(focus).2;
            let next = cur + step;
            // a step across the standard value stops on it
            self.set_slider(focus, if (cur - 1.0) * (next - 1.0) < 0.0 { 1.0 } else { next });
            self.grid_size_done();
            if let Some(q) = self.qs.as_mut() {
                q.kb = true;
            }
            self.invalidate();
            return true;
        }
        let on: Vec<u8> = ORDER.iter().copied().filter(|i| self.qs_enabled(*i)).collect();
        let at = on.iter().position(|i| *i == focus).unwrap_or(0);
        let step = |d: isize| on[((at as isize + d).rem_euclid(on.len() as isize)) as usize];
        let next = match vk {
            0x1B => {
                self.qs = None;
                self.invalidate();
                return true;
            }
            0x0D | 0x20 => {
                let f = focus;
                if let Some(q) = self.qs.as_mut() {
                    q.kb = true;
                }
                self.qs_use(f);
                return true;
            }
            0x09 => step(if shift { -1 } else { 1 }),
            0x25 | 0x26 => step(-1),
            0x27 | 0x28 => step(1),
            _ => return true,
        };
        if let Some(q) = self.qs.as_mut() {
            q.focus = next;
            q.kb = true;
        }
        self.invalidate();
        true
    }

    fn qs_ring(&mut self, id: u8, r: Rect, rad: f32, t: &Theme) {
        if self.qs.as_ref().is_some_and(|q| q.kb && q.focus == id) {
            self.gfx.stroke_round(r.inset(-2.0, -2.0), rad + 2.0, t.text, 2.0);
        }
    }

    fn qs_title(&mut self, s: &str, x: f32, y: f32, w: f32, t: &Theme) {
        self.gfx.text(s, Rect::new(x, y, w, 16.0), Font::Ui, 11.5, 600, Align::Left, t.text3);
    }

    pub fn paint_quick(&mut self, t: &Theme) {
        if self.qs.is_none() {
            return;
        }
        let r = self.qs_rect();
        let at = self.qs.as_ref().unwrap().at;
        let k = crate::paint::prog(at, 120.0, self.shooting);
        let dy = -(1.0 - k) * 6.0;
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), k.max(0.02));
        self.gfx.push_scale(r.x, r.y, 1.0, 0.0, dy);
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        let (x0, w0) = (r.x + PAD, r.w - 2.0 * PAD);
        let mut y = r.y + PAD;

        // Layout: two equal tiles, icon over a small label; the current one in the accent tint
        let lay_on = self.qs_layout_on();
        self.qs_title("Layout", x0, y, w0, t);
        y += 18.0;
        let view = self.tab().view;
        let tw = (w0 - 8.0) / 2.0;
        for (i, (id, label, glyph, v)) in [(Q_DETAILS, "Details", 0xE8FDu32, View::Details), (Q_GRID, "Grid", 0xF0E2, View::Grid)].into_iter().enumerate() {
            let tr = Rect::new(x0 + i as f32 * (tw + 8.0), y, tw, 64.0);
            let cur = view == v;
            let h = Hit::Qs(id);
            let (hk, pk) = if lay_on { (self.hk(h), self.pk(h)) } else { (0.0, 0.0) };
            if cur && lay_on {
                self.gfx.fill_round(tr, R_CTRL, t.sel);
            } else {
                self.gfx.fill_round(tr, R_CTRL, neutral(t));
            }
            self.gfx.fill_round(tr, R_CTRL, t.hover.a(t.hover.3 * hk));
            self.gfx.fill_round(tr, R_CTRL, t.press.a(t.press.3 * pk * 0.6));
            let fg = if !lay_on { t.text_dis } else if cur { t.accent_text } else { t.text };
            self.gfx.glyph(glyph, Rect::new(tr.x, tr.y + 10.0, tr.w, 24.0), 20.0, fg);
            self.gfx.text(label, Rect::new(tr.x, tr.y + 38.0, tr.w, 18.0), Font::Ui, 12.0, if cur { 600 } else { 400 }, Align::Center, fg);
            self.qs_ring(id, tr, R_CTRL, t);
            if lay_on {
                self.hit(tr, h);
            }
        }
        y += 64.0 + 8.0;

        // Size of the layout shown (Grid and Details apart): a small and a large glyph at the ends of a continuous
        // slider, with a mark at the standard size (the knob snaps to it)
        self.paint_slider(Q_SIZE, Rect::new(x0, y, w0, SIZE_H), true, t);
        y += SIZE_H;
        self.paint_slider(Q_GAP, Rect::new(x0, y, w0, SIZE_H), true, t);
        y += SIZE_H + PAD;
        self.gfx.fill(Rect::new(r.x, y, r.w, 1.0), t.divider);
        y += 1.0 + PAD;

        // Sort by: one segmented row (the current field filled) and a square direction button at its end
        let sort_on = self.qs_sort_on();
        self.qs_title("Sort by", x0, y, w0, t);
        y += 18.0;
        let (col, desc) = self.pane().sort;
        let seg = Rect::new(x0, y, w0 - 32.0 - 6.0, 32.0);
        self.gfx.fill_round(seg, R_CTRL, neutral(t));
        let sw = seg.w / 4.0;
        for (i, (label, c)) in SORTS.iter().enumerate() {
            let id = Q_SORT + i as u8;
            let sr = Rect::new(seg.x + i as f32 * sw, y, sw, 32.0).inset(2.0, 2.0);
            let cur = col == *c;
            let h = Hit::Qs(id);
            let (hk, pk) = if sort_on { (self.hk(h), self.pk(h)) } else { (0.0, 0.0) };
            if cur && sort_on {
                self.gfx.fill_round(sr, R_CTRL - 2.0, t.sel);
            }
            self.gfx.fill_round(sr, R_CTRL - 2.0, t.hover.a(t.hover.3 * hk));
            self.gfx.fill_round(sr, R_CTRL - 2.0, t.press.a(t.press.3 * pk * 0.6));
            let fg = if !sort_on { t.text_dis } else if cur { t.accent_text } else { t.text };
            self.gfx.text(label, sr, Font::Ui, 12.5, if cur { 600 } else { 400 }, Align::Center, fg);
            self.qs_ring(id, sr, R_CTRL - 2.0, t);
            if sort_on {
                self.hit(sr, h);
            }
        }
        let dr = Rect::new(seg.r() + 6.0, y, 32.0, 32.0);
        let h = Hit::Qs(Q_DIR);
        let (hk, pk) = if sort_on { (self.hk(h), self.pk(h)) } else { (0.0, 0.0) };
        self.gfx.fill_round(dr, R_CTRL, neutral(t));
        self.gfx.fill_round(dr, R_CTRL, t.hover.a(t.hover.3 * hk));
        self.gfx.fill_round(dr, R_CTRL, t.press.a(t.press.3 * pk * 0.6));
        self.gfx.glyph(if desc { 0xE74B } else { 0xE74A }, dr, 14.0, if sort_on { t.text } else { t.text_dis });
        self.qs_ring(Q_DIR, dr, R_CTRL, t);
        if sort_on {
            self.hit(dr, h);
        }
        y += 32.0 + PAD;
        self.gfx.fill(Rect::new(r.x, y, r.w, 1.0), t.divider);
        y += 1.0 + 6.0;

        // switches: label at the left, a small toggle at the right; the whole row is the target
        for (id, label, on) in [(Q_PANE, "Details pane", self.look.details), (Q_HIDDEN, "Hidden items", self.ui.hidden), (Q_EXT, "File name extensions", self.ui.ext)] {
            let rr = Rect::new(r.x + 4.0, y, r.w - 8.0, 36.0);
            let h = Hit::Qs(id);
            let (hk, pk) = (self.hk(h), self.pk(h));
            self.gfx.fill_round(rr, R_CTRL, t.hover.a(t.hover.3 * hk));
            self.gfx.fill_round(rr, R_CTRL, t.press.a(t.press.3 * pk * 0.6));
            self.gfx.text(label, Rect::new(x0, y, w0 - 52.0, 36.0), Font::Ui, 13.0, 400, Align::Left, t.text);
            let sr = Rect::new(r.r() - PAD - 40.0, y, 40.0, 36.0);
            let tr = Rect::new(sr.x, sr.y + 8.0, 40.0, 20.0);
            if on {
                self.gfx.fill_round(tr, 10.0, if hk > 0.5 { t.accent2 } else { t.accent });
                self.gfx.ellipse(tr.x + 29.0, tr.cy(), 6.0, t.on_accent, true, 1.0);
            } else {
                self.gfx.fill_round(tr, 10.0, t.layer2);
                self.gfx.stroke_round(tr, 10.0, t.field_bottom, 1.0);
                self.gfx.ellipse(tr.x + 10.0, tr.cy(), 6.0, t.text2, true, 1.0);
            }
            self.qs_ring(id, rr, R_CTRL, t);
            self.hit(rr, h);
            y += 36.0;
        }
        y += 6.0;
        self.gfx.fill(Rect::new(r.x, y, r.w, 1.0), t.divider);
        y += 1.0 + 4.0;

        // footer: All settings
        let fr = Rect::new(r.x + 4.0, y, r.w - 8.0, 36.0);
        let h = Hit::Qs(Q_ALL);
        let (hk, pk) = (self.hk(h), self.pk(h));
        self.gfx.fill_round(fr, R_CTRL, t.hover.a(t.hover.3 * hk));
        self.gfx.fill_round(fr, R_CTRL, t.press.a(t.press.3 * pk * 0.6));
        self.gfx.glyph(0xE713, Rect::new(x0, y, 16.0, 36.0), 15.0, t.text);
        self.gfx.text("All settings", Rect::new(x0 + 28.0, y, w0 - 28.0, 36.0), Font::Ui, 13.0, 400, Align::Left, t.text);
        self.qs_ring(Q_ALL, fr, R_CTRL, t);
        self.hit(fr, h);

        self.gfx.pop_xf();
        self.gfx.pop_layer();
        if k < 1.0 {
            self.anim();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_order_covers_every_control_once() {
        let mut seen = std::collections::HashSet::new();
        for id in ORDER {
            assert!(seen.insert(id));
        }
        assert_eq!(ORDER.len(), 2 + 2 + 4 + 1 + 3 + 1);
        assert_eq!(SORTS.len(), 4);
    }
}

/// The resting fill of tiles, the sort row and the direction button: visible in both themes.
fn neutral(t: &Theme) -> crate::gfx::C {
    if t.dark { t.text.a(0.06) } else { t.layer2 }
}
