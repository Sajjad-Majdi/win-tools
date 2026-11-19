//! Painting: every part of the window, sized from design-fm/mockup.html (DIPs at 96 dpi).
use crate::app::*;
use crate::ficons::{self, Kind};
use crate::fmt;
use crate::gfx::{Align, C, Font, Rect, Span};
use crate::model::*;
use crate::motion::Spring;
use crate::preview::{PvKind, Tok};
use crate::theme::{R_CARD, R_CTRL, R_MENU, R_SMALL, Theme};
use std::time::Instant;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Std,
    Accent,
    Ghost,
    Disabled,
}

/// Geometry of one pane.
#[derive(Clone, Copy, Debug)]
pub struct PG {
    pub all: Rect,
    pub head: Option<Rect>,
    pub strip: Option<Rect>,
    pub cols: Rect,
    pub list: Rect,
}

/// Popup and view progress: a spring (a little bouncy for popups up to 200 ms, gentle for views); exactly 1.0 when
/// settled, so `k != 1.0` means "still moving".
fn prog(at: Instant, ms: f32, shooting: bool) -> f32 {
    crate::motion::since(at, if ms <= 200.0 { Spring::Bouncy } else { Spring::Gentle }, shooting)
}

/// Content area: right of the sidebar, below the bar, above the status bar, left of the "What is this file?" panel.
pub fn content_rect(a: &App) -> Rect {
    let m = a.main_rect();
    let wi = if a.whatis.is_some() && !a.tab().split() && m.w > WHATIS_W + 300.0 { WHATIS_W } else { 0.0 };
    Rect::new(m.x + 1.0, m.y, m.w - 1.0 - wi, (m.h - STATUS_H).max(0.0))
}

/// The pane shows the This PC page (drive tiles + pinned folders).
pub fn is_pc(a: &App, t: usize, p: usize) -> bool {
    let pn = &a.tabs[t].panes[p];
    pn.loc == Loc::ThisPc && !pn.searching()
}

fn tiled(a: &App, t: usize, p: usize) -> bool {
    (a.tabs[t].view == View::Grid && !a.tabs[t].panes[p].searching()) || is_pc(a, t, p)
}

pub fn pane_geoms(a: &App, t: usize) -> Vec<PG> {
    let c = content_rect(a);
    let tab = &a.tabs[t];
    let n = tab.panes.len();
    let mut out = vec![];
    for (i, p) in tab.panes.iter().enumerate() {
        let w = c.w / n as f32;
        let all = Rect::new(c.x + w * i as f32, c.y, w, c.h);
        let mut y = all.y;
        let head = if n > 1 {
            let r = Rect::new(all.x, y, all.w, PANE_HEAD_H);
            y += PANE_HEAD_H;
            Some(r)
        } else {
            None
        };
        let strip = if p.search.is_some() && n == 1 {
            let r = Rect::new(all.x, y, all.w, STRIP_H);
            y += STRIP_H;
            Some(r)
        } else {
            None
        };
        let cols = if tiled(a, t, i) { Rect::new(all.x, y, all.w, 0.0) } else { Rect::new(all.x, y, all.w, HEADER_H) };
        let list = Rect::new(all.x, cols.b(), all.w, (all.b() - cols.b()).max(0.0));
        out.push(PG { all, head, strip, cols, list });
    }
    out
}

pub fn row_h(a: &App, t: usize, p: usize) -> f32 {
    let pane = &a.tabs[t].panes[p];
    if pane.searching() {
        ROW2_H
    } else if a.tabs[t].view == View::List {
        a.look.list_row_h()
    } else {
        a.look.row_h()
    }
}

pub fn grid_cols(list: Rect) -> usize {
    (((list.w - 40.0 + 10.0) / 150.0).floor() as usize).max(1)
}

/// This PC page: drive tiles, then the pinned folders, both in a wrapping grid.
#[derive(Clone, Copy, Debug)]
pub struct PcLay {
    pub tw: f32,
    pub cols: usize,
    pub n_d: usize,
    pub head_d: f32,
    pub y_d: f32,
    pub head_p: f32,
    pub y_p: f32,
    pub h: f32,
}
pub const PC_TILE_D: f32 = 68.0;
pub const PC_TILE_P: f32 = 56.0;

pub fn pc_lay(a: &App, t: usize, p: usize, list: Rect) -> PcLay {
    let rows = a.tabs[t].panes[p].rows();
    let n_d = rows.iter().take_while(|i| i.kind == crate::ficons::Kind::Drive).count();
    let n_p = rows.len() - n_d;
    let avail = (list.w - 40.0).max(120.0);
    let cols = (((avail + 12.0) / (300.0 + 12.0)).floor() as usize).max(1);
    let tw = ((avail + 12.0) / cols as f32 - 12.0).min(360.0);
    let head_d = 10.0;
    let y_d = head_d + 34.0;
    let rows_d = n_d.div_ceil(cols);
    let head_p = y_d + rows_d as f32 * (PC_TILE_D + 8.0) + 14.0;
    let y_p = head_p + 34.0;
    let rows_p = n_p.div_ceil(cols);
    let h = if n_p == 0 { head_p } else { y_p + rows_p as f32 * (PC_TILE_P + 8.0) + 12.0 };
    PcLay { tw, cols, n_d, head_d, y_d, head_p, y_p, h }
}

/// Rect of item `i` in content coordinates: window x, y from the top of the list content (scroll not applied).
pub fn item_rect(a: &App, t: usize, p: usize, i: usize) -> Rect {
    let g = pane_geoms(a, t)[p.min(a.tabs[t].panes.len() - 1)];
    let list = g.list;
    if is_pc(a, t, p) {
        let l = pc_lay(a, t, p, list);
        let (k, y0, th) = if i < l.n_d { (i, l.y_d, PC_TILE_D) } else { (i - l.n_d, l.y_p, PC_TILE_P) };
        return Rect::new(list.x + 20.0 + (k % l.cols) as f32 * (l.tw + 12.0), y0 + (k / l.cols) as f32 * (th + 8.0), l.tw, th);
    }
    if tiled(a, t, p) {
        let cols = grid_cols(list);
        return Rect::new(list.x + 20.0 + (i % cols) as f32 * 150.0, 14.0 + (i / cols) as f32 * 156.0, 140.0, 150.0);
    }
    let h = row_h(a, t, p);
    Rect::new(list.x + 6.0, 4.0 + i as f32 * h, list.w - 12.0, h)
}

/// (y of row i inside the scrolled content, its height, visible height)
pub fn row_geom(a: &App, t: usize, p: usize, i: usize) -> (f32, f32, f32) {
    let g = pane_geoms(a, t)[p.min(a.tabs[t].panes.len() - 1)];
    let r = item_rect(a, t, p, i);
    (r.y, r.h, g.list.h.max(1.0))
}

pub fn content_h(a: &App, t: usize, p: usize) -> f32 {
    let n = a.tabs[t].panes[p].rows().len();
    let g = pane_geoms(a, t)[p];
    if is_pc(a, t, p) {
        return pc_lay(a, t, p, g.list).h;
    }
    if tiled(a, t, p) {
        let cols = grid_cols(g.list);
        return 14.0 + n.div_ceil(cols) as f32 * 156.0 + 14.0;
    }
    8.0 + n as f32 * row_h(a, t, p)
}

/// Columns of the details / list view: Name, then Date modified, Type, Size from the right. When the pane is too
/// narrow the extra columns drop out (Type first, then Date, then Size) so Name keeps NAME_MIN.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColPos {
    pub name: (f32, f32),
    pub date: Option<(f32, f32)>,
    pub ty: Option<(f32, f32)>,
    pub size: Option<(f32, f32)>,
}

pub fn col_pos(a: &App, r: Rect, name_x: f32) -> ColPos {
    let lk = &a.look;
    let right = r.r() - 10.0;
    let mut want = [(lk.col_date, lk.w_date), (lk.col_type, lk.w_type), (lk.col_size, lk.w_size)];
    let total = |w: &[(bool, f32)]| w.iter().filter(|c| c.0).map(|c| c.1 + 12.0).sum::<f32>();
    for drop in [1usize, 0, 2] {
        if right - name_x - total(&want) >= crate::look::NAME_MIN {
            break;
        }
        want[drop].0 = false;
    }
    let mut x = right;
    let mut out = ColPos::default();
    if want[2].0 {
        x -= want[2].1;
        out.size = Some((x, want[2].1));
        x -= 12.0;
    }
    if want[1].0 {
        x -= want[1].1;
        out.ty = Some((x, want[1].1));
        x -= 12.0;
    }
    if want[0].0 {
        x -= want[0].1;
        out.date = Some((x, want[0].1));
        x -= 12.0;
    }
    out.name = (name_x, (x - name_x).max(0.0));
    out
}

impl App {
    pub fn hit(&mut self, r: Rect, h: Hit) {
        if !self.no_hits {
            self.hits.push((r, h));
        }
    }

    pub fn hit_at(&self, x: f32, y: f32) -> Hit {
        for (r, h) in self.hits.iter().rev() {
            if r.has(x, y) {
                return *h;
            }
        }
        Hit::None
    }

    fn hov(&self, h: Hit) -> bool {
        self.hover == h && !self.shooting
    }

    pub fn paint(&mut self) {
        let mut rc = windows::Win32::Foundation::RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        let (pw, ph) = ((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32);
        self.gfx.dpi = unsafe { GetDpiForWindow(self.hwnd) } as f32;
        let first = self.first_paint_ms.is_none();
        if !self.gfx.ensure(pw, ph) {
            return;
        }
        if first {
            trace(&format!("{:.1} ms: render target", self.started.elapsed().as_secs_f64() * 1000.0));
        }
        self.w = pw as f32 / self.scale();
        self.h = ph as f32 / self.scale();
        self.hits.clear();
        let t = self.t.clone();
        // smooth scrolling: advance before the layout is painted
        let mut moving = false;
        for tab in self.tabs.iter_mut() {
            for pn in tab.panes.iter_mut() {
                if !pn.sf.settled() {
                    if (pn.scroll - pn.sf_written).abs() > 0.5 {
                        // something else moved the list (keyboard, scroll_to): stop following
                        pn.sf.set(pn.scroll);
                    } else {
                        moving |= pn.sf.step();
                        pn.scroll = pn.sf.value;
                        pn.sf_written = pn.scroll;
                    }
                }
            }
        }
        self.gfx.begin();
        if self.mica && !self.gfx.offscreen() && self.look.see_through() {
            self.gfx.clear(C(0.0, 0.0, 0.0, 0.0));
        } else if self.mica && !self.gfx.offscreen() {
            self.gfx.clear(C(0.0, 0.0, 0.0, 0.0));
            self.gfx.gradient(Rect::new(0.0, 0.0, self.w, self.h), t.mica_a, t.mica_b);
        } else {
            self.gfx.gradient(Rect::new(0.0, 0.0, self.w, self.h), t.mica_a, t.mica_b);
        }
        self.paint_title(&t);
        self.paint_bar(&t);
        if self.rail() {
            self.paint_rail(&t);
        } else {
            self.paint_side(&t);
        }
        self.paint_main(&t);
        self.paint_overlays(&t);
        self.gfx.end();
        // keep the frame timer only while something moves (no timer at all when idle)
        moving |= self.hov_f.busy();
        moving |= self.press_f.busy();
        moving |= self.menu_out.is_some() || self.drop_out.is_some();
        if moving && !self.shooting {
            self.anim();
        }
        // caret blink only while a text field has focus
        let field = self.path_edit.is_some() || self.search_focus || self.inline.is_some() || self.palette.is_some() || self.rename.is_some() || self.key_focus || self.accent_field.is_some();
        if field && !self.caret_timer {
            self.caret_timer = true;
            self.timer(T_CARET, 530);
        }
        if first {
            trace(&format!("{:.1} ms: frame drawn", self.started.elapsed().as_secs_f64() * 1000.0));
        }
        if self.first_paint_ms.is_none() {
            self.first_paint_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
        }
    }

    // ------------------------------------------------------------------ small parts
    /// Press-scale around the centre of `r` while `h` is pressed (springs back on release). Pair with `unpress`.
    fn press_in(&self, r: Rect, h: Hit) -> bool {
        let k = self.pk(h);
        if k > 0.001 {
            self.gfx.push_scale(r.cx(), r.cy(), 1.0 - (1.0 - crate::motion::PRESS_SCALE) * k, 0.0, 0.0);
            return true;
        }
        false
    }

    pub fn button(&mut self, r: Rect, label: &str, glyph: u32, style: Btn, h: Hit, t: &Theme) {
        let hk = if style == Btn::Disabled { 0.0 } else { self.hk(h) };
        let pk = if style == Btn::Disabled { 0.0 } else { self.pk(h) };
        let scaled = style != Btn::Disabled && self.press_in(r, h);
        match style {
            Btn::Std => {
                self.gfx.fill_round(Rect::new(r.x, r.y + 1.0, r.w, r.h), R_CTRL, t.field_border);
                self.gfx.fill_round(r, R_CTRL, t.card.mix(t.layer2, pk));
                self.gfx.stroke_round(r, R_CTRL, t.stroke, 1.0);
                self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * hk * (1.0 - pk)));
            }
            Btn::Accent => {
                self.gfx.fill_round(r, R_CTRL, t.accent.mix(t.accent2, hk).mix(t.accent, pk * 0.6).a(1.0 - 0.12 * pk));
                self.gfx.fill(Rect::new(r.x + 4.0, r.b() - 1.0, r.w - 8.0, 1.0), C(0.0, 0.0, 0.0, 0.18));
            }
            Btn::Ghost => {
                self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * hk));
                self.gfx.fill_round(r, R_CTRL, t.press.a(t.press.3 * pk * 0.6));
            }
            Btn::Disabled => {
                self.gfx.fill_round(r, R_CTRL, t.layer2);
                self.gfx.stroke_round(r, R_CTRL, t.stroke, 1.0);
            }
        }
        let fg = match style {
            Btn::Accent => t.on_accent,
            Btn::Disabled => t.text_dis,
            _ => t.text,
        };
        let tw = self.gfx.measure(label, Font::Ui, 13.0, 400);
        let gw = if glyph != 0 { 14.0 + if label.is_empty() { 0.0 } else { 8.0 } } else { 0.0 };
        let x = r.x + (r.w - tw - gw) / 2.0;
        if glyph != 0 {
            self.gfx.glyph(glyph, Rect::new(x, r.y, 14.0, r.h), 14.0, fg);
        }
        self.gfx.text(label, Rect::new(x + gw, r.y, tw + 2.0, r.h), Font::Ui, 13.0, 400, Align::Left, fg);
        if scaled {
            self.gfx.pop_xf();
        }
        self.hit(r, h);
    }

    fn icon_btn(&mut self, r: Rect, glyph: u32, size: f32, color: C, h: Hit, rad: f32, t: &Theme) {
        let (hk, pk) = (self.hk(h), self.pk(h));
        let scaled = self.press_in(r, h);
        self.gfx.fill_round(r, rad, t.hover.a(t.hover.3 * hk));
        self.gfx.fill_round(r, rad, t.press.a(t.press.3 * pk * 0.6));
        self.gfx.glyph(glyph, r, size, color.a(color.3 * (1.0 - 0.15 * pk)));
        if scaled {
            self.gfx.pop_xf();
        }
        self.hit(r, h);
    }

    pub fn card(&self, r: Rect, rad: f32, t: &Theme, big: bool) {
        let a = if t.dark { if big { 0.55 } else { 0.40 } } else if big { 0.24 } else { 0.14 };
        self.gfx.shadow(r, rad, if big { 24.0 } else { 8.0 }, if big { 40.0 } else { 18.0 }, a);
        self.gfx.fill_round(r, rad, t.card);
        self.gfx.stroke_round(r.inset(-1.0, -1.0), rad + 1.0, if t.dark { C(0.0, 0.0, 0.0, 0.35) } else { C(0.06, 0.09, 0.16, 0.07) }, 1.0);
    }

    fn toggle(&mut self, r: Rect, on: bool, h: Hit, t: &Theme) {
        let tr = Rect::new(r.x, r.y + (r.h - 20.0) / 2.0, 40.0, 20.0);
        if on {
            self.gfx.fill_round(tr, 10.0, if self.hov(h) { t.accent2 } else { t.accent });
            self.gfx.ellipse(tr.x + 29.0, tr.cy(), 6.0, t.on_accent, true, 1.0);
        } else {
            self.gfx.fill_round(tr, 10.0, if self.hov(h) { t.layer2.mix(t.text2, 0.08) } else { t.layer2 });
            self.gfx.stroke_round(tr, 10.0, t.field_bottom, 1.0);
            self.gfx.ellipse(tr.x + 10.0, tr.cy(), 6.0, t.text2, true, 1.0);
        }
        self.hit(tr, h);
    }

    fn dropdown(&mut self, r: Rect, label: &str, h: Hit, t: &Theme, font: Font) {
        self.gfx.fill_round(Rect::new(r.x, r.y + 1.0, r.w, r.h), R_CTRL, t.field_bottom);
        self.gfx.fill_round(r, R_CTRL, t.field);
        self.gfx.stroke_round(r, R_CTRL, t.field_border, 1.0);
        if self.hov(h) {
            self.gfx.fill_round(r, R_CTRL, t.hover);
        }
        if font == Font::Fa {
            self.gfx.para(label, Rect::new(r.x + 12.0, r.y + 2.0, r.w - 40.0, r.h), Font::Fa, 13.0, 400, Align::Right, t.text, 28.0, true, 1);
        } else {
            self.gfx.text(label, Rect::new(r.x + 12.0, r.y, r.w - 36.0, r.h), Font::Ui, 13.0, 400, Align::Left, t.text);
        }
        self.gfx.glyph(0xE70D, Rect::new(r.r() - 26.0, r.y, 16.0, r.h), 9.0, t.text2);
        self.hit(r, h);
    }

    fn kbd(&mut self, x: f32, y: f32, s: &str, t: &Theme) -> f32 {
        let w = self.gfx.measure(s, Font::Ui, 11.5, 400) + 12.0;
        let r = Rect::new(x, y, w.max(20.0), 20.0);
        self.gfx.fill_round(r, R_SMALL, t.kbd);
        self.gfx.stroke_round(r, R_SMALL, t.stroke, 1.0);
        self.gfx.text(s, r, Font::Ui, 11.5, 400, Align::Center, t.text2);
        r.w
    }

    fn field_box(&mut self, r: Rect, focus: bool, t: &Theme) {
        self.gfx.fill_round(r, R_CTRL, t.field);
        self.gfx.stroke_round(r, R_CTRL, t.field_border, 1.0);
        if focus {
            self.gfx.clip(Rect::new(r.x, r.b() - 2.0, r.w, 2.0));
            self.gfx.fill_round(r, R_CTRL, t.accent);
            self.gfx.unclip();
        } else {
            self.gfx.clip(Rect::new(r.x, r.b() - 1.0, r.w, 1.0));
            self.gfx.fill_round(r, R_CTRL, t.field_bottom);
            self.gfx.unclip();
        }
    }

    /// Text of a field with selection and caret; returns nothing. Single line, left aligned, scrolled to the caret.
    pub fn field_text(&mut self, f: &crate::fmt::Field, r: Rect, size: f32, focus: bool, t: &Theme, spans: &[Span]) {
        let caret_x = self.gfx.caret_x(&f.text, Font::Ui, size, 400, f.caret);
        let off = (caret_x - (r.w - 6.0)).max(0.0);
        self.gfx.clip(r);
        if focus && f.has_sel() {
            let (a, b) = f.sel_range();
            let xa = self.gfx.caret_x(&f.text, Font::Ui, size, 400, a);
            let xb = self.gfx.caret_x(&f.text, Font::Ui, size, 400, b);
            self.gfx.fill(Rect::new(r.x + xa.min(xb) - off, r.cy() - 9.0, (xb - xa).abs(), 18.0), t.accent.a(0.28));
        }
        let tr = Rect::new(r.x - off, r.y, r.w + off + 400.0, r.h);
        if spans.is_empty() {
            self.gfx.text(&f.text, tr, Font::Ui, size, 400, Align::Left, t.text);
        } else {
            self.gfx.text_spans(&f.text, tr, Font::Ui, size, 400, t.text, spans);
        }
        if focus && (self.caret_on || self.shooting) {
            self.gfx.fill(Rect::new(r.x + caret_x - off + 1.0, r.cy() - 8.5, 1.0, 17.0), t.text);
        }
        self.gfx.unclip();
    }

    // ------------------------------------------------------------------ title bar with tabs
    fn paint_title(&mut self, t: &Theme) {
        let n = self.tabs.len();
        let caps_x = self.w - CAP_W * 3.0;
        let avail = (caps_x - 8.0 - 40.0 - 36.0).max(40.0);
        let tw = (avail / n as f32).min(200.0).max(if avail / n as f32 >= 96.0 { 96.0 } else { 36.0 });
        let la = self.look.layer_alpha();
        let mut x = 8.0;
        let mut tab_hits = vec![];
        for i in 0..n {
            let r = Rect::new(x, 8.0, tw, 32.0);
            let active = i == self.cur;
            if active {
                let (l, rr, top, b) = (r.x, r.r(), r.y, r.b());
                use crate::gfx::P;
                self.gfx.fill_path(
                    &[P::M(l - 8.0, b), P::Q(l, b, l, b - 8.0), P::L(l, top + 8.0), P::Q(l, top, l + 8.0, top), P::L(rr - 8.0, top), P::Q(rr, top, rr, top + 8.0), P::L(rr, b - 8.0), P::Q(rr, b, rr + 8.0, b), P::Z],
                    0.0,
                    0.0,
                    1.0,
                    t.bar.a(la),
                );
            } else {
                let hk = self.hk(Hit::Tab(i)).max(self.hk(Hit::TabClose(i)));
                self.gfx.fill_round(Rect::new(r.x, r.y, r.w, r.h - 4.0), 8.0, t.hover.a(t.hover.3 * hk));
                if i > 0 && i - 1 != self.cur {
                    self.gfx.fill(Rect::new(r.x, 17.0, 1.0, 14.0), t.stroke);
                }
            }
            let tab = &self.tabs[i];
            let loc = tab.pane().loc.clone();
            let title = tab.title();
            let small = tw < 80.0;
            let ix = if small { r.x + (r.w - 16.0) / 2.0 } else { r.x + 12.0 };
            match loc {
                Loc::Settings => self.gfx.glyph(0xE713, Rect::new(ix, r.y, 16.0, r.h), 15.0, t.text2),
                Loc::RecycleBin => self.gfx.glyph(0xE74D, Rect::new(ix, r.y, 16.0, r.h), 15.0, t.text2),
                Loc::ThisPc => self.gfx.glyph(0xE7F8, Rect::new(ix, r.y, 16.0, r.h), 15.0, t.text2),
                Loc::Folder(ref p) if p.parent().is_none() => ficons::draw(&self.gfx, t, Kind::Drive, Rect::new(ix, r.cy() - 8.0, 16.0, 16.0)),
                _ => ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(ix, r.cy() - 8.0, 16.0, 16.0)),
            }
            if !small {
                self.gfx.text(&title, Rect::new(r.x + 38.0, r.y, r.w - 38.0 - 34.0, r.h), Font::Ui, 12.0, 400, Align::Left, if active { t.text } else { t.text2 });
                let cr = Rect::new(r.r() - 30.0, r.y + 4.0, 24.0, 24.0);
                let hk = self.hk(Hit::TabClose(i));
                self.gfx.fill_round(cr, R_SMALL, t.hover.a(t.hover.3 * hk));
                self.gfx.glyph(0xE711, cr, 10.0, t.text2);
                tab_hits.push((r, Hit::Tab(i)));
                tab_hits.push((cr, Hit::TabClose(i)));
            } else {
                tab_hits.push((r, Hit::Tab(i)));
            }
            x += tw;
        }
        for (r, h) in tab_hits {
            self.hit(r, h);
        }
        let ar = Rect::new(x + 4.0, 8.0, 32.0, 32.0);
        if ar.r() < caps_x - 4.0 {
            self.icon_btn(ar, 0xE710, 12.0, t.text2, Hit::TabAdd, R_CTRL, t);
        }
        // caption buttons exactly as Windows 11 draws them: Segoe Fluent Icons 10 px, 46 px wide, full title bar height,
        // hover fill, close hover #C42B1C with a white glyph, press fills; restore glyph only when maximised
        let maxed = unsafe { IsZoomed(self.hwnd).as_bool() };
        let active_win = unsafe { GetForegroundWindow() } == self.hwnd || self.shooting || self.offscreen;
        let caps = [(Hit::Min, 0xE921u32), (Hit::Max, if maxed { 0xE923 } else { 0xE922 }), (Hit::Close, 0xE8BB)];
        for (i, (h, gl)) in caps.iter().enumerate() {
            let r = Rect::new(caps_x + i as f32 * CAP_W, 0.0, CAP_W, TITLE_H);
            let hk = self.hk(*h);
            let down = self.nc_down == *h && self.nc_hover == *h;
            let mut fg = if active_win { t.text } else { t.text.a(0.45) };
            if *h == Hit::Close {
                let red = if down { C::hex(0xC42B1C).a(0.9) } else { C::hex(0xC42B1C) };
                self.gfx.fill(r, red.a(red.3 * hk));
                fg = fg.mix(C::hex(0xFFFFFF), hk);
            } else {
                let (hov, prs) = if t.dark { (C(1.0, 1.0, 1.0, 0.0605), C(1.0, 1.0, 1.0, 0.0419)) } else { (C(0.0, 0.0, 0.0, 0.0373), C(0.0, 0.0, 0.0, 0.0241)) };
                self.gfx.fill(r, if down { prs } else { hov.a(hov.3 * hk) });
                if down {
                    fg = fg.a(fg.3 * 0.78);
                }
            }
            self.gfx.text(&char::from_u32(*gl).unwrap().to_string(), r, Font::Icon, 10.0, 400, Align::Center, fg);
            self.hit(r, *h);
        }
    }

    // ------------------------------------------------------------------ top bar
    /// x where the back button starts (a sidebar toggle sits before it while the sidebar is hidden)
    pub fn nav_x(&self) -> f32 {
        if self.side_hidden() { 48.0 } else { 10.0 }
    }

    pub fn omni_rect(&mut self) -> Rect {
        let narrow = self.w < 1000.0;
        let tiny = self.w < 560.0;
        let vw = if narrow { 34.0 } else { 53.0 };
        let nw = if tiny { 0.0 } else if narrow { 34.0 } else { 12.0 + 14.0 + 8.0 + self.gfx.measure("New", Font::Ui, 13.0, 400) + 12.0 };
        let vx = self.w - 10.0 - vw;
        let nx = vx - 4.0 - nw;
        // very narrow: no forward button
        let x0 = self.nav_x() + if tiny { 82.0 } else { 120.0 };
        Rect::new(x0, TITLE_H + 7.0, (nx - 4.0 - 6.0 - x0).max(60.0), 34.0)
    }

    fn crumbs(&self) -> Vec<(String, Loc)> {
        let loc = &self.pane().loc;
        let mut v = vec![];
        match loc {
            Loc::Folder(p) => {
                let s = p.to_string_lossy().to_string();
                if s.starts_with(r"\\") {
                    let parts: Vec<&str> = s.trim_start_matches('\\').split('\\').filter(|x| !x.is_empty()).collect();
                    let mut cur = format!(r"\\{}", parts.first().unwrap_or(&""));
                    if parts.len() > 1 {
                        cur = format!(r"{cur}\{}", parts[1]);
                    }
                    v.push((cur.clone(), Loc::Folder(cur.clone().into())));
                    for part in parts.iter().skip(2) {
                        cur = format!(r"{cur}\{part}");
                        v.push((part.to_string(), Loc::Folder(cur.clone().into())));
                    }
                } else {
                    v.push(("This PC".into(), Loc::ThisPc));
                    let mut acc = std::path::PathBuf::new();
                    for (i, c) in p.components().enumerate() {
                        acc.push(c.as_os_str());
                        if i == 0 {
                            acc.push("\\");
                            v.push((drive_label(&c.as_os_str().to_string_lossy()), Loc::Folder(acc.clone())));
                        } else if let std::path::Component::Normal(n) = c {
                            v.push((n.to_string_lossy().into_owned(), Loc::Folder(acc.clone())));
                        }
                    }
                }
            }
            other => v.push((other.title(), other.clone())),
        }
        v
    }

    fn paint_bar(&mut self, t: &Theme) {
        let bar = Rect::new(0.0, TITLE_H, self.w, BAR_H);
        self.gfx.fill(bar, t.bar.a(self.look.layer_alpha()));
        self.gfx.fill(Rect::new(0.0, TOP - 1.0, self.w, 1.0), t.divider);
        let pane = self.pane();
        let (can_back, can_fwd, can_up) = (!pane.back.is_empty(), !pane.fwd.is_empty(), !matches!(pane.loc, Loc::ThisPc));
        let y = TITLE_H + 8.0;
        let tiny = self.w < 560.0;
        if self.side_hidden() {
            self.icon_btn(Rect::new(10.0, y, 34.0, 32.0), 0xE700, 16.0, t.text, Hit::Side(usize::MAX), R_CTRL, t);
        }
        let nx0 = self.nav_x();
        self.icon_btn(Rect::new(nx0, y, 34.0, 32.0), 0xE72B, 16.0, if can_back { t.text } else { t.text_dis }, Hit::Back, R_CTRL, t);
        if !tiny {
            self.icon_btn(Rect::new(nx0 + 38.0, y, 34.0, 32.0), 0xE72A, 16.0, if can_fwd { t.text } else { t.text_dis }, Hit::Fwd, R_CTRL, t);
        }
        self.icon_btn(Rect::new(nx0 + if tiny { 38.0 } else { 76.0 }, y, 34.0, 32.0), 0xE74A, 16.0, if can_up { t.text } else { t.text_dis }, Hit::Up, R_CTRL, t);
        let o = self.omni_rect();
        let narrow = self.w < 1000.0;
        // New + view buttons
        let nx = o.r() + 6.0 + 4.0;
        let vx;
        if tiny {
            vx = nx;
            let g = if self.tab().view == View::Grid { 0xF0E2 } else { 0xE8FD };
            self.icon_btn(Rect::new(vx, y, 34.0, 32.0), g, 14.0, t.text, Hit::ViewBtn, R_CTRL, t);
        } else if narrow {
            self.icon_btn(Rect::new(nx, y, 34.0, 32.0), 0xE710, 14.0, t.text, Hit::New, R_CTRL, t);
            vx = nx + 34.0 + 4.0;
            let g = if self.tab().view == View::Grid { 0xF0E2 } else { 0xE8FD };
            self.icon_btn(Rect::new(vx, y, 34.0, 32.0), g, 14.0, t.text, Hit::ViewBtn, R_CTRL, t);
        } else {
            let nw = 12.0 + 14.0 + 8.0 + self.gfx.measure("New", Font::Ui, 13.0, 400) + 12.0;
            let r = Rect::new(nx, y, nw, 32.0);
            self.button(r, "New", 0xE710, Btn::Ghost, Hit::New, t);
            vx = nx + nw + 4.0;
            let vr = Rect::new(vx, y, 53.0, 32.0);
            let open = self.drop.as_ref().map(|d| d.kind == DropKind::View).unwrap_or(false);
            self.gfx.fill_round(vr, R_CTRL, t.hover.a(t.hover.3 * if open { 1.0 } else { self.hk(Hit::ViewBtn) }));
            let g = if self.tab().view == View::Grid { 0xF0E2 } else { 0xE8FD };
            self.gfx.glyph(g, Rect::new(vr.x + 12.0, vr.y, 14.0, 32.0), 14.0, t.text);
            self.gfx.glyph(0xE70D, Rect::new(vr.x + 32.0, vr.y, 9.0, 32.0), 9.0, t.text2);
            self.hit(vr, Hit::ViewBtn);
        }
        // omnibox
        let searching = self.pane().search.is_some();
        let editing = self.path_edit.is_some();
        if searching || editing {
            self.gfx.fill_round(o, R_CTRL, t.card);
            self.gfx.stroke_round(o, R_CTRL, t.field_border, 1.0);
            self.gfx.clip(Rect::new(o.x, o.b() - 2.0, o.w, 2.0));
            self.gfx.fill_round(o, R_CTRL, t.accent);
            self.gfx.unclip();
        } else {
            self.gfx.fill_round(o, R_CTRL, t.field);
            self.gfx.stroke_round(o, R_CTRL, t.field_border, 1.0);
            if self.hov(Hit::Omni) {
                self.gfx.fill_round(o, R_CTRL, t.hover.a(t.hover.3 * 0.5));
            }
        }
        self.hit(o, Hit::Omni);
        if editing {
            let f = self.path_edit.clone().unwrap();
            self.field_text(&f, Rect::new(o.x + 12.0, o.y, o.w - 24.0, o.h), 13.0, true, t, &[]);
        } else if searching {
            self.gfx.glyph(0xE721, Rect::new(o.x + 10.0, o.y, 16.0, o.h), 14.0, t.text2);
            let f = self.pane().search.as_ref().unwrap().field.clone();
            let focus = self.search_focus;
            self.field_text(&f, Rect::new(o.x + 37.0, o.y, o.w - 37.0 - 36.0, o.h), 13.0, focus, t, &[]);
            if f.text.is_empty() {
                self.gfx.text("Search", Rect::new(o.x + 38.0, o.y, 200.0, o.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
            }
            let cr = Rect::new(o.r() - 32.0, o.y + 4.0, 26.0, 26.0);
            self.icon_btn(cr, 0xE711, 10.0, t.text2, Hit::OmniClear, R_SMALL, t);
        } else {
            let sw = if narrow || o.w < 300.0 { 34.0 } else { 86.0 };
            let sr = Rect::new(o.r() - sw, o.y + 4.0, sw - 4.0, 26.0);
            self.gfx.fill_round(sr, R_SMALL, t.hover.a(t.hover.3 * self.hk(Hit::OmniSearch)));
            self.gfx.fill(Rect::new(sr.x, sr.y, 1.0, sr.h), t.divider);
            if sw < 40.0 {
                self.gfx.glyph(0xE721, sr, 14.0, t.text2);
            } else {
                self.gfx.glyph(0xE721, Rect::new(sr.x + 10.0, sr.y, 14.0, sr.h), 14.0, t.text2);
                self.gfx.text("Search", Rect::new(sr.x + 32.0, sr.y, 60.0, sr.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
            }
            self.hit(sr, Hit::OmniSearch);
            // location icon + crumbs
            let loc = self.pane().loc.clone();
            let ir = Rect::new(o.x + 9.0, o.cy() - 8.0, 16.0, 16.0);
            match loc {
                Loc::Settings => self.gfx.glyph(0xE713, Rect::new(ir.x, o.y, 16.0, o.h), 15.0, t.text2),
                Loc::RecycleBin => self.gfx.glyph(0xE74D, Rect::new(ir.x, o.y, 16.0, o.h), 15.0, t.text2),
                Loc::ThisPc => self.gfx.glyph(0xE7F8, Rect::new(ir.x, o.y, 16.0, o.h), 15.0, t.text2),
                _ => ficons::draw(&self.gfx, t, Kind::Folder, ir),
            }
            let crumbs = self.crumbs();
            let x0 = o.x + 8.0 + 16.0 + 4.0;
            let maxw = sr.x - 8.0 - x0;
            let widths: Vec<f32> = crumbs.iter().enumerate().map(|(i, c)| self.gfx.measure(&c.0, Font::Ui, 13.0, if i + 1 == crumbs.len() { 600 } else { 400 }) + 12.0).collect();
            let mut first = 0;
            let total = |from: usize| -> f32 { widths[from..].iter().sum::<f32>() + 14.0 * (widths.len() - from) as f32 + if from > 0 { 30.0 } else { 0.0 } };
            while first + 1 < crumbs.len() && total(first) > maxw {
                first += 1;
            }
            let mut x = x0;
            let cy = o.y + 4.0;
            if first > 0 {
                let r = Rect::new(x, cy, 26.0, 26.0);
                if self.hov(Hit::Crumb(first - 1)) {
                    self.gfx.fill_round(r, R_SMALL, t.hover);
                }
                self.gfx.text("...", r, Font::Ui, 13.0, 400, Align::Center, t.text);
                self.hit(r, Hit::Crumb(first - 1));
                x += 26.0;
                self.gfx.glyph(0xE76C, Rect::new(x, cy, 14.0, 26.0), 8.0, t.text3);
                x += 14.0;
            }
            let last = crumbs.len() - 1;
            for (i, c) in crumbs.iter().enumerate().skip(first) {
                let w = widths[i].min((sr.x - 8.0 - x).max(30.0));
                let r = Rect::new(x, cy, w, 26.0);
                self.gfx.fill_round(r, R_SMALL, t.hover.a(t.hover.3 * self.hk(Hit::Crumb(i))));
                self.gfx.text(&c.0, Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h), Font::Ui, 13.0, if i == last { 600 } else { 400 }, Align::Left, t.text);
                self.hit(r, Hit::Crumb(i));
                x += w;
                if i < last {
                    self.gfx.glyph(0xE76C, Rect::new(x, cy, 14.0, 26.0), 8.0, t.text3);
                    x += 14.0;
                }
            }
        }
    }

    // ------------------------------------------------------------------ sidebar
    fn side_selected(&self, loc: &Loc) -> bool {
        &self.pane().loc == loc
    }

    fn paint_side(&mut self, t: &Theme) {
        let sw = self.side_w();
        let x = 6.0;
        let w = sw - 12.0;
        let mut y = TOP + 8.0;
        if !self.look.see_through() && self.mica && !self.gfx.offscreen() {
            // transparency off: an opaque sidebar
            self.gfx.gradient(Rect::new(0.0, TOP, sw, self.h - TOP), t.mica_a, t.mica_b);
        }
        let bottom_limit = self.h - 8.0 - 32.0 - 8.0;
        self.gfx.clip(Rect::new(0.0, TOP, sw, (bottom_limit - TOP).max(0.0)));
        let group = |a: &mut App, y: &mut f32, name: &str| {
            a.gfx.text(name, Rect::new(x + 12.0, *y + 10.0, w, 18.0), Font::Ui, 12.0, 600, Align::Left, t.text3);
            *y += 30.0;
        };
        let fs = self.look.text();
        // This PC: the start page with the big drive tiles
        let r = Rect::new(x, y, w, 32.0);
        let sel = self.pane().loc == Loc::ThisPc;
        self.nav_bg(r, sel, Hit::SideThisPc, t);
        self.gfx.glyph(0xE7F8, Rect::new(r.x + 10.0, r.y, 16.0, r.h), 16.0, t.text2);
        self.gfx.text("This PC", Rect::new(r.x + 38.0, r.y, r.w - 46.0, r.h), Font::Ui, fs, 400, Align::Left, t.text);
        self.hit(r, Hit::SideThisPc);
        y += 36.0;
        let side = self.side.clone();
        let base = side.len();
        if self.look.show_drives {
            group(self, &mut y, "Drives");
            let drives = self.drives.clone();
            for (i, d) in drives.iter().enumerate() {
                if y + 46.0 > bottom_limit {
                    break;
                }
                let r = Rect::new(x, y, w, 46.0);
                let loc = Loc::Folder(d.root.clone().into());
                let sel = self.side_selected(&loc);
                self.nav_bg(r, sel, Hit::Side(base + i), t);
                let letter = d.root.trim_end_matches('\\');
                let name = if d.label.is_empty() { drive_label(letter) } else { format!("{} ({letter})", d.label) };
                let l1 = Rect::new(r.x, r.y + 9.5, r.w, 18.0);
                self.gfx.glyph(if d.kind == "network" { 0xE8CE } else if d.kind == "removable" { 0xE88E } else { 0xEDA2 }, Rect::new(r.x + 10.0, l1.y, 16.0, l1.h), 16.0, t.text2);
                let free = if d.total > 0 { format!("{} free", fmt::size(d.free)) } else { String::new() };
                let fw = if w > 170.0 { self.gfx.measure(&free, Font::Ui, 12.0, 400) } else { 0.0 };
                self.gfx.text(&name, Rect::new(r.x + 38.0, l1.y, r.w - 38.0 - fw - 16.0, l1.h), Font::Ui, fs, 400, Align::Left, t.text);
                if fw > 0.0 {
                    self.gfx.text(&free, Rect::new(r.r() - 10.0 - fw - 2.0, l1.y, fw + 2.0, l1.h), Font::Ui, 12.0, 400, Align::Right, t.text3);
                }
                let mr = Rect::new(r.x + 38.0, l1.b() + 5.0, r.w - 48.0, 4.0);
                self.gfx.fill_round(mr, 2.0, t.track);
                if d.total > 0 {
                    let used = 1.0 - d.free as f32 / d.total as f32;
                    self.gfx.fill_round(Rect::new(mr.x, mr.y, (mr.w * used).max(4.0), 4.0), 2.0, if used > 0.9 { t.danger } else { t.accent });
                }
                self.hit(r, Hit::Side(base + i));
                y += 46.0;
            }
            y += 6.0;
        }
        if self.look.show_pinned {
            group(self, &mut y, "Pinned");
            for (i, (name, glyph, color, loc)) in side.iter().enumerate() {
                if y + 32.0 > bottom_limit {
                    break;
                }
                let r = Rect::new(x, y, w, 32.0);
                let sel = self.side_selected(loc);
                self.nav_bg(r, sel, Hit::Side(i), t);
                if *glyph == 0 {
                    ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(r.x + 10.0, r.cy() - 8.0, 16.0, 16.0));
                } else {
                    let c = if *color == 0 { t.text2 } else { C::hex(*color) };
                    let c = if t.dark && *color != 0 { c.mix(C::hex(0xFFFFFF), 0.25) } else { c };
                    self.gfx.glyph(*glyph, Rect::new(r.x + 10.0, r.y, 16.0, r.h), 16.0, c);
                }
                self.gfx.text(name, Rect::new(r.x + 38.0, r.y, r.w - 46.0, r.h), Font::Ui, fs, 400, Align::Left, t.text);
                self.hit(r, Hit::Side(i));
                y += 32.0;
            }
            y += 6.0;
        }
        let recent = self.recent.clone();
        if self.look.show_recent && !recent.is_empty() && y + 30.0 + 32.0 < bottom_limit {
            group(self, &mut y, "Recent");
            let base = base + self.drives.len();
            for (i, r0) in recent.iter().enumerate() {
                if y + 32.0 > bottom_limit {
                    break;
                }
                let r = Rect::new(x, y, w, 32.0);
                self.nav_bg(r, false, Hit::Side(base + i), t);
                ficons::draw(&self.gfx, t, ficons::kind_of(&r0.name, r0.is_dir), Rect::new(r.x + 10.0, r.cy() - 8.0, 16.0, 16.0));
                self.gfx.text(&r0.name, Rect::new(r.x + 38.0, r.y, r.w - 46.0, r.h), Font::Ui, fs, 400, Align::Left, t.text);
                self.hit(r, Hit::Side(base + i));
                y += 32.0;
            }
        }
        self.gfx.unclip();
        let r = Rect::new(x, self.h - 8.0 - 32.0, w, 32.0);
        let sel = self.pane().loc == Loc::Settings;
        self.nav_bg(r, sel, Hit::SideSettings, t);
        self.gfx.glyph(0xE713, Rect::new(r.x + 10.0, r.y, 16.0, r.h), 16.0, t.text2);
        self.gfx.text("Settings", Rect::new(r.x + 38.0, r.y, r.w - 46.0, r.h), Font::Ui, fs, 400, Align::Left, t.text);
        self.hit(r, Hit::SideSettings);
        // the edge: drag to resize (double-click: default width)
        let er = Rect::new(sw - 4.0, TOP, 8.0, self.h - TOP);
        let ek = if self.side_drag.is_some() { 1.0 } else { self.hk(Hit::SideEdge) };
        self.gfx.fill(Rect::new(sw - 1.0, TOP, 2.0, self.h - TOP), t.accent.a(0.6 * ek));
        self.hit(er, Hit::SideEdge);
    }

    fn nav_bg(&mut self, r: Rect, sel: bool, h: Hit, t: &Theme) {
        let (hk, pk) = (self.hk(h), self.pk(h));
        if sel {
            self.gfx.fill_round(r, R_CTRL, t.navsel);
            self.gfx.fill_round(Rect::new(r.x, r.cy() - 7.0, 3.0, 14.0), 1.5, t.accent);
        } else {
            self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * hk));
            self.gfx.fill_round(r, R_CTRL, t.press.a(t.press.3 * pk * 0.7));
        }
    }

    fn paint_rail(&mut self, t: &Theme) {
        if self.side_hidden() {
            return;
        }
        let x = 6.0;
        let mut y = TOP + 8.0;
        let r = Rect::new(x, y, 40.0, 36.0);
        // the menu button: full sidebar / rail
        self.icon_btn(r, 0xE700, 16.0, t.text2, Hit::Side(usize::MAX), R_CTRL, t);
        y += 38.0;
        let r = Rect::new(x, y, 40.0, 36.0);
        let sel = self.pane().loc == Loc::ThisPc;
        self.nav_bg(r, sel, Hit::SideThisPc, t);
        self.gfx.glyph(0xE7F8, r, 16.0, t.text2);
        self.hit(r, Hit::SideThisPc);
        y += 38.0;
        let side = self.side.clone();
        let drives = self.drives.clone();
        let limit = self.h - 8.0 - 40.0;
        if self.look.show_drives {
            for (i, d) in drives.iter().enumerate() {
                if y + 36.0 > limit {
                    break;
                }
                let r = Rect::new(x, y, 40.0, 36.0);
                let loc = Loc::Folder(d.root.clone().into());
                let sel = self.side_selected(&loc);
                self.nav_bg(r, sel, Hit::Side(side.len() + i), t);
                let chip = Rect::new(r.cx() - 13.0, r.cy() - 10.0, 26.0, 20.0);
                self.gfx.fill_round(chip, R_SMALL, t.layer2);
                self.gfx.stroke_round(chip, R_SMALL, t.stroke, 1.0);
                self.gfx.text(d.root.trim_end_matches('\\'), chip, Font::Ui, 12.0, 600, Align::Center, t.text2);
                self.hit(r, Hit::Side(side.len() + i));
                y += 38.0;
            }
            y += 6.0;
            self.gfx.fill(Rect::new(x + 8.0, y, 24.0, 1.0), t.divider);
            y += 10.0;
        }
        if self.look.show_pinned {
            for (i, (_, glyph, color, loc)) in side.iter().enumerate() {
                if y + 36.0 > limit {
                    break;
                }
                let r = Rect::new(x, y, 40.0, 36.0);
                let sel = self.side_selected(loc);
                self.nav_bg(r, sel, Hit::Side(i), t);
                if *glyph == 0 {
                    ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(r.cx() - 8.0, r.cy() - 8.0, 16.0, 16.0));
                } else {
                    let c = if *color == 0 { t.text2 } else { C::hex(*color) };
                    self.gfx.glyph(*glyph, r, 16.0, c);
                }
                self.hit(r, Hit::Side(i));
                y += 38.0;
            }
        }
        let r = Rect::new(x, self.h - 8.0 - 36.0, 40.0, 36.0);
        let sel = self.pane().loc == Loc::Settings;
        self.nav_bg(r, sel, Hit::SideSettings, t);
        self.gfx.glyph(0xE713, r, 16.0, t.text2);
        self.hit(r, Hit::SideSettings);
    }

    // ------------------------------------------------------------------ main area
    fn paint_main(&mut self, t: &Theme) {
        let m = self.main_rect();
        self.gfx.fill(m, t.layer.a(self.look.layer_alpha()));
        self.gfx.fill(Rect::new(m.x, m.y, 1.0, m.h), t.stroke);
        let cur = self.cur;
        if self.pane().loc == Loc::Settings {
            self.paint_settings(t);
        } else {
            let geoms = pane_geoms(self, cur);
            let split = self.tabs[cur].split();
            for (p, g) in geoms.iter().enumerate() {
                if p > 0 {
                    self.gfx.fill(Rect::new(g.all.x, g.all.y, 1.0, g.all.h), t.stroke);
                }
                let dim = split && p != self.tabs[cur].active;
                if let Some(h) = g.head {
                    self.paint_pane_head(h, p, t);
                }
                if let Some(s) = g.strip {
                    self.paint_strip(s, p, t);
                }
                if dim {
                    self.gfx.opacity_layer(g.all, 0.86);
                }
                self.paint_list(*g, p, t);
                if dim {
                    self.gfx.pop_layer();
                }
            }
        }
        self.paint_status(t);
        if self.whatis.is_some() && !self.tab().split() && content_rect(self).w < m.w - 2.0 {
            self.paint_whatis(t);
        }
    }

    fn paint_pane_head(&mut self, h: Rect, p: usize, t: &Theme) {
        let pane = &self.tabs[self.cur].panes[p];
        let name = pane.loc.title();
        let parent = pane.loc.path().and_then(|x| x.parent()).map(|x| x.display().to_string()).unwrap_or_default();
        let sel = pane.selected();
        let right = if sel.is_empty() {
            format!("{} items", pane.rows().len())
        } else {
            let bytes: u64 = sel.iter().filter_map(|i| i.size).sum();
            format!("{} selected · {}", sel.len(), fmt::size(bytes))
        };
        let active = self.tabs[self.cur].active == p;
        self.hit(h, Hit::PaneHead(p));
        ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(h.x + 16.0, h.cy() - 8.0, 16.0, 16.0));
        let nw = self.gfx.measure(&name, Font::Ui, 13.0, 600);
        let rw = self.gfx.measure(&right, Font::Ui, 12.0, 400);
        let nw = nw.min(h.w * 0.5);
        self.gfx.text(&name, Rect::new(h.x + 42.0, h.y, nw + 2.0, h.h), Font::Ui, 13.0, 600, Align::Left, t.text.a(if active { 1.0 } else { 0.86 }));
        self.gfx.text(&parent, Rect::new(h.x + 42.0 + nw + 10.0, h.y, (h.w - 42.0 - nw - 10.0 - rw - 30.0).max(0.0), h.h), Font::Ui, 12.0, 400, Align::Left, t.text3);
        self.gfx.text(&right, Rect::new(h.r() - 16.0 - rw - 2.0, h.y, rw + 2.0, h.h), Font::Ui, 12.0, 400, Align::Right, t.text2);
        self.gfx.fill(Rect::new(h.x, h.b() - 1.0, h.w, 1.0), t.divider);
        if active {
            self.gfx.fill_round(Rect::new(h.x + 12.0, h.b() - 2.0, h.w - 24.0, 2.0), 1.0, t.accent);
        }
    }

    fn paint_strip(&mut self, s: Rect, p: usize, t: &Theme) {
        let pane = &self.tabs[self.cur].panes[p];
        let Some(se) = pane.search.as_ref() else { return };
        let (pc, ft, fs, fd, total, busy, meaning) = (se.pc, se.ftype, se.fsize, se.fdate, se.total, se.busy, se.meaning);
        let note = se.note.clone();
        let here = pane.loc.title();
        let searching = pane.searching();
        self.gfx.fill(Rect::new(s.x, s.b() - 1.0, s.w, 1.0), t.divider);
        // scope segmented control
        let w1 = self.gfx.measure(&here, Font::Ui, 13.0, 400).min(160.0) + 28.0;
        let w2 = self.gfx.measure("This PC", Font::Ui, 13.0, 600) + 28.0;
        let seg = Rect::new(s.x + 16.0, s.y + 8.0, w1 + w2 + 6.0, 32.0);
        self.gfx.fill_round(seg, R_CTRL, t.layer2);
        self.gfx.stroke_round(seg, R_CTRL, t.stroke, 1.0);
        for (i, (label, w)) in [(here.as_str(), w1), ("This PC", w2)].iter().enumerate() {
            let x = seg.x + 2.0 + if i == 0 { 0.0 } else { w1 + 2.0 };
            let r = Rect::new(x, seg.y + 2.0, *w, 28.0);
            let on = (i == 1) == pc;
            if on {
                self.gfx.fill_round(r, R_SMALL, t.card);
                self.gfx.stroke_round(r, R_SMALL, t.stroke, 1.0);
            } else if self.hov(Hit::Scope(i as u8)) {
                self.gfx.fill_round(r, R_SMALL, t.hover);
            }
            self.gfx.text(label, r.inset(12.0, 0.0), Font::Ui, 13.0, if on { 600 } else { 400 }, Align::Center, if on { t.text } else { t.text2 });
            self.hit(r, Hit::Scope(i as u8));
        }
        let mut x = seg.r() + 12.0;
        self.gfx.fill(Rect::new(x - 6.0, s.y + 14.0, 1.0, 20.0), t.divider);
        for (i, (name, idx, list)) in [("Type", ft, &TYPES[..]), ("Size", fs, &SIZES[..]), ("Date", fd, &DATES[..])].iter().enumerate() {
            let val = list[*idx].0;
            let wn = self.gfx.measure(name, Font::Ui, 12.5, 400);
            let wv = self.gfx.measure(val, Font::Ui, 12.5, 400);
            let r = Rect::new(x, s.y + 9.0, 12.0 + wn + 6.0 + wv + 6.0 + 9.0 + 10.0, 30.0);
            let on = *idx != 0;
            self.gfx.fill_round(r, R_CTRL, if on { t.sel } else { t.card });
            self.gfx.stroke_round(r, R_CTRL, if on { t.sel_stroke } else { t.stroke }, 1.0);
            if self.hov(Hit::Filter(i as u8)) {
                self.gfx.fill_round(r, R_CTRL, t.hover);
            }
            self.gfx.text(name, Rect::new(r.x + 12.0, r.y, wn + 2.0, r.h), Font::Ui, 12.5, 400, Align::Left, t.text);
            self.gfx.text(val, Rect::new(r.x + 12.0 + wn + 6.0, r.y, wv + 2.0, r.h), Font::Ui, 12.5, 400, Align::Left, t.text2);
            self.gfx.glyph(0xE70D, Rect::new(r.r() - 19.0, r.y, 9.0, r.h), 9.0, t.text2);
            self.hit(r, Hit::Filter(i as u8));
            x = r.r() + 8.0;
        }
        let count = if !searching {
            String::new()
        } else if busy && total == 0 {
            "Searching...".into()
        } else if !note.is_empty() {
            note
        } else {
            format!("{} result{}", fmt::thousands(total as u64), if total == 1 { "" } else { "s" })
        };
        let mut right = s.r() - 16.0;
        if self.meaning_on {
            let mw = 14.0 + 13.0 + 8.0 + self.gfx.measure("By meaning", Font::Ui, 13.0, 400) + 12.0;
            let r = Rect::new(right - mw, s.y + 8.0, mw, 32.0);
            if meaning {
                self.gfx.fill_round(r, R_CTRL, t.sel);
                self.gfx.stroke_round(r, R_CTRL, t.sel_stroke, 1.0);
            } else {
                self.gfx.fill_round(r, R_CTRL, t.card);
                self.gfx.stroke_round(r, R_CTRL, t.stroke, 1.0);
            }
            if self.hov(Hit::Meaning) {
                self.gfx.fill_round(r, R_CTRL, t.hover);
            }
            self.gfx.glyph(0xF4A5, Rect::new(r.x + 13.0, r.y, 14.0, r.h), 13.0, t.accent_text);
            self.gfx.text("By meaning", Rect::new(r.x + 35.0, r.y, mw - 40.0, r.h), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.hit(r, Hit::Meaning);
            right = r.x - 8.0;
        }
        self.gfx.text(&count, Rect::new(x + 6.0, s.y, (right - x - 6.0).max(0.0), s.h), Font::Ui, 12.0, 400, Align::Left, t.text2);
    }

    fn paint_list(&mut self, g: PG, p: usize, t: &Theme) {
        let cur = self.cur;
        let view = self.tabs[cur].view;
        let pane_searching = self.tabs[cur].panes[p].searching();
        let pc = is_pc(self, cur, p);
        let grid = tiled(self, cur, p) && !pc;
        let (fs, fs2) = (self.look.text(), self.look.small_text());
        let small = view == View::List && !pane_searching;
        let isz = if small { self.look.icon().min(16.0) } else { self.look.icon() };
        // column header (dividers can be dragged; double-click fits)
        if !grid && !pc && g.cols.h > 0.0 {
            let c = g.cols;
            let (sort, desc) = self.tabs[cur].panes[p].sort;
            let rr = Rect::new(c.x + 6.0, c.y, c.w - 12.0, c.h);
            let cp = col_pos(self, rr, rr.x + 10.0 + isz + 10.0);
            let mut heads: Vec<(&str, f32, f32, SortCol, bool, u8)> = vec![("Name", rr.x + 10.0, cp.name.0 + cp.name.1 - rr.x - 10.0, SortCol::Name, false, 0)];
            if let Some((x, w)) = cp.date {
                heads.push(("Date modified", x, w, SortCol::Date, false, 1));
            }
            if let Some((x, w)) = cp.ty {
                heads.push(("Type", x, w, SortCol::Type, false, 3));
            }
            if let Some((x, w)) = cp.size {
                heads.push(("Size", x, w, SortCol::Size, true, 2));
            }
            for (label, x, w, col, right, idx) in heads.iter().copied() {
                let hr = Rect::new(x - 6.0, c.y + 4.0, w + 12.0, c.h - 8.0);
                let h = Hit::Header(p, idx);
                self.gfx.fill_round(hr, R_SMALL, t.hover.a(t.hover.3 * self.hk(h)));
                self.gfx.fill_round(hr, R_SMALL, t.press.a(t.press.3 * self.pk(h) * 0.7));
                let lw = self.gfx.measure(label, Font::Ui, 12.0, 400).min(w);
                let tx = if right { x + w - lw } else { x };
                let arrow_x = if right { tx - 14.0 } else { tx + lw + 6.0 };
                self.gfx.text(label, Rect::new(tx, c.y, lw + 2.0, c.h), Font::Ui, 12.0, 400, Align::Left, t.text2);
                if sort == col && !pane_searching && arrow_x + 8.0 < x + w + 6.0 {
                    self.gfx.glyph(if desc { 0xE70D } else { 0xE70E }, Rect::new(arrow_x, c.y, 8.0, c.h), 8.0, t.text2);
                }
                self.hit(hr, h);
            }
            for (k, col) in [cp.date, cp.ty, cp.size].iter().enumerate() {
                if let Some((x, _)) = col {
                    let h = Hit::ColEdge(p, k as u8);
                    let on = self.col_drag.map(|d| d.0 == k as u8).unwrap_or(false);
                    let hk = if on { 1.0 } else { self.hk(h) };
                    self.gfx.fill(Rect::new(x - 6.5, c.y + 9.0, 1.0, c.h - 18.0), t.divider.mix(t.accent, hk));
                    self.hit(Rect::new(x - 10.0, c.y, 8.0, c.h), h);
                }
            }
            self.gfx.fill(Rect::new(c.x + 6.0, c.b() - 1.0, c.w - 12.0, 1.0), t.divider);
        }
        let list = g.list;
        if list.h <= 0.0 {
            return;
        }
        self.hit(list, Hit::ListBg(p));
        // clamp scroll (and the smooth-scroll target)
        let ch = content_h(self, cur, p);
        let max = (ch - list.h).max(0.0);
        {
            let pane = &mut self.tabs[cur].panes[p];
            pane.scroll = pane.scroll.clamp(0.0, max);
            if !pane.sf.settled() {
                pane.sf.target = pane.sf.target.clamp(0.0, max);
            }
        }
        // selection changes: a single selection slides to its new row, newly selected rows fade in
        let (seen, sel_now) = {
            let pane = &self.tabs[cur].panes[p];
            (pane.sel_seen.clone(), pane.sel.clone())
        };
        if seen != sel_now {
            let mut slide = None;
            let mut fades = vec![];
            if seen.len() == 1 && sel_now.len() == 1 && !self.shooting {
                let old = seen.iter().next().unwrap().clone();
                if let Some(oi) = self.tabs[cur].panes[p].rows().iter().position(|r| r.path == old) {
                    slide = Some(item_rect(self, cur, p, oi));
                }
            }
            if slide.is_none() {
                fades = sel_now.difference(&seen).cloned().collect();
            }
            let now = Instant::now();
            let pane = &mut self.tabs[cur].panes[p];
            pane.slide = slide.map(|r| (r, now));
            for f in fades {
                pane.sel_fade.insert(f, now);
            }
            pane.sel_seen = sel_now;
        }
        let pane = &mut self.tabs[cur].panes[p];
        pane.sel_fade.retain(|_, at| crate::motion::moving(*at, Spring::Snappy));
        if pane.slide.map(|s| !crate::motion::moving(s.1, Spring::Snappy)).unwrap_or(false) {
            pane.slide = None;
        }
        let animating_sel = !pane.sel_fade.is_empty() || pane.slide.is_some();
        let scroll = pane.scroll;
        let loading = pane.loading;
        let error = pane.error.clone();
        let rows = pane.rows().clone();
        let sel = pane.sel.clone();
        let sel_fade = pane.sel_fade.clone();
        let slide = pane.slide;
        let focus = if pane.kb { pane.focus } else { None };
        let shown_at = pane.shown_at;
        let back_anim = pane.back_anim;
        if animating_sel {
            self.anim();
        }
        // folder change: the new view fades in and settles from 8 px (back: from above)
        let k = prog(shown_at, 300.0, self.shooting);
        let dy = (1.0 - k) * if back_anim { -8.0 } else { 8.0 };
        self.gfx.clip(list);
        if k != 1.0 {
            self.gfx.opacity_layer(list, k.clamp(0.05, 1.0));
            self.anim();
        }
        if rows.is_empty() {
            let msg = if let Some(e) = &error {
                e.clone()
            } else if loading {
                String::new()
            } else if pane_searching {
                if self.tabs[cur].panes[p].search.as_ref().map(|s| s.busy).unwrap_or(false) { String::new() } else { "No results".into() }
            } else {
                "This folder is empty".into()
            };
            self.gfx.text(&msg, Rect::new(list.x, list.y + 40.0, list.w, 24.0), Font::Ui, 13.0, 400, Align::Center, t.text3);
        }
        let menu_rows: Vec<std::path::PathBuf> = self.menu.as_ref().map(|m| m.paths.clone()).unwrap_or_default();
        let drop_target = self.drag_in.as_ref().and_then(|d| d.target.clone());
        let inline = self.inline.as_ref().filter(|i| i.pane == p).map(|i| (i.path.clone(), i.field.clone()));
        let to_win = |r: Rect| Rect::new(r.x, list.y + r.y - scroll + dy, r.w, r.h);
        let rad = if pc { R_CARD } else { R_CTRL };
        // selection fill of one item: fades in, or slides from the previous item
        let sel_fill = |a: &mut App, i: usize, r: Rect, path: &std::path::PathBuf| {
            let mut fr = r;
            let mut alpha = 1.0;
            if let Some((from, at)) = slide {
                let k = crate::motion::since(at, Spring::Snappy, a.shooting);
                let f = to_win(from);
                fr = Rect::new(f.x + (r.x - f.x) * k, f.y + (r.y - f.y) * k, f.w + (r.w - f.w) * k, f.h + (r.h - f.h) * k);
            } else if let Some(at) = sel_fade.get(path) {
                alpha = crate::motion::since(*at, Spring::Snappy, a.shooting).clamp(0.0, 1.0);
            }
            let _ = i;
            a.gfx.fill_round(fr, rad, t.sel.a(alpha));
            fr
        };
        let n = rows.len();
        // which items can be on screen
        let (first, last) = if grid || pc {
            (0, n)
        } else {
            let rh = row_h(self, cur, p);
            let f = ((scroll - 4.0 - dy.abs()) / rh).floor().max(0.0) as usize;
            (f.min(n), (((scroll + list.h + dy.abs()) / rh).ceil() as usize + 1).min(n))
        };
        let pl = if pc { Some(pc_lay(self, cur, p, list)) } else { None };
        if let Some(l) = pl {
            // section titles of the This PC page
            let hy = list.y + l.head_d - scroll + dy;
            self.gfx.text("Drives", Rect::new(list.x + 22.0, hy, 300.0, 30.0), Font::Ui, 14.0, 600, Align::Left, t.text);
            if l.n_d < n {
                let hy = list.y + l.head_p - scroll + dy;
                self.gfx.text("Pinned", Rect::new(list.x + 22.0, hy, 300.0, 30.0), Font::Ui, 14.0, 600, Align::Left, t.text);
            }
        }
        for i in first..last {
            let it = &rows[i];
            let r = to_win(item_rect(self, cur, p, i));
            if r.b() < list.y || r.y > list.b() {
                continue;
            }
            let h = Hit::Row(p, i);
            let is_sel = sel.contains(&it.path);
            let (hk, pk) = (self.hk(h), self.pk(h));
            let target = drop_target.as_ref() == Some(&it.path);
            if target {
                self.gfx.fill_round(r, rad, t.sel);
                self.gfx.stroke_round(r, rad, t.accent, 1.5);
            } else if is_sel {
                sel_fill(self, i, r, &it.path);
                if grid || pc {
                    self.gfx.stroke_round(r, rad, t.sel_stroke.a(if slide.is_some() { 0.0 } else { 1.0 }), 1.0);
                }
            } else if menu_rows.contains(&it.path) {
                self.gfx.fill_round(r, rad, t.hover);
                self.gfx.stroke_round(r, rad, t.stroke, 1.0);
            }
            if !target {
                self.gfx.fill_round(r, rad, t.hover.a(t.hover.3 * hk * if is_sel { 0.6 } else { 1.0 }));
                self.gfx.fill_round(r, rad, t.press.a(t.press.3 * pk * 0.8));
            }
            if focus == Some(i) {
                self.gfx.stroke_round(r, rad, t.accent, 1.0);
            }
            if let Some(l) = pl {
                if i < l.n_d {
                    self.paint_drive_tile(r, it, t);
                } else {
                    ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(r.x + 12.0, r.cy() - 16.0, 32.0, 32.0));
                    self.gfx.text(&it.name, Rect::new(r.x + 56.0, r.y + 9.0, r.w - 66.0, 20.0), Font::Ui, fs, 400, Align::Left, t.text);
                    self.gfx.text(&it.folder, Rect::new(r.x + 56.0, r.y + 29.0, r.w - 66.0, 18.0), Font::Ui, fs2, 400, Align::Left, t.text3);
                }
            } else if grid {
                let th = Rect::new(r.x + 6.0, r.y + 6.0, 128.0, 96.0);
                let has_thumb = matches!(it.kind, Kind::Img | Kind::Video | Kind::Pdf);
                let bmp = if has_thumb { self.thumb_bitmap(&it.path) } else { None };
                if let Some(b) = bmp {
                    let (bw, bh) = self.thumbs.get(&it.path).and_then(|x| x.as_ref()).map(|x| (x.0 as f32, x.1 as f32)).unwrap_or((128.0, 96.0));
                    let s = (th.w / bw).max(th.h / bh);
                    let (cw, chh) = (th.w / s, th.h / s);
                    let src = Rect::new((bw - cw) / 2.0, (bh - chh) / 2.0, cw, chh);
                    self.gfx.bitmap_src(&b, th, src, 1.0);
                    self.gfx.stroke_round(th, R_CTRL, t.stroke, 1.0);
                } else {
                    ficons::draw(&self.gfx, t, it.kind, Rect::new(th.cx() - 32.0, th.cy() - 32.0, 64.0, 64.0));
                }
                self.gfx.para(&self.display_name(it), Rect::new(r.x + 6.0, r.y + 108.0, 128.0, 34.0), Font::Ui, 12.0, 400, Align::Center, t.text, 16.0, false, 2);
            } else {
                let rh = r.h;
                ficons::draw(&self.gfx, t, it.kind, Rect::new(r.x + 10.0, r.cy() - isz / 2.0, isz, isz));
                let cp = col_pos(self, r, r.x + 10.0 + isz + 10.0);
                let (nx, nw) = cp.name;
                let name = self.display_name(it);
                if let Some((_, f)) = inline.as_ref().filter(|x| x.0 == it.path) {
                    let fr = Rect::new(nx - 6.0, r.y + (rh - 26.0) / 2.0, nw + 6.0, 26.0);
                    self.field_box(fr, true, t);
                    self.field_text(f, Rect::new(fr.x + 6.0, fr.y, fr.w - 12.0, fr.h), 13.0, true, t, &[]);
                } else if pane_searching {
                    let spans: Vec<Span> = it.hl.iter().map(|(s, l)| Span { start: *s, len: *l, color: t.accent_text, weight: 600, italic: false }).collect();
                    self.gfx.text_spans(&name, Rect::new(nx, r.y + 5.0, nw, 20.0), Font::Ui, fs, 400, t.text, &spans);
                    self.gfx.text(&it.folder, Rect::new(nx, r.y + 25.0, nw, 16.0), Font::Ui, fs2, 400, Align::Left, t.text3);
                } else {
                    self.gfx.text(&name, Rect::new(nx, r.y, nw, rh), Font::Ui, if small { fs - 0.5 } else { fs }, 400, Align::Left, t.text);
                }
                if let Some((x, w)) = cp.date {
                    let date = if it.kind == Kind::Drive { it.folder.clone() } else { fmt::date(it.modified, w < 130.0) };
                    self.gfx.text(&date, Rect::new(x, r.y, w, rh), Font::Ui, fs2, 400, Align::Left, t.text2);
                }
                if let Some((x, w)) = cp.ty {
                    let ty = if it.kind == Kind::Drive { "Drive".to_string() } else { ficons::type_label(&it.name, it.is_dir) };
                    self.gfx.text(&ty, Rect::new(x, r.y, w, rh), Font::Ui, fs2, 400, Align::Left, t.text2);
                }
                if let Some((x, w)) = cp.size {
                    let size = it.size.map(fmt::size).unwrap_or_default();
                    self.gfx.text(&size, Rect::new(x, r.y, w, rh), Font::Ui, fs2, 400, Align::Right, t.text2);
                }
            }
            self.hit(Rect::new(r.x, r.y.max(list.y), r.w, (r.b().min(list.b()) - r.y.max(list.y)).max(0.0)), h);
        }
        if k != 1.0 {
            self.gfx.pop_layer();
        }
        // rubber-band selection rectangle
        if let Some(m) = self.marquee.as_ref().filter(|m| m.pane == p) {
            let (ax, ay) = (m.x0, list.y + m.y0 - scroll);
            let mr = Rect::new(ax.min(m.x1), ay.min(m.y1), (ax - m.x1).abs(), (ay - m.y1).abs());
            self.gfx.fill_round(mr, 2.0, t.accent.a(0.16));
            self.gfx.stroke_round(mr, 2.0, t.accent.a(0.85), 1.0);
        }
        // scroll bar
        if max > 0.0 {
            let track = Rect::new(list.r() - 8.0, list.y + 4.0, 6.0, (list.h - 8.0).max(0.0));
            let th = (track.h * list.h / ch).max(30.0).min(track.h);
            let ty = track.y + (track.h - th) * (scroll / max);
            let wk = if self.drag_scroll.is_some() { 1.0 } else { self.hk(Hit::Scroll(p)) };
            let bw = 3.0 + 3.0 * wk;
            self.gfx.fill_round(Rect::new(track.r() - bw - 1.0, ty, bw, th), bw / 2.0, t.text3.a(0.45 + 0.25 * wk));
            self.hit(track, Hit::Scroll(p));
        }
        self.gfx.unclip();
    }

    /// One drive of the This PC page: drive image, name, usage bar (red above 90 % full), "X free of Y".
    fn paint_drive_tile(&mut self, r: Rect, it: &Item, t: &Theme) {
        let root = it.path.to_string_lossy().to_string();
        let d = self.drives.iter().find(|d| d.root.eq_ignore_ascii_case(&root)).cloned();
        // drive image: a slab with a lighter top face and a status light
        let ix = r.x + 12.0;
        let iy = r.cy() - 14.0;
        let body = Rect::new(ix, iy + 10.0, 40.0, 16.0);
        self.gfx.fill_round(Rect::new(ix + 2.0, iy + 4.0, 36.0, 12.0), 3.0, t.page_stroke.mix(t.page, 0.55));
        self.gfx.fill_round(body, 3.5, t.page_stroke.mix(t.text2, 0.25));
        self.gfx.fill_round(body.inset(1.0, 1.0), 3.0, t.page.mix(t.page_stroke, 0.35));
        self.gfx.ellipse(body.x + 7.0, body.cy(), 1.6, t.ok, true, 1.0);
        if d.as_ref().map(|d| d.kind == "network").unwrap_or(false) {
            self.gfx.glyph(0xE8CE, Rect::new(ix + 22.0, iy - 2.0, 16.0, 16.0), 12.0, t.text2);
        }
        let tx = r.x + 64.0;
        let tw = r.w - 64.0 - 14.0;
        self.gfx.text(&it.name, Rect::new(tx, r.y + 8.0, tw, 20.0), Font::Ui, self.look.text(), 400, Align::Left, t.text);
        if let Some(d) = d.filter(|d| d.total > 0) {
            let used = 1.0 - d.free as f32 / d.total as f32;
            let bar = Rect::new(tx, r.y + 31.0, tw, 6.0);
            self.gfx.fill_round(bar, 3.0, t.track);
            self.gfx.fill_round(Rect::new(bar.x, bar.y, (bar.w * used).max(6.0), 6.0), 3.0, if used > 0.9 { t.danger } else { t.accent });
            let s = format!("{} free of {}", fmt::size(d.free), fmt::size(d.total));
            self.gfx.text(&s, Rect::new(tx, r.y + 41.0, tw, 18.0), Font::Ui, self.look.small_text(), 400, Align::Left, t.text2);
        }
    }

    pub fn display_name(&self, it: &Item) -> String {
        if !self.ui.ext && !it.is_dir && it.kind != Kind::Drive {
            if let Some(i) = it.name.rfind('.')
                && i > 0
            {
                return it.name[..i].to_string();
            }
        }
        it.name.clone()
    }

    fn paint_status(&mut self, t: &Theme) {
        let m = self.main_rect();
        let c = content_rect(self);
        let y = m.b() - STATUS_H;
        let r = Rect::new(m.x + 1.0, y, c.w, STATUS_H);
        self.gfx.fill(Rect::new(r.x, y, r.w, 1.0), t.divider);
        let pane = self.pane();
        let mut x = r.x + 18.0;
        if pane.loc != Loc::Settings {
            let n = pane.rows().len();
            let items = if pane.searching() { String::new() } else { format!("{} item{}", fmt::thousands(n as u64), if n == 1 { "" } else { "s" }) };
            let sel = pane.selected();
            let selected = if sel.is_empty() {
                String::new()
            } else {
                let bytes: u64 = sel.iter().filter_map(|i| i.size).sum();
                format!("{} selected · {}", sel.len(), fmt::size(bytes))
            };
            for s in [items, selected] {
                if s.is_empty() {
                    continue;
                }
                let w = self.gfx.measure(&s, Font::Ui, 12.0, 400);
                self.gfx.text(&s, Rect::new(x, y, w + 2.0, STATUS_H), Font::Ui, 12.0, 400, Align::Left, t.text2);
                x += w + 14.0;
            }
        }
        // copy queue item
        let (active, title, frac, clashes, paused) = {
            let q = self.jobs.0.lock().unwrap();
            let act: Vec<&crate::jobs::Job> = q.jobs.iter().filter(|j| j.active()).collect();
            if act.is_empty() {
                (false, String::new(), 0.0, 0, false)
            } else {
                let (d, tt) = act.iter().fold((0u64, 0u64), |a, j| (a.0 + j.done, a.1 + j.total));
                let cl: usize = act.iter().map(|j| j.open_clashes()).sum();
                (true, act[0].title(), if tt > 0 { d as f32 / tt as f32 } else { 0.0 }, cl, act.iter().all(|j| j.paused()))
            }
        };
        if active || self.xfer_open {
            let label = if active { title } else { "Transfers".to_string() };
            let lw = self.gfx.measure(&label, Font::Ui, 12.0, 400);
            let pct = format!("{}%", (frac * 100.0).round() as u32);
            let pw = self.gfx.measure(&pct, Font::Ui, 12.0, 400);
            let cw = if clashes > 0 { 30.0 } else { 0.0 };
            let w = 10.0 + 14.0 + 8.0 + lw + 8.0 + if active { 56.0 + 8.0 + pw + 4.0 } else { 0.0 } + cw + 10.0;
            let xr = Rect::new(r.r() - 8.0 - w, y + 3.0, w, 24.0);
            if self.xfer_open {
                self.gfx.fill_round(xr, R_SMALL, t.press);
            } else if self.hov(Hit::Xfer) {
                self.gfx.fill_round(xr, R_SMALL, t.hover);
            }
            let mut xx = xr.x + 10.0;
            self.gfx.glyph(if paused { 0xE769 } else { 0xE8C8 }, Rect::new(xx, xr.y, 14.0, xr.h), 13.0, t.text);
            xx += 22.0;
            self.gfx.text(&label, Rect::new(xx, xr.y, lw + 2.0, xr.h), Font::Ui, 12.0, 400, Align::Left, t.text);
            xx += lw + 8.0;
            if active {
                let mr = Rect::new(xx, xr.cy() - 2.0, 56.0, 4.0);
                self.gfx.fill_round(mr, 2.0, t.track);
                self.gfx.fill_round(Rect::new(mr.x, mr.y, (56.0 * frac).max(3.0), 4.0), 2.0, t.accent);
                xx += 64.0;
                self.gfx.text(&pct, Rect::new(xx, xr.y, pw + 2.0, xr.h), Font::Ui, 12.0, 400, Align::Left, t.text);
                xx += pw + 6.0;
            }
            if clashes > 0 {
                self.gfx.glyph(0xE7BA, Rect::new(xx, xr.y, 14.0, xr.h), 12.0, t.warn);
                self.gfx.text(&clashes.to_string(), Rect::new(xx + 16.0, xr.y, 14.0, xr.h), Font::Ui, 12.0, 600, Align::Left, t.warn);
            }
            self.hit(xr, Hit::Xfer);
        }
    }

    // ------------------------------------------------------------------ what is this file?
    fn paint_whatis(&mut self, t: &Theme) {
        let m = self.main_rect();
        let r = Rect::new(m.r() - WHATIS_W, m.y, WHATIS_W, m.h);
        self.gfx.fill(r, t.layer.a(self.look.layer_alpha()));
        self.gfx.fill(Rect::new(r.x, r.y, 1.0, r.h), t.stroke);
        self.hit(r, Hit::Block);
        let Some(w) = self.whatis.as_ref() else { return };
        let item = w.item.clone();
        let answer = w.answer.clone();
        let facts = w.facts.clone();
        // header
        let hr = Rect::new(r.x + 1.0, r.y, r.w - 1.0, 44.0);
        self.gfx.glyph(0xF4A5, Rect::new(hr.x + 15.0, hr.y, 16.0, hr.h), 15.0, t.accent_text);
        self.gfx.text("What is this file?", Rect::new(hr.x + 41.0, hr.y, 200.0, hr.h), Font::Ui, 13.0, 600, Align::Left, t.text);
        self.icon_btn(Rect::new(hr.r() - 36.0, hr.y + 8.0, 28.0, 28.0), 0xE711, 10.0, t.text2, Hit::WhatIs(0), R_SMALL, t);
        self.gfx.fill(Rect::new(hr.x, hr.b() - 1.0, hr.w, 1.0), t.divider);
        // file
        let mut y = hr.b() + 16.0;
        ficons::draw(&self.gfx, t, item.kind, Rect::new(r.x + 16.0, y, 40.0, 40.0));
        let tx = r.x + 16.0 + 40.0 + 12.0;
        let tw = r.r() - 16.0 - tx;
        let nh = self.gfx.para(&item.name, Rect::new(tx, y, tw, 40.0), Font::Ui, 13.0, 600, Align::Left, t.text, 18.0, false, 3);
        let meta = format!("{}{}", ficons::type_label(&item.name, item.is_dir), item.size.map(|s| format!(" · {}", fmt::size(s))).unwrap_or_default());
        self.gfx.text(&meta, Rect::new(tx, y + nh + 4.0, tw, 16.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        y += (nh + 24.0).max(44.0) + 14.0;
        // answer card
        let cw = r.w - 32.0;
        let (text, rtl, color) = match &answer {
            None => ("Reading the file...".to_string(), false, t.text3),
            Some(Err(e)) => (e.clone(), false, t.text2),
            Some(Ok(a)) => (if a.lang == "fa" { fmt::fa_digits(&a.what) } else { a.what.clone() }, a.lang == "fa", t.text),
        };
        let (font, size, lh) = if rtl { (Font::Fa, 14.0, 28.0) } else { (Font::Ui, 13.0, 20.0) };
        let th = self.gfx.para_height(&text, cw - 32.0, font, size, lh, rtl);
        let card = Rect::new(r.x + 16.0, y, cw, th + 28.0);
        self.gfx.fill_round(card, R_CARD, t.card);
        self.gfx.stroke_round(card, R_CARD, t.stroke, 1.0);
        self.gfx.para(&text, Rect::new(card.x + 16.0, card.y + 14.0, cw - 32.0, th + 4.0), font, size, 400, if rtl { Align::Right } else { Align::Left }, color, lh, rtl, 0);
        y = card.b() + 16.0;
        for (k, v) in &facts {
            self.gfx.text(k, Rect::new(r.x + 16.0, y, 64.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text3);
            let vh = self.gfx.para(v, Rect::new(r.x + 90.0, y + 1.0, r.r() - 16.0 - r.x - 90.0, 40.0), Font::Ui, 12.0, 400, Align::Left, t.text, 16.0, false, 2);
            y += vh.max(16.0) + 8.0;
        }
        // actions that fit the answer
        let actions: Vec<String> = match &answer {
            Some(Ok(a)) if !a.actions.is_empty() => a.actions.iter().take(2).cloned().collect(),
            _ => {
                if item.is_archive() {
                    vec!["extract".into(), "delete".into()]
                } else {
                    vec!["open".into(), "delete".into()]
                }
            }
        };
        let ar = Rect::new(r.x + 1.0, r.b() - 64.0, r.w - 1.0, 64.0);
        self.gfx.fill(Rect::new(ar.x, ar.y, ar.w, 1.0), t.divider);
        let bw = (ar.w - 32.0 - 8.0) / 2.0;
        for (i, a) in actions.iter().enumerate() {
            let (label, glyph, code) = match a.as_str() {
                "extract" => ("Extract here", 0xE8B5, 1u8),
                "delete" => ("Delete", 0xE74D, 2),
                "run" => ("Run", 0xE768, 3),
                _ => ("Open", 0xE8E5, 3),
            };
            let br = Rect::new(ar.x + 15.0 + i as f32 * (bw + 8.0), ar.y + 13.0, bw, 32.0);
            self.button(br, label, glyph, Btn::Std, Hit::WhatIs(code), t);
        }
    }

    // ------------------------------------------------------------------ settings
    fn set_card(&mut self, r: Rect, t: &Theme) {
        self.gfx.fill_round(r, R_CARD, t.card);
        self.gfx.stroke_round(r, R_CARD, t.stroke, 1.0);
    }

    fn set_row(&mut self, x: f32, y: f32, w: f32, h: f32, glyph: u32, title: &str, sub: &str, first: bool, t: &Theme) {
        if !first {
            self.gfx.fill(Rect::new(x, y, w, 1.0), t.divider);
        }
        self.gfx.glyph(glyph, Rect::new(x + 16.0, y, 18.0, h), 16.0, t.text2);
        if sub.is_empty() {
            self.gfx.text(title, Rect::new(x + 46.0, y, w - 200.0, h), Font::Ui, 13.0, 400, Align::Left, t.text);
        } else {
            self.gfx.text(title, Rect::new(x + 46.0, y + h / 2.0 - 18.0, w - 46.0 - 72.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.gfx.text(sub, Rect::new(x + 46.0, y + h / 2.0 + 1.0, w - 46.0 - 72.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        }
    }

    fn paint_settings(&mut self, t: &Theme) {
        let c = content_rect(self);
        self.hit(c, Hit::ListBg(0));
        let max = (self.settings_h - c.h).max(0.0);
        if self.pane().scroll > max && self.settings_h > 0.0 {
            self.pane_mut().scroll = max;
        }
        let scroll = self.pane().scroll;
        self.gfx.clip(c);
        // one column on a narrow window
        let one = c.w < 760.0;
        let colw = if one { c.w - 40.0 } else { (c.w - 40.0 - 12.0) / 2.0 };
        let lx = c.x + 20.0;
        let rx = if one { lx } else { lx + colw + 12.0 };
        let mut y = c.y + 16.0 - scroll;
        // Default file manager
        let is_def = fmcore::defaultfm::is_default();
        let r = Rect::new(lx, y, colw, 120.0);
        self.set_card(r, t);
        let big = Rect::new(r.x + 16.0, r.y + 16.0, 40.0, 40.0);
        self.gfx.fill_round(big, R_CTRL, t.sel);
        self.gfx.glyph(0xEC50, big, 20.0, t.accent_text);
        self.gfx.text("Default file manager", Rect::new(r.x + 70.0, r.y + 16.0, colw - 90.0, 20.0), Font::Ui, 14.0, 600, Align::Left, t.text);
        let sub = if is_def { "Rust File Explorer opens folders and Win+E now" } else { "Windows Explorer opens folders and Win+E now" };
        self.gfx.text(sub, Rect::new(r.x + 70.0, r.y + 38.0, colw - 90.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        let b1w = self.gfx.measure("Set as default file manager", Font::Ui, 13.0, 400) + 26.0;
        self.button(Rect::new(r.x + 70.0, r.y + 70.0, b1w, 32.0), "Set as default file manager", 0, if is_def { Btn::Disabled } else { Btn::Accent }, Hit::Set(1), t);
        let b2w = self.gfx.measure("Restore Explorer", Font::Ui, 13.0, 400) + 26.0;
        self.button(Rect::new(r.x + 70.0 + b1w + 8.0, r.y + 70.0, b2w, 32.0), "Restore Explorer", 0, if is_def { Btn::Std } else { Btn::Disabled }, Hit::Set(2), t);
        y = r.b() + 12.0;
        // Personalize: theme, background, transparency, accent, corners
        let lk = self.look.clone();
        let rows_h = 38.0 + 48.0 * 3.0 + 48.0 + 44.0 + if self.accent_field.is_some() { 44.0 } else { 0.0 } + 48.0 + 4.0;
        let r = Rect::new(lx, y, colw, rows_h);
        self.set_card(r, t);
        self.gfx.text("Personalize", Rect::new(r.x + 16.0, r.y + 12.0, 200.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        let mut ry = r.y + 38.0;
        let dw = 168.0f32.min(r.w * 0.45);
        let dd = |a: &mut App, ry: f32, glyph: u32, title: &str, val: &str, id: u8, first: bool| {
            a.set_row(r.x, ry, r.w, 48.0, glyph, title, "", first, t);
            a.dropdown(Rect::new(r.r() - 16.0 - dw, ry + 8.0, dw, 32.0), val, Hit::Set(id), t, Font::Ui);
        };
        dd(self, ry, 0xE790, "Theme", crate::look::THEMES[lk.theme as usize % 3], 3, true);
        ry += 48.0;
        let bd = if self.can_mica { crate::look::BACKDROPS[lk.backdrop as usize % 4] } else { "Solid (needs Windows 11)" };
        dd(self, ry, 0xE771, "Background", bd, 20, false);
        ry += 48.0;
        dd(self, ry, 0xE7B3, "Transparency", crate::look::TRANSPARENCY[lk.transparency as usize % 4], 21, false);
        ry += 48.0;
        // accent: Windows, presets, custom
        self.set_row(r.x, ry, r.w, 48.0, 0xE790, "Accent color", match lk.accent {
            None => "From Windows",
            Some(c) => crate::look::ACCENTS.iter().find(|a| a.1 == c).map(|a| a.0).unwrap_or("Custom"),
        }, false, t);
        ry += 48.0;
        let mut sx = r.x + 46.0;
        let sel_i: u8 = match lk.accent {
            None => 40,
            Some(c) => crate::look::ACCENTS.iter().position(|a| a.1 == c).map(|i| 41 + i as u8).unwrap_or(50),
        };
        let wa = self.windows_accent();
        for id in 40u8..=50 {
            if (42..50).contains(&id) && sx + 30.0 * 2.0 > r.r() - 12.0 && id != 50 {
                continue;
            }
            let col = match id {
                40 => wa,
                50 => lk.accent.map(C::hex).unwrap_or(t.layer2),
                k => C::hex(crate::look::ACCENTS[(k - 41) as usize].1),
            };
            let sr = Rect::new(sx, ry + 4.0, 28.0, 28.0);
            let h = Hit::Set(id);
            let hk = self.hk(h);
            if sel_i == id {
                self.gfx.ellipse(sr.cx(), sr.cy(), 13.5, t.text, false, 2.0);
            } else if hk > 0.0 {
                self.gfx.ellipse(sr.cx(), sr.cy(), 13.5, t.text3.a(hk), false, 1.0);
            }
            self.gfx.ellipse(sr.cx(), sr.cy(), 10.0, col, true, 1.0);
            self.gfx.ellipse(sr.cx(), sr.cy(), 10.0, t.text.a(0.14), false, 1.0);
            if id == 50 && sel_i != 50 {
                self.gfx.glyph(0xE710, sr, 10.0, t.text2);
            }
            self.hit(sr, h);
            sx += 32.0;
        }
        ry += 44.0;
        if let Some(f) = self.accent_field.clone() {
            let fr = Rect::new(r.x + 46.0, ry, 130.0, 32.0);
            self.field_box(fr, true, t);
            self.field_text(&f, Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), 13.0, true, t, &[]);
            self.hit(fr, Hit::Set(50));
            let aw = self.gfx.measure("Apply", Font::Ui, 13.0, 400) + 26.0;
            self.button(Rect::new(fr.r() + 8.0, ry, aw, 32.0), "Apply", 0, Btn::Accent, Hit::Set(51), t);
            ry += 44.0;
        }
        dd(self, ry, 0xE8B3, "Corners", crate::look::CORNERS[lk.square as usize], 25, false);
        y = r.b() + 12.0;
        // Layout: density, text and icon size, sidebar, columns, start page
        let rows_h = 38.0 + 48.0 * 5.0 + 3.0 * 44.0 + 30.0 + 3.0 * 44.0 + 30.0 + 48.0 + 4.0;
        let r = Rect::new(lx, y, colw, rows_h);
        self.set_card(r, t);
        self.gfx.text("Layout", Rect::new(r.x + 16.0, r.y + 12.0, 200.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        let mut ry = r.y + 38.0;
        let dd = |a: &mut App, ry: f32, glyph: u32, title: &str, val: &str, id: u8, first: bool| {
            a.set_row(r.x, ry, r.w, 48.0, glyph, title, "", first, t);
            a.dropdown(Rect::new(r.r() - 16.0 - dw, ry + 8.0, dw, 32.0), val, Hit::Set(id), t, Font::Ui);
        };
        dd(self, ry, 0xE8FD, "Density", crate::look::DENSITY[lk.density as usize % 2], 22, true);
        ry += 48.0;
        dd(self, ry, 0xE8D2, "Text size", crate::look::SIZES3[lk.font as usize % 3], 23, false);
        ry += 48.0;
        dd(self, ry, 0xE8B9, "Icon size", crate::look::SIZES3[lk.icons as usize % 3], 24, false);
        ry += 48.0;
        let sw_label = crate::look::SIDEBAR_WIDTHS.iter().find(|w| (w.1 - lk.side_w).abs() < 1.0).map(|w| w.0.to_string()).unwrap_or_else(|| if lk.side_w < 1.0 { "Icons only".into() } else { format!("{} px", lk.side_w.round()) });
        dd(self, ry, 0xE700, "Sidebar width", &sw_label, 26, false);
        ry += 48.0;
        let sub = |a: &mut App, ry: &mut f32, title: &str| {
            a.gfx.fill(Rect::new(r.x, *ry, r.w, 1.0), t.divider);
            a.gfx.text(title, Rect::new(r.x + 46.0, *ry + 8.0, 300.0, 20.0), Font::Ui, 12.0, 600, Align::Left, t.text3);
            *ry += 30.0;
        };
        sub(self, &mut ry, "Sidebar sections");
        for (title, on, id) in [("Drives", lk.show_drives, 28u8), ("Pinned", lk.show_pinned, 27), ("Recent", lk.show_recent, 29)] {
            self.gfx.text(title, Rect::new(r.x + 46.0, ry, r.w - 120.0, 44.0), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry, 40.0, 44.0), on, Hit::Set(id), t);
            ry += 44.0;
        }
        sub(self, &mut ry, "Columns");
        for (title, on, id) in [("Date modified", lk.col_date, 30u8), ("Type", lk.col_type, 31), ("Size", lk.col_size, 32)] {
            self.gfx.text(title, Rect::new(r.x + 46.0, ry, r.w - 120.0, 44.0), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry, 40.0, 44.0), on, Hit::Set(id), t);
            ry += 44.0;
        }
        let start_label = self.start_options().into_iter().find(|o| o.1 == lk.start).map(|o| o.0).unwrap_or_else(|| "This PC".into());
        dd(self, ry, 0xE80F, "Start page", &start_label, 33, false);
        y = r.b() + 12.0;
        // Files and folders
        let rows: [(u32, &str, &str, bool, u8); 4] = [
            (0xE8B7, "Show folder sizes", "", self.ui.sizes, 4),
            (0xE8AC, "Show file extensions", "", self.ui.ext, 5),
            (0xE7B3, "Show hidden files", "", self.ui.hidden, 6),
            (0xE74D, "Ask before delete", "Delete goes to the Recycle Bin and can be undone", self.ui.ask_delete, 7),
        ];
        let r = Rect::new(lx, y, colw, 38.0 + 4.0 * 48.0 + 4.0);
        self.set_card(r, t);
        self.gfx.text("Files and folders", Rect::new(r.x + 16.0, r.y + 12.0, 200.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        let mut ry = r.y + 38.0;
        for (i, (g, title, sub, on, id)) in rows.iter().enumerate() {
            let h = if sub.is_empty() { 48.0 } else { 52.0 };
            self.set_row(r.x, ry, r.w, h, *g, title, sub, i == 0, t);
            self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry, 40.0, h), *on, Hit::Set(*id), t);
            ry += h;
        }
        // right column: search and AI (below the left one on a narrow window)
        let left_end = y + 4.0 * 48.0 + 38.0 + 4.0 + 12.0 + 4.0;
        let mut y = if one { left_end } else { c.y + 16.0 - scroll };
        let ev = match self.set.ev_running {
            None => "Checking...".to_string(),
            Some(true) => format!("Running · {} items", fmt::thousands(self.set.ev_total as u64)),
            Some(false) => "Not running (search walks the folder instead)".into(),
        };
        let sem = crate::models::semsearch_detect();
        let sem_on = crate::models::switch_on(crate::models::Section::Semsearch) && sem.is_some();
        let ai_switch = crate::ai::switch_on() && crate::ai::has_key();
        let has_key = crate::ai::has_key();
        let lang = crate::ai::lang();
        let note_h = self.gfx.para_height(crate::ai::SENT_NOTE, colw - 46.0 - 72.0, Font::Ui, 12.0, 16.0, false);
        let ai_h = 30.0 + note_h + 12.0;
        let rows_h = 38.0 + 52.0 * 2.0 + ai_h + 96.0 + 52.0 + 4.0;
        let r = Rect::new(rx, y, colw, rows_h);
        self.set_card(r, t);
        self.gfx.text("Search and AI", Rect::new(r.x + 16.0, r.y + 12.0, 200.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        let mut ry = r.y + 38.0;
        self.set_row(r.x, ry, r.w, 52.0, 0xE721, "Everything index", "", true, t);
        self.gfx.fill(Rect::new(r.x + 46.0, ry + 8.0, r.w - 80.0, 36.0), t.card);
        self.gfx.text("Everything index", Rect::new(r.x + 46.0, ry + 8.0, r.w - 80.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
        let dot_c = if self.set.ev_running == Some(true) { t.ok } else { t.text3 };
        self.gfx.ellipse(r.x + 50.0, ry + 35.0, 4.0, dot_c, true, 1.0);
        self.gfx.text(&ev, Rect::new(r.x + 60.0, ry + 27.0, r.w - 80.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        ry += 52.0;
        // meaning search: its own switch; uses the semsearch already installed on this PC
        let sem_sub = match &sem {
            Some(p) => format!("Already installed · {}", p.parent().map(|d| d.display().to_string()).unwrap_or_default()),
            None => "Not installed on this PC".to_string(),
        };
        self.set_row(r.x, ry, r.w, 52.0, 0xF4A5, "Search by meaning", &sem_sub, false, t);
        if sem.is_some() {
            self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry, 40.0, 52.0), sem_on, Hit::Set(8), t);
        }
        ry += 52.0;
        // AI features: switch + exactly what is sent
        self.gfx.fill(Rect::new(r.x, ry, r.w, 1.0), t.divider);
        self.gfx.glyph(0xF4A5, Rect::new(r.x + 16.0, ry + 8.0, 18.0, 20.0), 16.0, t.text2);
        self.gfx.text("AI features", Rect::new(r.x + 46.0, ry + 8.0, r.w - 120.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
        self.gfx.para(crate::ai::SENT_NOTE, Rect::new(r.x + 46.0, ry + 30.0, r.w - 46.0 - 72.0, note_h + 2.0), Font::Ui, 12.0, 400, Align::Left, t.text2, 16.0, false, 0);
        self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry + 4.0, 40.0, 28.0), ai_switch, Hit::Set(9), t);
        ry += ai_h;
        // API key (masked) + Save + Test key + link
        self.gfx.fill(Rect::new(r.x, ry, r.w, 1.0), t.divider);
        self.gfx.glyph(0xE8D7, Rect::new(r.x + 16.0, ry + 8.0, 18.0, 20.0), 16.0, t.text2);
        self.gfx.text("API key", Rect::new(r.x + 46.0, ry + 8.0, 120.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
        let lw = self.gfx.measure("Get a free key", Font::Ui, 12.0, 400);
        let lr = Rect::new(r.r() - 16.0 - lw - 18.0, ry + 8.0, lw + 18.0, 20.0);
        self.gfx.text("Get a free key", Rect::new(lr.x, lr.y, lw + 2.0, lr.h), Font::Ui, 12.0, 400, Align::Left, if self.hov(Hit::Set(14)) { t.accent2 } else { t.accent_text });
        self.gfx.glyph(0xE8A7, Rect::new(lr.x + lw + 4.0, lr.y, 12.0, lr.h), 10.0, t.accent_text);
        self.hit(lr, Hit::Set(14));
        let fy = ry + 34.0;
        let sw_ = self.gfx.measure("Save", Font::Ui, 13.0, 400) + 26.0;
        let tw_ = self.gfx.measure("Test key", Font::Ui, 13.0, 400) + 26.0;
        let fr = Rect::new(r.x + 46.0, fy, r.w - 46.0 - 16.0 - sw_ - 8.0 - tw_ - 8.0, 32.0);
        let focus = self.key_focus;
        self.field_box(fr, focus, t);
        let masked: String = if self.key_field.text.is_empty() && !focus && has_key { "\u{2022}".repeat(24) } else { "\u{2022}".repeat(self.key_field.text.chars().count()) };
        let mut mf = crate::fmt::Field::new(&masked);
        mf.caret = masked.chars().count();
        mf.anchor = mf.caret;
        self.field_text(&mf, Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), 13.0, focus, t, &[]);
        if masked.is_empty() && !focus {
            self.gfx.text("Paste your key", Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
        }
        self.hit(fr, Hit::Set(15));
        let can_save = !self.key_field.text.trim().is_empty();
        self.button(Rect::new(fr.r() + 8.0, fy, sw_, 32.0), "Save", 0, if can_save { Btn::Accent } else { Btn::Disabled }, Hit::Set(16), t);
        self.button(Rect::new(fr.r() + 8.0 + sw_ + 8.0, fy, tw_, 32.0), "Test key", 0, if has_key || can_save { Btn::Std } else { Btn::Disabled }, Hit::Set(17), t);
        let status = self.set.msg.clone().unwrap_or_else(|| if has_key { "Key saved (encrypted for this Windows user)".into() } else { "No key saved".into() });
        let sc = if self.set.msg_ok { t.ok } else if self.set.msg.is_some() { t.warn } else { t.text3 };
        self.gfx.text(&status, Rect::new(r.x + 46.0, fy + 36.0, r.w - 62.0, 18.0), Font::Ui, 12.0, 400, Align::Left, sc);
        ry += 96.0;
        self.set_row(r.x, ry, r.w, 52.0, 0xF2B7, "AI answer language", "", false, t);
        let (lab, font) = if lang == "fa" { ("فارسی", Font::Fa) } else { ("English", Font::Ui) };
        self.dropdown(Rect::new(r.r() - 16.0 - 120.0, ry + 10.0, 120.0, 32.0), lab, Hit::Set(11), t, font);
        y = r.b() + 12.0;
        // command line for agents
        let cli: [(&str, &str, &str); 10] = [
            ("wfm open", " <path>", "Open a folder in a new tab"),
            ("wfm ls", " <path> --json", "List a folder"),
            ("wfm search", " <query>", "Search with Everything"),
            ("wfm copy", " <src…> <dst>", "Copy through the queue"),
            ("wfm move", " <src…> <dst>", "Move through the queue"),
            ("wfm rename", " <path> <name>", "Rename one item"),
            ("wfm trash", " <path…>", "Move to the Recycle Bin"),
            ("wfm undo", "", "Undo the last action"),
            ("wfm queue", " --json", "Show the copy queue"),
            ("wfm save-as", " <path>", "Fill and confirm an open Save As"),
        ];
        let r = Rect::new(rx, y, colw, 44.0 + 12.0 + 25.0 * cli.len() as f32 + 12.0);
        self.set_card(r, t);
        self.gfx.text("Command line for agents", Rect::new(r.x + 16.0, r.y + 12.0, 240.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        let on_path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join("wfm.exe").is_file())).unwrap_or(false);
        let st = if on_path { "wfm is on PATH" } else { "wfm.exe is next to RustFileExplorer.exe" };
        let sw = self.gfx.measure(st, Font::Ui, 12.0, 400);
        self.gfx.ellipse(r.r() - 16.0 - sw - 10.0, r.y + 22.0, 4.0, if on_path { t.ok } else { t.text3 }, true, 1.0);
        self.gfx.text(st, Rect::new(r.r() - 16.0 - sw - 1.0, r.y + 12.0, sw + 2.0, 20.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        let box_ = Rect::new(r.x + 12.0, r.y + 44.0, r.w - 24.0, 12.0 + 25.0 * cli.len() as f32);
        self.gfx.fill_round(box_, R_CTRL, t.code_bg);
        self.gfx.stroke_round(box_, R_CTRL, t.stroke, 1.0);
        let mut cy = box_.y + 6.0;
        for (cmd, args, what) in cli {
            let cwid = self.gfx.measure(cmd, Font::Mono, 12.0, 600);
            self.gfx.text(cmd, Rect::new(box_.x + 12.0, cy, cwid + 2.0, 25.0), Font::Mono, 12.0, 600, Align::Left, t.text);
            self.gfx.text(args, Rect::new(box_.x + 12.0 + cwid, cy, 186.0 - cwid, 25.0), Font::Mono, 12.0, 400, Align::Left, t.text3);
            self.gfx.text(what, Rect::new(box_.x + 12.0 + 196.0, cy, box_.w - 220.0, 25.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
            cy += 25.0;
        }
        self.settings_h = r.b() + 16.0 + scroll - c.y;
        self.gfx.unclip();
    }

    // ------------------------------------------------------------------ overlays
    fn paint_overlays(&mut self, t: &Theme) {
        if self.toast.is_some() {
            self.paint_toast(t);
        }
        if self.xfer_open {
            self.paint_transfers(t);
        }
        self.paint_closing(t);
        if self.menu.is_some() {
            self.paint_menu(t, None);
        }
        if self.drop.is_some() {
            self.paint_drop(false, t, None);
        }
        if self.sub.is_some() {
            self.paint_drop(true, t, None);
        }
        if self.palette.is_some() {
            self.paint_palette(t);
        }
        if self.rename.is_some() {
            self.paint_rename(t);
            if self.drop.is_some() {
                self.paint_drop(false, t, None);
            }
        }
        if self.pv.is_some() {
            self.paint_preview(t);
            if self.drop.is_some() {
                self.paint_drop(false, t, None);
            }
        }
        if self.confirm.is_some() {
            self.paint_confirm(t);
        }
        if self.drag_in.is_some() {
            self.paint_ghost(t);
        }
    }

    /// Menus that were just closed fade out quickly (accelerating), without hits.
    fn paint_closing(&mut self, t: &Theme) {
        if let Some((m, at)) = self.menu_out.take() {
            let o = crate::motion::lin(at, 130.0);
            if o < 1.0 && !self.shooting {
                let keep = self.menu.replace(m);
                self.no_hits = true;
                self.paint_menu(t, Some(o));
                self.no_hits = false;
                if let Some(m) = std::mem::replace(&mut self.menu, keep) {
                    self.menu_out = Some((m, at));
                }
            }
        }
        if let Some((d, sub, at)) = self.drop_out.take() {
            let o = crate::motion::lin(at, 130.0);
            if o < 1.0 && !self.shooting {
                let keep = if sub { self.sub.replace(d) } else { self.drop.replace(d) };
                self.no_hits = true;
                self.paint_drop(sub, t, Some(o));
                self.no_hits = false;
                let back = if sub { std::mem::replace(&mut self.sub, keep) } else { std::mem::replace(&mut self.drop, keep) };
                if let Some(d) = back {
                    self.drop_out = Some((d, sub, at));
                }
            }
        }
    }

    fn paint_toast(&mut self, t: &Theme) {
        let Some(to) = self.toast.as_ref() else { return };
        let age = to.at.elapsed().as_secs_f32();
        if !self.shooting && age > 6.0 && !to.hover {
            self.toast = None;
            return;
        }
        let (text, bold, undo) = (to.text.clone(), to.bold.clone(), to.undo);
        let k = prog(to.at, 200.0, self.shooting);
        let m = self.main_rect();
        let tw = self.gfx.measure(&text, Font::Ui, 13.0, 400);
        let bw = self.gfx.measure(&bold, Font::Ui, 13.0, 600);
        let uw = if undo { self.gfx.measure("Undo", Font::Ui, 13.0, 400) + 12.0 + 14.0 + 8.0 + 12.0 } else { 0.0 };
        let w = 16.0 + 16.0 + 12.0 + tw + bw + 12.0 + uw + 8.0 + 28.0 + 6.0;
        let r = Rect::new(m.x + 24.0, self.h - 42.0 - 48.0 + (1.0 - k) * 12.0, w, 48.0);
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), k.clamp(0.05, 1.0));
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        self.gfx.glyph(0xE73E, Rect::new(r.x + 16.0, r.y, 16.0, r.h), 14.0, t.ok);
        self.gfx.text(&text, Rect::new(r.x + 44.0, r.y, tw + 2.0, r.h), Font::Ui, 13.0, 400, Align::Left, t.text);
        self.gfx.text(&bold, Rect::new(r.x + 44.0 + tw, r.y, bw + 2.0, r.h), Font::Ui, 13.0, 600, Align::Left, t.text);
        let mut x = r.x + 44.0 + tw + bw + 12.0;
        if undo {
            self.button(Rect::new(x, r.y + 8.0, uw, 32.0), "Undo", 0xE7A7, Btn::Std, Hit::Toast(1), t);
            x += uw + 8.0;
        }
        self.icon_btn(Rect::new(x, r.y + 10.0, 28.0, 28.0), 0xE711, 10.0, t.text2, Hit::Toast(0), R_SMALL, t);
        self.gfx.pop_layer();
        if k != 1.0 {
            self.anim();
        }
    }

    pub fn menu_rect(&self) -> Option<Rect> {
        let m = self.menu.as_ref()?;
        let mut h = 8.0 + if m.bg { 0.0 } else { 47.0 };
        for it in &m.items {
            h += 32.0 + if it.sep { 9.0 } else { 0.0 };
        }
        if m.loading {
            h += 32.0;
        }
        let w = 292.0;
        let x = m.x.min(self.w - w - 8.0).max(4.0);
        let y = if m.y + h > self.h - 8.0 { (m.y - h).max(TITLE_H) } else { m.y };
        Some(Rect::new(x, y, w, h))
    }

    fn menu_rows(&mut self, r: Rect, items: &[MItem], y0: f32, hit_of: impl Fn(usize) -> Hit, t: &Theme) {
        let mut y = y0;
        let any_icon = items.iter().any(|x| x.glyph != 0 || x.icon.is_some() || x.swatch.is_some());
        for (i, it) in items.iter().enumerate() {
            if it.sep {
                self.gfx.fill(Rect::new(r.x, y + 4.0, r.w, 1.0), t.divider);
                y += 9.0;
            }
            let ir = Rect::new(r.x + 4.0, y, r.w - 8.0, 32.0);
            let h = hit_of(i);
            let open_sub = it.sub && self.sub.as_ref().map(|s| s.anchor.y == ir.y && s.child).unwrap_or(false);
            if it.enabled {
                let hk = if open_sub { 1.0 } else { self.hk(h) };
                self.gfx.fill_round(ir, R_CTRL, t.hover.a(t.hover.3 * hk));
                self.gfx.fill_round(ir, R_CTRL, t.press.a(t.press.3 * self.pk(h) * 0.8));
            }
            let fg = if it.enabled { t.text } else { t.text_dis };
            let ib = Rect::new(ir.x + 10.0, ir.cy() - 8.0, 16.0, 16.0);
            if let Some(c) = it.swatch {
                let col = if c == u32::MAX { self.windows_accent() } else { C::hex(c) };
                self.gfx.ellipse(ib.cx(), ib.cy(), 7.5, col, true, 1.0);
                self.gfx.ellipse(ib.cx(), ib.cy(), 7.5, t.text.a(0.18), false, 1.0);
            } else if let Some(icon) = it.icon.clone() {
                // the shell entry's own icon, drawn at 16 DIP like ours
                let key = std::path::PathBuf::from(format!("menuicon:{}:{}", it.label, icon.width));
                if self.bmp_target != self.gfx.target_id() {
                    self.bitmaps.clear();
                    self.bmp_target = self.gfx.target_id();
                }
                let bmp = match self.bitmaps.get(&key) {
                    Some(b) => b.clone(),
                    None => {
                        let b = self.gfx.make_bitmap(icon.width, icon.height, &icon.bgra);
                        self.bitmaps.insert(key, b.clone());
                        b
                    }
                };
                if let Some(b) = bmp {
                    self.gfx.bitmap(&b, ib, if it.enabled { 1.0 } else { 0.4 });
                }
            } else if it.glyph != 0 {
                self.gfx.glyph(it.glyph, Rect::new(ir.x + 10.0, ir.y, 16.0, ir.h), 16.0, fg);
            }
            let tx = if any_icon { ir.x + 38.0 } else { ir.x + 12.0 };
            let kw = if it.keys.is_empty() { 0.0 } else { self.gfx.measure(&it.keys, Font::Ui, 12.0, 400) + 4.0 };
            self.gfx.text(&it.label, Rect::new(tx, ir.y, ir.r() - tx - kw - 30.0, ir.h), Font::Ui, 13.0, if it.bold { 600 } else { 400 }, Align::Left, fg);
            if !it.keys.is_empty() {
                self.gfx.text(&it.keys, Rect::new(ir.r() - 10.0 - kw, ir.y, kw, ir.h), Font::Ui, 12.0, 400, Align::Right, t.text3);
            }
            if it.sub {
                self.gfx.glyph(0xE76C, Rect::new(ir.r() - 24.0, ir.y, 14.0, ir.h), 10.0, t.text2);
            }
            if it.checked {
                self.gfx.glyph(0xE73E, Rect::new(ir.r() - 26.0, ir.y, 16.0, ir.h), 12.0, t.accent_text);
            }
            self.hit(ir, h);
            y += 32.0;
        }
    }

    /// The Windows accent (for the "Windows accent" swatch).
    pub fn windows_accent(&self) -> C {
        crate::theme::make(self.t.dark, None).accent
    }

    /// Open / close motion of a popup: (opacity, scale, y offset). `out` = closing progress 0..1.
    fn pop_motion(&self, at: Instant, out: Option<f32>) -> (f32, f32, f32) {
        if let Some(o) = out {
            let k = crate::motion::fade_out(o);
            return (1.0 - k, 1.0 - 0.02 * k, -3.0 * k);
        }
        let k = prog(at, 150.0, self.shooting);
        ((k * 1.8).clamp(0.0, 1.0), 0.96 + 0.04 * k, -(1.0 - k) * crate::motion::POP_SLIDE)
    }

    fn paint_menu(&mut self, t: &Theme, out: Option<f32>) {
        let Some(r) = self.menu_rect() else { return };
        let m = self.menu.as_ref().unwrap();
        let (op, sc, dy) = self.pop_motion(m.at, out);
        let moving = out.is_none() && prog(m.at, 150.0, self.shooting) != 1.0;
        let (ax, ay) = (m.x.clamp(r.x, r.r()), if m.y > r.y { r.b() } else { r.y });
        let items = m.items.clone();
        let bg = m.bg;
        let loading = m.loading;
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), op.max(0.02));
        self.gfx.push_scale(ax, ay, sc, 0.0, dy);
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        let mut y = r.y + 4.0;
        if !bg {
            let qs = [(0u8, 0xE8C6u32), (1, 0xE8C8), (2, 0xE8AC), (3, 0xE74D)];
            let slot = (r.w - 20.0) / 4.0;
            for (i, (id, g)) in qs.iter().enumerate() {
                let br = Rect::new(r.x + 10.0 + slot * i as f32 + (slot - 40.0) / 2.0, y + 4.0, 40.0, 36.0);
                self.icon_btn(br, *g, 16.0, t.text, Hit::MenuQuick(*id), R_CTRL, t);
            }
            y += 46.0;
            self.gfx.fill(Rect::new(r.x, y - 1.0, r.w, 1.0), t.divider);
            y += 4.0;
        }
        self.menu_rows(r, &items, y, Hit::MenuItem, t);
        if loading {
            let ly = r.b() - 36.0;
            self.gfx.text("More from apps...", Rect::new(r.x + 42.0, ly, 200.0, 32.0), Font::Ui, 12.0, 400, Align::Left, t.text3);
        }
        self.gfx.pop_xf();
        self.gfx.pop_layer();
        if moving {
            self.anim();
        }
    }

    pub fn drop_rect(&self, sub: bool) -> Option<Rect> {
        let d = if sub { self.sub.as_ref()? } else { self.drop.as_ref()? };
        let mut h = 8.0;
        for it in &d.items {
            h += 32.0 + if it.sep { 9.0 } else { 0.0 };
        }
        let (mut x, mut y) = if d.child { (d.anchor.r() + 2.0, d.anchor.y - 4.0) } else { (d.anchor.x, d.anchor.b() + 4.0) };
        if d.child && x + d.width > self.w - 4.0 {
            x = d.anchor.x - d.width - 2.0;
        }
        x = x.min(self.w - d.width - 4.0).max(4.0);
        if y + h > self.h - 4.0 {
            y = if d.child { (self.h - 4.0 - h).max(4.0) } else { (d.anchor.y - 4.0 - h).max(4.0) };
        }
        Some(Rect::new(x, y, d.width, h))
    }

    fn paint_drop(&mut self, sub: bool, t: &Theme, out: Option<f32>) {
        let Some(r) = self.drop_rect(sub) else { return };
        let d = if sub { self.sub.as_ref().unwrap() } else { self.drop.as_ref().unwrap() };
        let (op, sc, dy) = self.pop_motion(d.at, out);
        let moving = out.is_none() && prog(d.at, 150.0, self.shooting) != 1.0;
        let (ax, ay) = if d.child { (r.x, r.y) } else { (d.anchor.cx().clamp(r.x, r.r()), if d.anchor.y < r.y { r.y } else { r.b() }) };
        let dy = if d.child { 0.0 } else { dy };
        let items = d.items.clone();
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), op.max(0.02));
        self.gfx.push_scale(ax, ay, sc, if d.child { -(1.0 - sc) * 60.0 } else { 0.0 }, dy);
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        if sub {
            self.menu_rows(r, &items, r.y + 4.0, Hit::SubItem, t);
        } else {
            self.menu_rows(r, &items, r.y + 4.0, Hit::Drop, t);
        }
        self.gfx.pop_xf();
        self.gfx.pop_layer();
        if moving {
            self.anim();
        }
    }

    fn paint_transfers(&mut self, t: &Theme) {
        // running and waiting jobs first (in queue order), then the finished ones, newest first
        let jobs: Vec<crate::jobs::Job> = {
            let q = self.jobs.0.lock().unwrap();
            let mut v: Vec<crate::jobs::Job> = q.jobs.iter().filter(|j| j.active()).cloned().collect();
            v.extend(q.jobs.iter().rev().filter(|j| !j.active()).cloned());
            v.truncate(5);
            v
        };
        let w = 420.0;
        // heights
        let mut hs = vec![];
        for j in &jobs {
            let mut h = 10.0 + 18.0 + 17.0 + 12.0 + 22.0 + 12.0;
            if j.state == crate::jobs::State::Asking {
                h += 12.0 + 178.0;
            }
            if matches!(j.state, crate::jobs::State::Done | crate::jobs::State::Undone | crate::jobs::State::Cancelled | crate::jobs::State::Failed(_)) {
                h -= 12.0;
            }
            hs.push(h);
        }
        let total_h = 12.0 + 36.0 + hs.iter().sum::<f32>() + if jobs.is_empty() { 48.0 } else { 0.0 };
        let r = Rect::new(self.w - 10.0 - w, self.h - 38.0 - total_h, w, total_h);
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        self.gfx.text("Transfers", Rect::new(r.x + 16.0, r.y + 8.0, 200.0, 32.0), Font::Ui, 14.0, 600, Align::Left, t.text);
        let all_paused = jobs.iter().filter(|j| j.active()).all(|j| j.paused()) && jobs.iter().any(|j| j.active());
        let pl = if all_paused { "Resume all" } else { "Pause all" };
        let pw = 12.0 + 14.0 + 8.0 + self.gfx.measure(pl, Font::Ui, 13.0, 400) + 12.0;
        if jobs.iter().any(|j| j.active()) {
            self.button(Rect::new(r.r() - 10.0 - pw, r.y + 8.0, pw, 32.0), pl, if all_paused { 0xE768 } else { 0xE769 }, Btn::Ghost, Hit::XferHead(0), t);
        }
        if jobs.is_empty() {
            self.gfx.text("Nothing is copying", Rect::new(r.x + 16.0, r.y + 48.0, 300.0, 32.0), Font::Ui, 13.0, 400, Align::Left, t.text3);
        }
        let mut y = r.y + 6.0 + 36.0;
        for (i, j) in jobs.iter().enumerate() {
            if i > 0 {
                self.gfx.fill(Rect::new(r.x, y, r.w, 1.0), t.divider);
            }
            let jr = Rect::new(r.x + 16.0, y + 10.0, r.w - 32.0, hs[i] - 22.0);
            let ic = Rect::new(jr.x, jr.y, 32.0, 32.0);
            self.gfx.fill_round(ic, R_CTRL, t.layer2);
            let (g, gc) = match &j.state {
                crate::jobs::State::Done | crate::jobs::State::Undone => (0xE73E, t.ok),
                crate::jobs::State::Failed(_) | crate::jobs::State::Cancelled => (0xE7BA, t.warn),
                _ if j.kind == crate::jobs::Kind::Move => (0xE8DE, t.accent_text),
                _ => (0xE8C8, t.accent_text),
            };
            self.gfx.glyph(g, ic, 15.0, gc);
            let bx = jr.x + 44.0;
            let bw = jr.r() - 70.0 - bx;
            self.gfx.text(&j.title(), Rect::new(bx, jr.y, bw, 18.0), Font::Ui, 13.0, 600, Align::Left, t.text);
            let dest = format!("to {}", j.dest.display());
            self.gfx.text(&dest, Rect::new(bx, jr.y + 19.0, bw + 60.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
            let mut yy = jr.y + 37.0;
            let line = match &j.state {
                crate::jobs::State::Waiting => format!("Waiting · {}", fmt::size(j.total)),
                crate::jobs::State::Done => format!("Done · {}{}", fmt::size(j.total), if j.identical > 0 { format!(" · {} identical skipped", j.identical) } else { String::new() }),
                crate::jobs::State::Undone => "Undone".into(),
                crate::jobs::State::Cancelled => "Cancelled".into(),
                crate::jobs::State::Failed(e) => e.clone(),
                _ => {
                    let sp = if j.speed > 0.0 && !j.paused() { format!("   {}/s", fmt::size(j.speed as u64)) } else { String::new() };
                    let eta = j.eta.filter(|_| !j.paused()).map(|e| format!("   {}", fmt::duration(e))).unwrap_or_default();
                    format!("{} of {}{sp}{eta}{}", fmt::size(j.done), fmt::size(j.total), if j.paused() { "   Paused" } else { "" })
                }
            };
            if j.active() && j.state != crate::jobs::State::Waiting {
                let pr = Rect::new(bx, yy + 1.0, bw + 26.0, 4.0);
                self.gfx.fill_round(pr, 2.0, t.track);
                let f = if j.total > 0 { j.done as f32 / j.total as f32 } else { 0.0 };
                self.gfx.fill_round(Rect::new(pr.x, pr.y, (pr.w * f).max(3.0), 4.0), 2.0, t.accent);
                yy += 12.0;
            }
            self.gfx.text(&line, Rect::new(bx, yy, bw + 60.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
            // actions
            if j.active() {
                let pr = Rect::new(jr.r() - 62.0 + 6.0, jr.y - 2.0, 30.0, 30.0);
                self.icon_btn(pr, if j.paused() || j.state == crate::jobs::State::Waiting && j.paused() { 0xE768 } else { 0xE769 }, 13.0, t.text, Hit::Job(j.id, 0), R_CTRL, t);
                self.icon_btn(Rect::new(pr.r() + 2.0, pr.y, 30.0, 30.0), 0xE711, 11.0, t.text, Hit::Job(j.id, 1), R_CTRL, t);
            } else if j.state == crate::jobs::State::Done && !j.journal.is_empty() {
                let uw = 12.0 + 14.0 + 8.0 + self.gfx.measure("Undo", Font::Ui, 13.0, 400) + 10.0;
                self.button(Rect::new(jr.r() - uw + 6.0, jr.y - 2.0, uw, 30.0), "Undo", 0xE7A7, Btn::Ghost, Hit::Job(j.id, 2), t);
            }
            if j.state == crate::jobs::State::Asking
                && let Some(c) = j.clashes.iter().find(|c| c.choice.is_none())
            {
                let cr = Rect::new(bx, yy + 26.0, bw + 26.0, 170.0);
                self.gfx.fill_round(cr, R_CTRL, t.warn_fill);
                self.gfx.stroke_round(cr, R_CTRL, t.warn_stroke, 1.0);
                self.gfx.glyph(0xE7BA, Rect::new(cr.x + 12.0, cr.y + 10.0, 14.0, 18.0), 14.0, t.warn);
                let name = c.src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.gfx.text(&name, Rect::new(cr.x + 34.0, cr.y + 10.0, cr.w - 46.0, 18.0), Font::Ui, 12.5, 600, Align::Left, t.text);
                let place = c.target.parent().map(|p| folder_title(p)).unwrap_or_default();
                self.gfx.text(&format!("is already in {place}"), Rect::new(cr.x + 34.0, cr.y + 28.0, cr.w - 46.0, 18.0), Font::Ui, 12.5, 400, Align::Left, t.text);
                let rows = [("Copying", c.src_size, c.src_time), (&*format!("In {place}"), c.dst_size, c.dst_time)];
                let mut ry = cr.y + 56.0;
                for (lab, sz, tm) in rows {
                    self.gfx.text(lab, Rect::new(cr.x + 34.0, ry, 64.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                    self.gfx.text(&format!("{} · {}", fmt::size(sz), fmt::date(tm, false)), Rect::new(cr.x + 34.0 + 64.0, ry, cr.w - 110.0, 18.0), Font::Ui, 12.0, 600, Align::Left, t.text);
                    ry += 20.0;
                }
                let bw3 = (cr.w - 24.0 - 12.0) / 3.0;
                for (k, (lab, style)) in [("Skip", Btn::Accent), ("Replace", Btn::Std), ("Keep both", Btn::Std)].iter().enumerate() {
                    self.button(Rect::new(cr.x + 12.0 + k as f32 * (bw3 + 6.0), cr.y + 104.0, bw3, 30.0), lab, 0, *style, Hit::Clash(j.id, k as u8), t);
                }
                let others = j.open_clashes().saturating_sub(1);
                if others > 0 {
                    let bxr = Rect::new(cr.x + 12.0, cr.y + 144.0, 18.0, 18.0);
                    if j.same_for_rest {
                        self.gfx.fill_round(bxr, R_SMALL, t.accent);
                        self.gfx.glyph(0xE73E, bxr, 11.0, t.on_accent);
                    } else {
                        self.gfx.fill_round(bxr, R_SMALL, t.field);
                        self.gfx.stroke_round(bxr, R_SMALL, t.field_bottom, 1.0);
                    }
                    let lab = format!("Same choice for the other {others} conflict{}", if others == 1 { "" } else { "s" });
                    self.gfx.text(&lab, Rect::new(bxr.r() + 8.0, bxr.y - 1.0, cr.w - 50.0, 20.0), Font::Ui, 12.0, 400, Align::Left, t.text);
                    self.hit(Rect::new(bxr.x, bxr.y, cr.w - 24.0, 18.0), Hit::SameChoice(j.id));
                }
            }
            y += hs[i];
        }
    }

    fn paint_palette(&mut self, t: &Theme) {
        let Some(p) = self.palette.as_ref() else { return };
        let k = prog(p.at, 150.0, self.shooting);
        let field = p.field.clone();
        let sel = p.sel;
        let items: Vec<(String, String, bool, u32, Vec<&'static str>, Vec<usize>, bool)> = p.items.iter().map(|i| (i.label.clone(), i.sub.clone(), i.folder, i.glyph, i.keys.clone(), i.hl.clone(), i.loc.is_some())).collect();
        let groups = items.iter().filter(|i| i.6).count();
        let actions = items.len() - groups;
        let h = 48.0 + if groups > 0 { 30.0 } else { 0.0 } + if actions > 0 { 30.0 } else { 0.0 } + 40.0 * items.len() as f32 + 6.0;
        let o = self.omni_rect();
        let r = Rect::new(o.x, 44.0 - (1.0 - k) * 8.0, 720.0f32.min(self.w - o.x - 10.0), h);
        self.gfx.opacity_layer(r.inset(-40.0, -40.0), k.clamp(0.05, 1.0));
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        let ir = Rect::new(r.x, r.y, r.w, 48.0);
        self.gfx.glyph(0xE721, Rect::new(ir.x + 16.0, ir.y, 16.0, ir.h), 15.0, t.text2);
        self.field_text(&field, Rect::new(ir.x + 44.0, ir.y, ir.w - 110.0, ir.h), 15.0, true, t, &[]);
        let _ = self.kbd(ir.r() - 16.0 - 30.0, ir.y + 14.0, "Esc", t);
        self.gfx.fill(Rect::new(r.x, ir.b() - 1.0, r.w, 1.0), t.divider);
        let mut y = ir.b();
        let mut last_group = None;
        for (i, (label, sub, folder, glyph, keys, hl, is_loc)) in items.iter().enumerate() {
            if last_group != Some(*is_loc) {
                self.gfx.text(if *is_loc { "Folders" } else { "Actions" }, Rect::new(r.x + 18.0, y + 10.0, 200.0, 16.0), Font::Ui, 12.0, 600, Align::Left, t.text3);
                y += 30.0;
                last_group = Some(*is_loc);
            }
            let rr = Rect::new(r.x + 6.0, y, r.w - 12.0, 40.0);
            if i == sel {
                self.gfx.fill_round(rr, R_CTRL, t.sel);
                self.gfx.fill_round(Rect::new(rr.x, rr.y + 13.0, 3.0, 14.0), 1.5, t.accent);
            } else if self.hov(Hit::Pal(i)) {
                self.gfx.fill_round(rr, R_CTRL, t.hover);
            }
            if *folder {
                ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(rr.x + 12.0, rr.cy() - 9.0, 18.0, 18.0));
            } else {
                self.gfx.glyph(*glyph, Rect::new(rr.x + 12.0, rr.y, 20.0, rr.h), 16.0, t.text2);
            }
            // matched letters in accent
            let mut spans = vec![];
            let mut u = 0u32;
            for (ci, ch) in label.chars().enumerate() {
                let l = ch.len_utf16() as u32;
                if hl.contains(&ci) {
                    spans.push(Span { start: u, len: l, color: t.accent_text, weight: 400, italic: false });
                }
                u += l;
            }
            let lw = self.gfx.measure(label, Font::Ui, 13.0, 400);
            self.gfx.text_spans(label, Rect::new(rr.x + 44.0, rr.y, lw + 4.0, rr.h), Font::Ui, 13.0, 400, t.text, &spans);
            let mut kx = rr.r() - 12.0;
            let kws: Vec<f32> = keys.iter().map(|k| self.gfx.measure(k, Font::Ui, 11.5, 400) + 12.0).map(|w: f32| w.max(20.0)).collect();
            let ktot: f32 = kws.iter().sum::<f32>() + 4.0 * kws.len().saturating_sub(1) as f32;
            kx -= ktot;
            self.gfx.text(sub, Rect::new(rr.x + 44.0 + lw + 10.0, rr.y, (kx - rr.x - 54.0 - lw - 10.0).max(0.0), rr.h), Font::Ui, 12.0, 400, Align::Left, t.text3);
            for kk in keys.iter() {
                let w = self.kbd(kx, rr.y + 10.0, kk, t);
                kx += w + 4.0;
            }
            self.hit(rr, Hit::Pal(i));
            y += 40.0;
        }
        self.gfx.pop_layer();
        if k != 1.0 {
            self.anim();
        }
    }

    fn paint_rename(&mut self, t: &Theme) {
        let Some(rn) = self.rename.as_ref() else { return };
        let k = prog(rn.at, 180.0, self.shooting);
        let (mode, n, rows, problem, ai_busy, ai_err, style) = (rn.mode, rn.items.len(), rn.rows.clone(), rn.problem.clone(), rn.ai_busy, rn.ai_err.clone(), rn.style);
        let (pattern, find, repl, start, digits, focus) = (rn.pattern.clone(), rn.find.clone(), rn.repl.clone(), rn.start.clone(), rn.digits, rn.focus);
        self.gfx.fill(Rect::new(0.0, 0.0, self.w, self.h), t.scrim.a(t.scrim.3 * k));
        self.hit(Rect::new(0.0, 0.0, self.w, self.h), Hit::Block);
        let shown = rows.len().min(9);
        let w = 800.0f32.min(self.w - 40.0);
        let h = 20.0 + 44.0 + 16.0 + 18.0 + 6.0 + 34.0 + 16.0 + 33.0 + 31.0 * shown as f32 + 16.0 + 68.0;
        let r = Rect::new(((self.w - w) / 2.0).max(190.0f32.min(self.w - w - 20.0)), (self.h - h) / 2.0 + (1.0 - k) * 8.0, w, h);
        self.gfx.opacity_layer(r.inset(-60.0, -60.0), k.clamp(0.05, 1.0));
        self.card(r, R_MENU, t, true);
        self.hit(r, Hit::Block);
        self.gfx.text(&format!("Rename {n} item{}", if n == 1 { "" } else { "s" }), Rect::new(r.x + 24.0, r.y + 20.0, 300.0, 44.0), Font::Display, 20.0, 600, Align::Left, t.text);
        // mode segmented + suggest names
        let mut x = r.r() - 24.0;
        if self.ai_on {
            let sw = 12.0 + 14.0 + 8.0 + self.gfx.measure("Suggest names", Font::Ui, 13.0, 400) + 12.0;
            x -= sw;
            let br = Rect::new(x, r.y + 22.0, sw, 32.0);
            if mode == 3 {
                self.gfx.fill_round(br, R_CTRL, t.sel);
                self.gfx.stroke_round(br, R_CTRL, t.sel_stroke, 1.0);
                self.gfx.glyph(0xF4A5, Rect::new(br.x + 12.0, br.y, 14.0, br.h), 13.0, t.accent_text);
                self.gfx.text("Suggest names", Rect::new(br.x + 34.0, br.y, sw - 40.0, br.h), Font::Ui, 13.0, 400, Align::Left, t.text);
                self.hit(br, Hit::Dlg(13));
            } else {
                self.button(br, "Suggest names", 0xF4A5, Btn::Std, Hit::Dlg(13), t);
            }
            x -= 12.0;
        }
        let labels = ["Pattern", "Find & replace", "Numbers"];
        let ws: Vec<f32> = labels.iter().map(|l| self.gfx.measure(l, Font::Ui, 13.0, 600) + 28.0).collect();
        let segw: f32 = ws.iter().sum::<f32>() + 2.0 * 4.0;
        let seg = Rect::new(x - segw, r.y + 20.0, segw, 36.0);
        self.gfx.fill_round(seg, R_CTRL, t.layer2);
        self.gfx.stroke_round(seg, R_CTRL, t.stroke, 1.0);
        let mut sx = seg.x + 3.0;
        for (i, l) in labels.iter().enumerate() {
            let sr = Rect::new(sx, seg.y + 3.0, ws[i], 30.0);
            let on = mode == i as u8;
            if on {
                self.gfx.fill_round(sr, R_SMALL, t.card);
                self.gfx.stroke_round(sr, R_SMALL, t.stroke, 1.0);
            } else if self.hov(Hit::Dlg(10 + i as u8)) {
                self.gfx.fill_round(sr, R_SMALL, t.hover);
            }
            self.gfx.text(l, sr, Font::Ui, 13.0, if on { 600 } else { 400 }, Align::Center, if on { t.text } else { t.text2 });
            self.hit(sr, Hit::Dlg(10 + i as u8));
            sx += ws[i] + 2.0;
        }
        // inputs
        let mut y = r.y + 20.0 + 44.0 + 16.0;
        let fx = r.x + 24.0;
        let fw = r.w - 48.0;
        let tok_spans = |s: &str, t: &Theme| -> Vec<Span> {
            let mut v = vec![];
            let mut u = 0u32;
            let mut in_tok = None;
            for ch in s.chars() {
                if ch == '{' {
                    in_tok = Some(u);
                }
                u += ch.len_utf16() as u32;
                if ch == '}'
                    && let Some(st) = in_tok.take()
                {
                    v.push(Span { start: st, len: u - st, color: t.accent_text, weight: 600, italic: false });
                }
            }
            v
        };
        match mode {
            0 | 2 => {
                let pw = fw - 12.0 - 96.0 - 12.0 - 96.0;
                self.gfx.text(if mode == 0 { "Pattern" } else { "Name" }, Rect::new(fx, y, 200.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                self.gfx.text("Start at", Rect::new(fx + pw + 12.0, y, 96.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                self.gfx.text("Digits", Rect::new(fx + pw + 12.0 + 96.0 + 12.0, y, 96.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                y += 24.0;
                let pr = Rect::new(fx, y, pw, 34.0);
                self.field_box(pr, focus == 0, t);
                let spans = tok_spans(&pattern.text, t);
                self.field_text(&pattern, Rect::new(pr.x + 10.0, pr.y, pr.w - 80.0, pr.h), 13.0, focus == 0, t, &spans);
                self.hit(pr, Hit::Dlg(20));
                let ir = Rect::new(pr.r() - 66.0, pr.y + 4.0, 60.0, 26.0);
                if self.hov(Hit::Dlg(21)) {
                    self.gfx.fill_round(ir, R_SMALL, t.hover);
                }
                self.gfx.text("Insert", Rect::new(ir.x + 8.0, ir.y, 40.0, ir.h), Font::Ui, 12.0, 400, Align::Left, t.text);
                self.gfx.glyph(0xE70D, Rect::new(ir.r() - 16.0, ir.y, 9.0, ir.h), 8.0, t.text2);
                self.hit(ir, Hit::Dlg(21));
                let sr = Rect::new(pr.r() + 12.0, y, 96.0, 34.0);
                self.field_box(sr, focus == 1, t);
                self.field_text(&start, Rect::new(sr.x + 10.0, sr.y, sr.w - 20.0, sr.h), 13.0, focus == 1, t, &[]);
                self.hit(sr, Hit::Dlg(22));
                self.dropdown(Rect::new(sr.r() + 12.0, y + 1.0, 96.0, 32.0), &digits.to_string(), Hit::Dlg(23), t, Font::Ui);
            }
            1 => {
                let hw = (fw - 12.0) / 2.0;
                self.gfx.text("Find", Rect::new(fx, y, 200.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                self.gfx.text("Replace with", Rect::new(fx + hw + 12.0, y, 200.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                y += 24.0;
                let a = Rect::new(fx, y, hw, 34.0);
                self.field_box(a, focus == 0, t);
                self.field_text(&find, Rect::new(a.x + 10.0, a.y, a.w - 20.0, a.h), 13.0, focus == 0, t, &[]);
                self.hit(a, Hit::Dlg(20));
                let b = Rect::new(fx + hw + 12.0, y, hw, 34.0);
                self.field_box(b, focus == 1, t);
                self.field_text(&repl, Rect::new(b.x + 10.0, b.y, b.w - 20.0, b.h), 13.0, focus == 1, t, &[]);
                self.hit(b, Hit::Dlg(22));
            }
            _ => {
                self.gfx.text("Style", Rect::new(fx, y, 200.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                y += 24.0;
                let tw = 12.0 + 14.0 + 8.0 + self.gfx.measure("Try again", Font::Ui, 13.0, 400) + 12.0;
                self.dropdown(Rect::new(fx, y + 1.0, fw - tw - 12.0, 32.0), crate::input::STYLES[style], Hit::Dlg(24), t, Font::Ui);
                self.button(Rect::new(fx + fw - tw, y + 1.0, tw, 32.0), "Try again", 0xE72C, Btn::Std, Hit::Dlg(25), t);
            }
        }
        y += 34.0 + 16.0;
        // preview table
        let tr = Rect::new(fx, y, fw, 33.0 + 31.0 * shown as f32);
        self.gfx.fill_round(tr, 8.0, t.layer);
        self.gfx.stroke_round(tr, 8.0, t.stroke, 1.0);
        self.gfx.clip(Rect::new(tr.x + 1.0, tr.y + 1.0, tr.w - 2.0, 31.0));
        self.gfx.fill_round(tr, 8.0, t.layer2);
        self.gfx.unclip();
        self.gfx.fill(Rect::new(tr.x, tr.y + 32.0, tr.w, 1.0), t.divider);
        let half = (tr.w - 28.0 - 24.0) / 2.0;
        self.gfx.text("Current name", Rect::new(tr.x + 14.0, tr.y, half, 32.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        self.gfx.text("New name", Rect::new(tr.x + 14.0 + half + 24.0, tr.y, half, 32.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        let mut ry = tr.y + 33.0;
        for (i, (old, new, ch)) in rows.iter().take(shown).enumerate() {
            if i > 0 {
                self.gfx.fill(Rect::new(tr.x, ry - 1.0, tr.w, 1.0), t.divider);
            }
            self.gfx.text(old, Rect::new(tr.x + 14.0, ry, half, 30.0), Font::Ui, 13.0, 400, Align::Left, t.text2);
            self.gfx.glyph(0xE72A, Rect::new(tr.x + 14.0 + half, ry, 24.0, 30.0), 10.0, t.text3);
            let spans = ch.map(|(s, l)| vec![Span { start: s, len: l, color: t.accent_text, weight: 600, italic: false }]).unwrap_or_default();
            self.gfx.text_spans(new, Rect::new(tr.x + 14.0 + half + 24.0, ry, half, 30.0), Font::Ui, 13.0, 400, t.text, &spans);
            ry += 31.0;
        }
        if mode == 3 && rows.is_empty() {
            let msg = if ai_busy { "Reading the files...".to_string() } else { ai_err.clone().unwrap_or_default() };
            self.gfx.text(&msg, Rect::new(tr.x + 14.0, tr.y + 33.0, tr.w - 28.0, 30.0), Font::Ui, 13.0, 400, Align::Left, t.text3);
        }
        // footer
        let fr = Rect::new(r.x, r.b() - 68.0, r.w, 68.0);
        self.gfx.clip(fr);
        self.gfx.fill_round(Rect::new(r.x, fr.y - 20.0, r.w, 88.0), 10.0, t.layer2);
        self.gfx.unclip();
        self.gfx.fill(Rect::new(fr.x, fr.y, fr.w, 1.0), t.divider);
        let changed = rows.iter().filter(|r| r.0 != r.1).count();
        let (icon, ic, info) = match &problem {
            Some(p) => (0xE7BA, t.warn, p.clone()),
            None => (0xE73E, t.ok, format!("{changed} new name{} · no clashes", if changed == 1 { "" } else { "s" })),
        };
        self.gfx.glyph(icon, Rect::new(fr.x + 24.0, fr.y, 14.0, fr.h), 13.0, ic);
        self.gfx.text(&info, Rect::new(fr.x + 44.0, fr.y, fr.w - 330.0, fr.h), Font::Ui, 12.0, 400, Align::Left, t.text2);
        let rw = 96.0;
        let ok = problem.is_none() && changed > 0;
        self.button(Rect::new(fr.r() - 24.0 - rw, fr.y + 17.0, rw, 32.0), "Rename", 0, if ok { Btn::Accent } else { Btn::Disabled }, Hit::Dlg(1), t);
        self.button(Rect::new(fr.r() - 24.0 - rw - 8.0 - rw, fr.y + 17.0, rw, 32.0), "Cancel", 0, Btn::Std, Hit::Dlg(0), t);
        self.gfx.pop_layer();
        if k != 1.0 {
            self.anim();
        }
    }

    fn paint_preview(&mut self, t: &Theme) {
        let Some(pv) = self.pv.as_ref() else { return };
        let k = prog(pv.at, 200.0, self.shooting);
        let from = pv.from;
        let path = pv.path.clone();
        let data = pv.data.clone();
        let scroll = pv.scroll;
        self.gfx.fill(Rect::new(0.0, 0.0, self.w, self.h), t.scrim.a(t.scrim.3 * k));
        self.hit(Rect::new(0.0, 0.0, self.w, self.h), Hit::Pv(9));
        let w = 920.0f32.min(self.w - 40.0);
        let h = 632.0f32.min(self.h - 40.0);
        let fin = Rect::new((self.w - w) / 2.0, (self.h - h) / 2.0, w, h);
        // grows from the row to the centre
        let r = Rect::new(from.x + (fin.x - from.x) * k, from.y + (fin.y - from.y) * k, from.w + (fin.w - from.w) * k, from.h + (fin.h - from.h) * k);
        self.gfx.opacity_layer(fin.inset(-60.0, -60.0), k.clamp(0.05, 1.0));
        self.card(r, R_MENU, t, true);
        self.hit(r, Hit::Block);
        if k != 1.0 {
            self.anim();
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let is_dir = path.is_dir();
        // header
        let hr = Rect::new(r.x, r.y, r.w, 56.0);
        ficons::draw(&self.gfx, t, ficons::kind_of(&name, is_dir), Rect::new(hr.x + 18.0, hr.cy() - 12.0, 24.0, 24.0));
        let mut bx = hr.r() - 10.0 - 36.0;
        self.icon_btn(Rect::new(bx, hr.y + 10.0, 36.0, 36.0), 0xE711, 12.0, t.text, Hit::Pv(0), R_CTRL, t);
        bx -= 12.0;
        self.gfx.fill(Rect::new(bx, hr.y + 18.0, 1.0, 20.0), t.divider);
        bx -= 12.0;
        let ow = self.gfx.measure("Open", Font::Ui, 13.0, 400) + 30.0;
        bx -= ow;
        self.button(Rect::new(bx, hr.y + 12.0, ow, 32.0), "Open", 0, Btn::Accent, Hit::Pv(1), t);
        bx -= 12.0;
        let ww = self.gfx.measure("Open with", Font::Ui, 13.0, 400) + 12.0 + 9.0 + 6.0 + 12.0;
        bx -= ww;
        let owr = Rect::new(bx, hr.y + 12.0, ww, 32.0);
        self.button(owr, "", 0, Btn::Std, Hit::Pv(2), t);
        self.gfx.text("Open with", Rect::new(owr.x + 12.0, owr.y, ww - 24.0, owr.h), Font::Ui, 13.0, 400, Align::Left, t.text);
        self.gfx.glyph(0xE70D, Rect::new(owr.r() - 21.0, owr.y, 9.0, owr.h), 9.0, t.text2);
        if data.as_ref().map(|d| d.kind == PvKind::Text).unwrap_or(false) {
            let cw = 12.0 + 14.0 + 8.0 + self.gfx.measure("Copy", Font::Ui, 13.0, 400) + 12.0;
            bx -= 12.0 + cw;
            self.button(Rect::new(bx, hr.y + 12.0, cw, 32.0), "Copy", 0xE8C8, Btn::Ghost, Hit::Pv(3), t);
        }
        self.gfx.text(&name, Rect::new(hr.x + 54.0, hr.y + 8.0, bx - hr.x - 66.0, 20.0), Font::Ui, 14.0, 600, Align::Left, t.text);
        let meta = data.as_ref().map(|d| d.meta.clone()).unwrap_or_default();
        self.gfx.text(&meta, Rect::new(hr.x + 54.0, hr.y + 28.0, bx - hr.x - 66.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        self.gfx.fill(Rect::new(hr.x, hr.b() - 1.0, hr.w, 1.0), t.divider);
        // body
        let pdf = data.as_ref().map(|d| d.kind == PvKind::Pdf).unwrap_or(false);
        let body = Rect::new(r.x, hr.b(), r.w, r.h - 56.0 - if pdf { 36.0 } else { 0.0 });
        self.gfx.clip(body);
        let code = data.as_ref().map(|d| d.kind == PvKind::Text).unwrap_or(false);
        self.gfx.fill(body, if code { t.code_bg } else { t.layer2 });
        match &data {
            None => {
                self.gfx.text("Loading...", body, Font::Ui, 13.0, 400, Align::Center, t.text3);
            }
            Some(d) if d.kind == PvKind::Text => {
                let lh = 21.0;
                let mut y = body.y + 16.0 - scroll;
                let mono = d.code;
                for (i, line) in d.lines.iter().enumerate() {
                    if y > body.b() {
                        break;
                    }
                    if y + lh >= body.y {
                        self.gfx.text(&(i + 1).to_string(), Rect::new(body.x, y, 48.0 + 18.0 - 18.0, lh), Font::Mono, 13.0, 400, Align::Right, t.text3);
                        let s: String = line.iter().map(|x| x.0.as_str()).collect();
                        let mut spans = vec![];
                        let mut u = 0u32;
                        for (txt, tok) in line {
                            let l = txt.encode_utf16().count() as u32;
                            let (c, italic) = match tok {
                                Tok::Keyword => (Some(t.k), false),
                                Tok::Type => (Some(t.ty), false),
                                Tok::Str => (Some(t.st), false),
                                Tok::Comment => (Some(t.cm), true),
                                Tok::Number => (Some(t.nu), false),
                                Tok::Plain => (None, false),
                            };
                            if let Some(c) = c {
                                spans.push(Span { start: u, len: l, color: c, weight: 400, italic });
                            }
                            u += l;
                        }
                        let font = if mono { Font::Mono } else { Font::Ui };
                        self.gfx.text_spans(&s, Rect::new(body.x + 48.0 + 18.0, y, body.w - 90.0, lh), font, 13.0, 400, t.text, &spans);
                    }
                    y += lh;
                }
            }
            Some(d) => {
                if let Some((iw, ih, _)) = &d.img {
                    let key = std::path::PathBuf::from(format!("pv:{}", d.path.display()));
                    if self.bmp_target != self.gfx.target_id() {
                        self.bitmaps.clear();
                        self.bmp_target = self.gfx.target_id();
                    }
                    let bmp = match self.bitmaps.get(&key) {
                        Some(b) => b.clone(),
                        None => {
                            let (w0, h0, px) = d.img.as_ref().unwrap();
                            let b = self.gfx.make_bitmap(*w0, *h0, px);
                            self.bitmaps.insert(key, b.clone());
                            b
                        }
                    };
                    if let Some(b) = bmp {
                        let (iw, ih) = (*iw as f32, *ih as f32);
                        let small = d.kind == PvKind::Other;
                        let (mw, mh) = (body.w - 2.0 * 112.0, body.h - 56.0);
                        let s = if small { 1.0f32.min(160.0 / iw.max(ih)) * (iw.max(ih) / iw.max(ih)) } else { (mw / iw).min(mh / ih).min(4.0) };
                        let (dw, dh) = if small { (iw.min(160.0), ih.min(160.0)) } else { (iw * s, ih * s) };
                        let ir = Rect::new(body.cx() - dw / 2.0, body.cy() - dh / 2.0, dw, dh);
                        if !small {
                            self.gfx.shadow(ir, 2.0, 4.0, 10.0, if t.dark { 0.4 } else { 0.18 });
                        }
                        self.gfx.bitmap(&b, ir, 1.0);
                    }
                } else {
                    ficons::draw(&self.gfx, t, ficons::kind_of(&name, is_dir), Rect::new(body.cx() - 48.0, body.cy() - 48.0, 96.0, 96.0));
                }
                if d.kind == PvKind::Other {
                    self.gfx.text("No preview for this type. Press Open to open it.", Rect::new(body.x, body.cy() + 96.0, body.w, 20.0), Font::Ui, 12.0, 400, Align::Center, t.text3);
                }
            }
        }
        // prev / next arrows
        if !code {
            for (i, (x, g)) in [(body.x + 16.0, 0xE76Bu32), (body.r() - 16.0 - 40.0, 0xE76C)].iter().enumerate() {
                let ar = Rect::new(*x, body.cy() - 20.0, 40.0, 40.0);
                self.gfx.shadow(ar, 20.0, 3.0, 8.0, if t.dark { 0.3 } else { 0.1 });
                self.gfx.fill_round(ar, 20.0, t.glass);
                self.gfx.stroke_round(ar, 20.0, t.glass_stroke, 1.0);
                if self.hov(Hit::Pv(4 + i as u8)) {
                    self.gfx.fill_round(ar, 20.0, t.hover);
                }
                self.gfx.glyph(*g, ar, 14.0, t.text);
                self.hit(ar, Hit::Pv(4 + i as u8));
            }
        }
        self.gfx.unclip();
        if pdf {
            let fr = Rect::new(r.x, r.b() - 36.0, r.w, 36.0);
            self.gfx.fill(Rect::new(fr.x, fr.y, fr.w, 1.0), t.divider);
            let pages = data.as_ref().map(|d| d.pages).unwrap_or(0);
            let page = data.as_ref().map(|d| d.page).unwrap_or(0);
            let s = if pages > 0 { format!("Page {} of {pages}", page + 1) } else { "Page 1".into() };
            let sw = self.gfx.measure(&s, Font::Ui, 12.0, 400);
            self.gfx.text(&s, Rect::new(fr.cx() - sw / 2.0, fr.y, sw + 2.0, fr.h), Font::Ui, 12.0, 400, Align::Left, t.text2);
            let up = Rect::new(fr.cx() - sw / 2.0 - 34.0, fr.y + 4.0, 28.0, 28.0);
            let dn = Rect::new(fr.cx() + sw / 2.0 + 6.0, fr.y + 4.0, 28.0, 28.0);
            self.icon_btn(up, 0xE70E, 9.0, if page > 0 { t.text } else { t.text_dis }, Hit::Pv(6), R_SMALL, t);
            self.icon_btn(dn, 0xE70D, 9.0, if page + 1 < pages { t.text } else { t.text_dis }, Hit::Pv(7), R_SMALL, t);
        }
        self.gfx.pop_layer();
    }

    fn paint_ghost(&mut self, t: &Theme) {
        let Some(d) = self.drag_in.as_ref() else { return };
        let (x, y, n, eff) = (d.x, d.y, d.paths.len(), d.effect);
        if eff == 0 {
            return;
        }
        let target = d.target.clone().or_else(|| self.pane().dir()).map(|p| folder_title(&p)).unwrap_or_default();
        let verb = if eff == 2 { "Move to " } else { "Copy to " };
        let vw = self.gfx.measure(verb, Font::Ui, 12.5, 400);
        let tw = self.gfx.measure(&target, Font::Ui, 12.5, 600);
        let r = Rect::new(x + 14.0, y + 14.0, 10.0 + 24.0 + 16.0 + vw + tw + 14.0, 40.0);
        self.card(r, R_MENU, t, false);
        let k0 = d.paths.first().map(|p| ficons::kind_of(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p.is_dir())).unwrap_or(Kind::File);
        ficons::draw(&self.gfx, t, k0, Rect::new(r.x + 10.0, r.cy() - 12.0, 24.0, 24.0));
        let bs = n.to_string();
        let bw = self.gfx.measure(&bs, Font::Ui, 11.0, 600) + 10.0;
        let br = Rect::new(r.x + 14.0, r.y - 8.0, bw.max(18.0), 18.0);
        self.gfx.fill_round(br, 9.0, t.accent);
        self.gfx.text(&bs, br, Font::Ui, 11.0, 600, Align::Center, t.on_accent);
        self.gfx.text(verb, Rect::new(r.x + 50.0, r.y, vw + 2.0, r.h), Font::Ui, 12.5, 400, Align::Left, t.text);
        self.gfx.text(&target, Rect::new(r.x + 50.0 + vw, r.y, tw + 2.0, r.h), Font::Ui, 12.5, 600, Align::Left, t.text);
    }
}

