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

/// Settings > Accent > Custom: height of the colour picker under the swatches
const PICK_H: f32 = 108.0;

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Std,
    Accent,
    /// destructive (delete for good, empty the bin)
    Danger,
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
    // the tiles share the whole width (no empty band on the right)
    let cols = (((avail + 12.0) / (260.0 + 12.0)).floor() as usize).max(1);
    let tw = (avail + 12.0) / cols as f32 - 12.0;
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
    // the Recycle Bin always shows where each item came from (in the Type column's place)
    let bin = a.pane().loc == Loc::RecycleBin;
    let mut want = [(lk.col_date, lk.w_date), (lk.col_type || bin, if bin { lk.w_type.max(220.0) } else { lk.w_type }), (lk.col_size, lk.w_size)];
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

    pub(crate) fn hov(&self, h: Hit) -> bool {
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
        self.name_w.clear();
        self.inline_rect = Rect::default();
        self.field_geo.clear();
        // set again by the sidebar / rail when they are drawn (a hidden sidebar must not catch the wheel)
        self.side_area = Rect::default();
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
        // One palette, one opacity: the title / tab strip and the sidebar are the window colour, the toolbar, content
        // and cards their own steps of the same hue, each drawn ONCE at the panel opacity (Personalize >
        // Transparency), so the Mica / Acrylic backdrop shows evenly through all of them. Text stays opaque.
        let pa = self.panel_alpha();
        let sx = self.main_rect().x;
        if self.mica && !self.gfx.offscreen() {
            self.gfx.clear(C(0.0, 0.0, 0.0, 0.0));
        } else {
            // Solid, or a test shot (it cannot show the backdrop): the opaque window colour underneath
            self.gfx.clear(t.mica_a);
        }
        if pa > 0.0 {
            self.gfx.fill(Rect::new(0.0, 0.0, self.w, TITLE_H), t.mica_a.a(pa));
            self.gfx.fill(Rect::new(0.0, TOP, sx, self.h - TOP), t.mica_a.a(pa));
        }
        self.paint_title(&t);
        self.paint_bar(&t);
        if self.rail() {
            self.paint_rail(&t);
        } else {
            self.paint_side(&t);
        }
        self.paint_main(&t);
        // a narrow window: the full sidebar over the content, opened with the menu button
        if self.side_overlay() {
            let r = Rect::new(0.0, TOP, self.look.side_w, self.h - TOP);
            self.gfx.shadow(r, 0.0, 0.0, 24.0, if t.dark { 0.5 } else { 0.18 });
            self.gfx.fill(r, t.mica_a);
            self.gfx.fill(Rect::new(r.r() - 1.0, r.y, 1.0, r.h), t.stroke);
            self.hit(r, Hit::Block);
            self.paint_side(&t);
        }
        self.paint_overlays(&t);
        self.paint_sug(&t);
        self.paint_tip(&t);
        self.gfx.end();
        // keep the frame timer only while something moves (no timer at all when idle)
        if !self.side_sf.settled() {
            moving |= self.side_sf.step();
            self.rail_scroll = self.side_sf.value.max(0.0);
        }
        moving |= self.hov_f.busy();
        moving |= self.press_f.busy();
        moving |= self.drop_f.busy();
        moving |= self.menu_out.is_some() || self.drop_out.is_some();
        if moving && !self.shooting {
            self.anim();
        }
        // caret blink only while a text field has focus
        let field = self.path_edit.is_some() || self.search_focus || self.inline.is_some() || self.palette.is_some() || self.rename.is_some() || self.key_focus || self.relay_focus != 0 || self.accent_field.is_some();
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
    pub(crate) fn press_in(&self, r: Rect, h: Hit) -> bool {
        let k = self.pk(h);
        if k > 0.001 {
            self.gfx.push_scale(r.cx(), r.cy(), 1.0 - (1.0 - crate::motion::PRESS_SCALE) * k, 0.0, 0.0);
            return true;
        }
        false
    }

    pub fn button(&mut self, r: Rect, label: &str, glyph: u32, style: Btn, h: Hit, t: &Theme) {
        self.round_button(r, label, glyph, style, h, R_CTRL, t);
    }

    pub(crate) fn round_button(&mut self, r: Rect, label: &str, glyph: u32, style: Btn, h: Hit, radius: f32, t: &Theme) {
        let hk = if style == Btn::Disabled { 0.0 } else { self.hk(h) };
        let pk = if style == Btn::Disabled { 0.0 } else { self.pk(h) };
        let scaled = style != Btn::Disabled && self.press_in(r, h);
        match style {
            Btn::Std => {
                self.gfx.fill_round(Rect::new(r.x, r.y + 1.0, r.w, r.h), radius, t.field_border);
                self.gfx.fill_round(r, radius, t.card.mix(t.layer2, pk));
                self.gfx.stroke_round(r, radius, t.stroke, 1.0);
                self.gfx.fill_round(r, radius, t.hover.a(t.hover.3 * hk * (1.0 - pk)));
            }
            Btn::Accent => {
                self.gfx.fill_round(r, radius, t.accent.mix(t.accent2, hk).mix(t.accent, pk * 0.6).a(1.0 - 0.12 * pk));
                self.gfx.fill(Rect::new(r.x + 4.0, r.b() - 1.0, r.w - 8.0, 1.0), C(0.0, 0.0, 0.0, 0.18));
            }
            Btn::Danger => {
                let red = if t.dark { C::hex(0xD13438) } else { C::hex(0xC42B1C) };
                self.gfx.fill_round(r, radius, red.mix(C::hex(0xFFFFFF), 0.10 * hk).a(1.0 - 0.12 * pk));
            }
            Btn::Ghost => {
                self.gfx.fill_round(r, radius, t.hover.a(t.hover.3 * hk));
                self.gfx.fill_round(r, radius, t.press.a(t.press.3 * pk * 0.6));
            }
            Btn::Disabled => {
                self.gfx.fill_round(r, radius, t.layer2);
                self.gfx.stroke_round(r, radius, t.stroke, 1.0);
            }
        }
        let fg = match style {
            Btn::Accent => t.on_accent,
            Btn::Danger => C::hex(0xFFFFFF),
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

    pub(crate) fn icon_btn(&mut self, r: Rect, glyph: u32, size: f32, color: C, h: Hit, rad: f32, t: &Theme) {
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

    /// A text input: one even, subtle border at rest; a clean accent ring all around when focused (no bottom bar).
    fn field_box(&mut self, r: Rect, focus: bool, t: &Theme) {
        self.gfx.fill_round(r, R_CTRL, t.field);
        if focus {
            self.gfx.stroke_round(r, R_CTRL, t.accent, 1.5);
        } else {
            self.gfx.stroke_round(r, R_CTRL, t.field_border, 1.0);
        }
    }

    /// Text of a field with selection and caret. Single line, left aligned; the text scrolls sideways the way an
    /// EDIT control's does (`fmt::edit_scroll`). `id` names the box so that clicks and drags find it again
    /// (`FieldId::None`: a masked box, keys only).
    pub fn field_text(&mut self, id: FieldId, f: &crate::fmt::Field, r: Rect, size: f32, focus: bool, t: &Theme, spans: &[Span]) {
        let caret_x = self.gfx.caret_pos(&f.text, Font::Ui, size, 400, f.caret, f.lead);
        let text_w = self.gfx.measure(&f.text, Font::Ui, size, 400);
        let off = f.scroll_for(caret_x, text_w, r.w - 2.0);
        if id != FieldId::None {
            self.field_geo.push(FieldGeo { id, r, size });
            if off != f.scroll
                && let Some(m) = self.field_by_id(id)
            {
                m.scroll = off;
            }
        }
        self.gfx.clip(r);
        if focus && f.has_sel() {
            let (a, b) = f.sel_range();
            for (x, w) in self.gfx.sel_rects(&f.text, Font::Ui, size, 400, a, b) {
                self.gfx.fill(Rect::new(r.x + x - off, r.cy() - 9.0, w, 18.0), t.accent.a(0.28));
            }
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
        // Windows 11 Explorer: tabs shrink to a minimum width, then the strip scrolls (arrows at both ends) and the
        // "+" stays put, so nothing ever runs under the caption buttons
        const MIN_TAB_W: f32 = 72.0;
        const ARROW_W: f32 = 24.0;
        let overflow = MIN_TAB_W * n as f32 > avail;
        let (sx0, sw) = if overflow { (8.0 + ARROW_W, avail - 2.0 * ARROW_W) } else { (8.0, avail) };
        let tw = (sw / n as f32).clamp(MIN_TAB_W, 200.0);
        self.tab_max = if overflow { (tw * n as f32 - sw).max(0.0) } else { 0.0 };
        // bring the active tab into view when it (or the number of tabs) changed
        if self.tab_seen != (self.cur, n) {
            self.tab_seen = (self.cur, n);
            if overflow {
                let (a, b) = (tw * self.cur as f32, tw * (self.cur + 1) as f32);
                if a < self.tab_scroll {
                    self.tab_scroll = a;
                } else if b > self.tab_scroll + sw {
                    self.tab_scroll = b - sw;
                }
            }
        }
        self.tab_scroll = self.tab_scroll.clamp(0.0, self.tab_max);
        let scroll = self.tab_scroll;
        self.tab_strip = if overflow { Rect::new(sx0, 0.0, sw, TITLE_H) } else { Rect::new(8.0, 0.0, tw * n as f32, TITLE_H) };
        let strip = Rect::new(sx0 - 3.0, 0.0, sw + 6.0, TITLE_H);
        let la = self.panel_alpha();
        let mut tab_hits = vec![];
        // Windows 11 Explorer: the active tab is the toolbar's own surface rising into the title strip (same fill,
        // rounded top corners, no bottom edge, small inverted curves where it meets the toolbar), so tab and toolbar
        // read as one piece. Other tabs stay flat on the strip with a rounded hover. Icon, title and close button
        // share one vertical centre on every tab, so nothing moves when the active tab changes.
        const TAB_TOP: f32 = 8.0;
        const TAB_GAP: f32 = 4.0;
        let th = TITLE_H - TAB_TOP - 2.0;
        let cy = TAB_TOP + th / 2.0;
        let hov: Vec<f32> = (0..n).map(|i| self.hk(Hit::Tab(i)).max(self.hk(Hit::TabClose(i)))).collect();
        // the active tab last: its feet lie over the gap to its neighbours
        let order: Vec<usize> = (0..n).filter(|&i| i != self.cur).chain((self.cur < n).then_some(self.cur)).collect();
        if overflow {
            self.gfx.clip(strip);
        }
        for i in order {
            let x = sx0 + tw * i as f32 - scroll;
            if overflow && (x + tw < strip.x || x > strip.r()) {
                continue;
            }
            let active = i == self.cur;
            let r = if active { Rect::new(x + TAB_GAP / 2.0, TAB_TOP, tw - TAB_GAP, TITLE_H - TAB_TOP) } else { Rect::new(x + TAB_GAP / 2.0, TAB_TOP, tw - TAB_GAP, th) };
            // the tab a tab menu is open for shows as hovered
            let hov_i = if self.menu.is_some() && self.menu_tab == Some(i) { 1.0 } else { hov[i] };
            if active {
                let (rad, foot) = (self.gfx.rad(R_CTRL), self.gfx.rad(6.0));
                self.gfx.fill_path(&tab_shape(r, rad, foot, true), 0.0, 0.0, 1.0, t.bar.a(la));
                let sr = Rect::new(r.x + 0.5, r.y + 0.5, r.w - 1.0, r.h - 0.5);
                self.gfx.stroke_path(&tab_shape(sr, rad, foot, false), 0.0, 0.0, 1.0, t.stroke.a(t.stroke.3 * 0.8), 1.0);
            } else {
                self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * hov_i));
                self.drop_light(r, Hit::Tab(i), R_CTRL, t);
                // a thin divider in the gap between two quiet tabs
                if i > 0 && i - 1 != self.cur {
                    let q = 1.0 - hov[i].max(hov[i - 1]);
                    if q > 0.01 {
                        self.gfx.fill(Rect::new(x - 0.5, cy - 7.0, 1.0, 14.0), t.stroke.a(t.stroke.3 * q));
                    }
                }
            }
            let tab = &self.tabs[i];
            let loc = tab.pane().loc.clone();
            let title = tab.title();
            // the icon stays where it is; the close button is on every tab while they are wide, and on the active and
            // the hovered tab when they are narrow; the title takes what is left
            let ir = Rect::new(r.x + 10.0, cy - 8.0, 16.0, 16.0);
            match loc {
                Loc::Settings => self.gfx.glyph(0xE713, ir, 15.0, t.text2),
                Loc::Cleanup => self.gfx.glyph(crate::cleanup_page::BROOM, ir, 15.0, t.text2),
                Loc::RecycleBin => self.gfx.glyph(0xE74D, ir, 15.0, t.text2),
                Loc::ThisPc => self.gfx.glyph(0xE7F8, ir, 15.0, t.text2),
                Loc::Folder(ref p) if p.parent().is_none() => ficons::draw(&self.gfx, t, Kind::Drive, ir),
                _ => ficons::draw(&self.gfx, t, Kind::Folder, ir),
            }
            let wide = tw >= 96.0;
            let show_close = wide || active || hov_i > 0.02;
            let cs = 20.0;
            let cr = Rect::new(r.r() - 6.0 - cs, cy - cs / 2.0, cs, cs);
            let tx = ir.r() + 8.0;
            let tail = if show_close { cr.x - 4.0 } else { r.r() - 8.0 };
            if tail - tx >= 24.0 {
                self.name_w.push((Hit::Tab(i), tail - tx));
                self.gfx.text_mid(&title, Rect::new(tx, cy - th / 2.0, tail - tx, th), Font::Ui, 12.0, 400, Align::Left, if active { t.text } else { t.text2 });
            } else {
                // no room for the name: the tooltip gives it
                self.name_w.push((Hit::Tab(i), 0.0));
            }
            // a hit rectangle never reaches under the scroll arrows
            let clipped = |r: Rect| if overflow { let x0 = r.x.max(strip.x); let x1 = r.r().min(strip.r()); Rect::new(x0, r.y, (x1 - x0).max(0.0), r.h) } else { r };
            tab_hits.push((clipped(r), Hit::Tab(i)));
            if show_close {
                let hk = self.hk(Hit::TabClose(i));
                let fade = if wide || active { 1.0 } else { hov_i.min(1.0) };
                self.gfx.fill_round(cr, R_SMALL, t.hover.a(t.hover.3 * 1.4 * hk));
                let fg = if hk > 0.5 { t.text } else { t.text2 };
                self.gfx.glyph(0xE711, cr, 10.0, fg.a(fg.3 * fade));
                tab_hits.push((clipped(cr), Hit::TabClose(i)));
            }
        }
        if overflow {
            self.gfx.unclip();
        }
        for (r, h) in tab_hits {
            self.hit(r, h);
        }
        if overflow {
            // scroll arrows at both ends of the strip
            let (lo, hi) = (self.tab_scroll > 0.5, self.tab_scroll < self.tab_max - 0.5);
            let ay = cy - 14.0;
            self.icon_btn(Rect::new(8.0, ay, ARROW_W, 28.0), 0xE76B, 10.0, if lo { t.text2 } else { t.text_dis }, Hit::TabScroll(-1), R_CTRL, t);
            self.icon_btn(Rect::new(sx0 + sw, ay, ARROW_W, 28.0), 0xE76C, 10.0, if hi { t.text2 } else { t.text_dis }, Hit::TabScroll(1), R_CTRL, t);
        }
        let ar = Rect::new(8.0 + if overflow { avail } else { tw * n as f32 } + 2.0, cy - 14.0, 28.0, 28.0);
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

    pub(crate) fn toolbar_compact(&self) -> bool {
        self.w < 1000.0
    }

    fn toolbar_action_width(&mut self, label: &str) -> f32 {
        if self.toolbar_compact() { 34.0 } else { 46.0 + self.gfx.measure(label, Font::Ui, 13.0, 400) }
    }

    fn toolbar_action(&mut self, r: Rect, label: &str, glyph: u32, enabled: bool, hit: Hit, t: &Theme) {
        if self.toolbar_compact() {
            self.icon_btn(r, glyph, 14.0, if enabled { t.text } else { t.text_dis }, hit, R_CTRL, t);
        } else {
            self.button(r, label, glyph, if enabled { Btn::Ghost } else { Btn::Disabled }, hit, t);
        }
    }

    fn toolbar_widths(&mut self) -> (f32, f32, f32) {
        let this_pc = self.pane().loc == Loc::ThisPc;
        let cleanup = if this_pc { self.toolbar_action_width("Clean up") } else { 0.0 };
        let new = if this_pc || self.w >= 560.0 { self.toolbar_action_width("New") } else { 0.0 };
        let view = if self.toolbar_compact() { 34.0 } else { 53.0 };
        (cleanup, new, view)
    }

    pub fn omni_rect(&mut self) -> Rect {
        let (cw, nw, vw) = self.toolbar_widths();
        let nx = self.w - 10.0 - vw - 4.0 - nw - if cw > 0.0 { cw + 4.0 } else { 0.0 };
        // very narrow: no forward button
        let x0 = self.nav_x() + if self.w < 560.0 { 82.0 } else { 120.0 };
        Rect::new(x0, TITLE_H + 7.0, (nx - 10.0 - x0).max(60.0), 34.0)
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
        self.gfx.fill(bar, t.bar.a(self.panel_alpha()));
        self.gfx.fill(Rect::new(0.0, TOP - 1.0, self.w, 1.0), t.divider);
        let pane = self.pane();
        let cleanup = pane.loc == Loc::Cleanup;
        let (can_back, can_fwd, can_up) = (!pane.back.is_empty() || self.cleanup_back_tab().is_some(), !pane.fwd.is_empty(), !matches!(pane.loc, Loc::ThisPc | Loc::Settings | Loc::Cleanup));
        let y = TITLE_H + 8.0;
        let tiny = self.w < 560.0;
        if self.side_hidden() {
            self.icon_btn(Rect::new(10.0, y, 34.0, 32.0), 0xE89F, 16.0, t.text, Hit::Side(usize::MAX), R_CTRL, t);
        }
        let nx0 = self.nav_x();
        self.icon_btn(Rect::new(nx0, y, 34.0, 32.0), 0xE72B, 16.0, if can_back { t.text } else { t.text_dis }, Hit::Back, R_CTRL, t);
        if !tiny {
            self.icon_btn(Rect::new(nx0 + 38.0, y, 34.0, 32.0), 0xE72A, 16.0, if can_fwd { t.text } else { t.text_dis }, Hit::Fwd, R_CTRL, t);
        }
        self.icon_btn(Rect::new(nx0 + if tiny { 38.0 } else { 76.0 }, y, 34.0, 32.0), 0xE74A, 16.0, if can_up { t.text } else { t.text_dis }, Hit::Up, R_CTRL, t);
        let o = self.omni_rect();
        let narrow = self.toolbar_compact();
        // Cleanup, New and View buttons.
        let mut nx = o.r() + 10.0;
        let (cw, nw, vw) = self.toolbar_widths();
        if cw > 0.0 {
            self.toolbar_action(Rect::new(nx, y, cw, 32.0), "Clean up", crate::cleanup_page::BROOM, true,
                Hit::Cleanup(crate::cleanup_page::Target::Entry), t);
            nx += cw + 4.0;
        }
        if nw > 0.0 {
            // New makes items only inside a folder.
            self.toolbar_action(Rect::new(nx, y, nw, 32.0), "New", 0xE710, self.pane().loc.path().is_some(), Hit::New, t);
            nx += nw + 4.0;
        }
        let vr = Rect::new(nx, y, vw, 32.0);
        let g = if self.tab().view == View::Grid { 0xF0E2 } else { 0xE8FD };
        // Settings and Clean up have no views: the button is dim, with no hover pill, tooltip or click
        let off = cleanup || self.pane().loc == Loc::Settings;
        if narrow {
            if off {
                self.gfx.glyph(g, vr, 14.0, t.text_dis);
            } else {
                self.icon_btn(vr, g, 14.0, t.text, Hit::ViewBtn, R_CTRL, t);
            }
        } else {
            let open = self.drop.as_ref().map(|d| d.kind == DropKind::View).unwrap_or(false);
            if !off {
                self.gfx.fill_round(vr, R_CTRL, t.hover.a(t.hover.3 * if open { 1.0 } else { self.hk(Hit::ViewBtn) }));
            }
            self.gfx.glyph(g, Rect::new(vr.x + 12.0, vr.y, 14.0, 32.0), 14.0, if off { t.text_dis } else { t.text });
            self.gfx.glyph(0xE70D, Rect::new(vr.x + 32.0, vr.y, 9.0, 32.0), 9.0, if off { t.text_dis } else { t.text2 });
            if !off {
                self.hit(vr, Hit::ViewBtn);
            }
        }
        // omnibox
        let searching = self.pane().search.is_some();
        let editing = self.path_edit.is_some();
        if editing || (searching && self.search_focus) {
            self.gfx.fill_round(o, R_CTRL, t.card);
            self.gfx.stroke_round(o, R_CTRL, t.accent, 1.5);
        } else if searching {
            self.gfx.fill_round(o, R_CTRL, t.card);
            self.gfx.stroke_round(o, R_CTRL, t.field_border, 1.0);
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
            self.field_text(FieldId::Path, &f, Rect::new(o.x + 12.0, o.y, o.w - 24.0, o.h), 13.0, true, t, &[]);
        } else if searching {
            // no hits + AI on: Enter runs AI Search now, so the magnifier turns into the AI sparkle with a short pop, one
            // ring out of the box and two slow glows, then rests (Sajjad, 2026-11-23)
            // also while AI is thinking (it keeps glowing) and while its results are shown
            let empty = self.pane().rows().is_empty();
            let (armed, asking) = self
                .pane()
                .search
                .as_ref()
                .map(|s| {
                    let asking = s.busy && !s.note.is_empty();
                    // (a search on its way after an empty answer keeps the sparkle: no pop again per word)
                    (self.ai_on && (asking || s.ai.is_some() || (empty && (!s.busy || s.quiet()) && !s.field.text.trim().is_empty())), asking)
                })
                .unwrap_or((false, false));
            if !armed {
                self.ai_armed = None;
            } else if self.ai_armed.is_none() {
                self.ai_armed = Some(Instant::now());
            }
            let gr = Rect::new(o.x + 10.0, o.y, 16.0, o.h);
            match self.ai_armed {
                Some(at) => {
                    let k = prog(at, 200.0, self.shooting);
                    let el = if self.shooting { 9.0 } else { at.elapsed().as_secs_f32() };
                    let kv = k.clamp(0.0, 1.0);
                    if kv < 1.0 {
                        self.gfx.glyph(0xE721, gr, (14.0 * (1.0 - kv)).max(0.5), t.text2.a(t.text2.3 * (1.0 - kv)));
                    }
                    if el < 0.6 {
                        let q = el / 0.6;
                        self.gfx.stroke_round(o.inset(-4.0 * q, -4.0 * q), R_CTRL + 4.0 * q, t.accent.a(0.5 * (1.0 - q)), 1.5);
                    }
                    let pulse = (el < 2.4 || asking) && !self.shooting;
                    let glow = if pulse { 0.7 + 0.3 * (el * std::f32::consts::TAU / 1.2).cos() } else { 1.0 };
                    self.gfx.glyph(0xF4A5, gr, (14.0 * k).max(0.5), t.accent_text.a(t.accent_text.3 * glow * kv));
                    if pulse || k != 1.0 {
                        self.anim();
                    }
                }
                None => self.gfx.glyph(0xE721, gr, 14.0, t.text2),
            }
            let f = self.pane().search.as_ref().unwrap().field.clone();
            let focus = self.search_focus;
            self.field_text(FieldId::Search, &f, Rect::new(o.x + 37.0, o.y, o.w - 37.0 - 36.0, o.h), 13.0, focus, t, &[]);
            if f.text.is_empty() {
                // with AI on, say that plain words work too
                let hint = if self.ai_on { "Search, or describe what you need in your own words" } else { "Search" };
                self.gfx.text(hint, Rect::new(o.x + 38.0, o.y, o.w - 80.0, o.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
            }
            let cr = Rect::new(o.r() - 32.0, o.y + 4.0, 26.0, 26.0);
            self.icon_btn(cr, 0xE711, 10.0, t.text2, Hit::OmniClear, R_SMALL, t);
        } else {
            // Settings has nothing to search: no Search button there
            let settings = cleanup || self.pane().loc == Loc::Settings;
            // the full "Search" button stays until the address box is really short (only the icon then)
            let sw = if settings { 4.0 } else if o.w < 230.0 { 34.0 } else { 86.0 };
            let sr = Rect::new(o.r() - sw, o.y + 4.0, sw - 4.0, 26.0);
            if !settings {
                self.gfx.fill_round(sr, R_SMALL, t.hover.a(t.hover.3 * self.hk(Hit::OmniSearch)));
                self.gfx.fill(Rect::new(sr.x, sr.y, 1.0, sr.h), t.divider);
                if sw < 40.0 {
                    self.gfx.glyph(0xE721, sr, 14.0, t.text2);
                } else {
                    self.gfx.glyph(0xE721, Rect::new(sr.x + 10.0, sr.y, 14.0, sr.h), 14.0, t.text2);
                    self.gfx.text("Search", Rect::new(sr.x + 32.0, sr.y, 60.0, sr.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
                }
                self.hit(sr, Hit::OmniSearch);
            }
            // location icon + crumbs
            let loc = self.pane().loc.clone();
            let ir = Rect::new(o.x + 9.0, o.cy() - 8.0, 16.0, 16.0);
            match loc {
                Loc::Settings => self.gfx.glyph(0xE713, Rect::new(ir.x, o.y, 16.0, o.h), 15.0, t.text2),
                Loc::Cleanup => self.gfx.glyph(crate::cleanup_page::BROOM, Rect::new(ir.x, o.y, 16.0, o.h), 15.0, t.text2),
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
                self.drop_light(r, Hit::Crumb(i), R_SMALL, t);
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
        let sw = if self.side_overlay() { self.look.side_w } else { self.side_w() };
        let x = 6.0;
        let w = sw - 12.0;
        let mut y = TOP + 8.0;
        // the menu button: collapse to icons (remembered), or close the overlay
        // collapse (or close the overlay): E8A0 reads as an arrow into the edge
        self.icon_btn(Rect::new(x, y, 40.0, 32.0), 0xE8A0, 16.0, t.text2, Hit::Side(usize::MAX), R_CTRL, t);
        y += 38.0;
        let bottom_limit = self.h - 8.0 - 32.0 - 8.0;
        // everything between the toggle and Settings scrolls (wheel / touchpad with momentum); nothing is dropped
        let area = Rect::new(0.0, y - 2.0, sw, (bottom_limit - y + 2.0).max(0.0));
        self.side_area = area;
        let y_top = y;
        y -= self.rail_scroll;
        self.gfx.clip(area);
        let vis = |r: Rect| r.b() > area.y + 1.0 && r.y < area.b() - 1.0;
        let mut reveal: Option<Rect> = None;
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
        if vis(r) {
            self.hit(r, Hit::SideThisPc);
        }
        if sel {
            reveal = Some(r);
        }
        y += 36.0;
        let side = self.side.clone();
        let base = side.len();
        if self.look.show_drives {
            group(self, &mut y, "Drives");
            let drives = self.drives.clone();
            for (i, d) in drives.iter().enumerate() {
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
                if vis(r) {
                    self.hit(r, Hit::Side(base + i));
                }
                if sel {
                    reveal = Some(r);
                }
                y += 46.0;
            }
            y += 6.0;
        }
        if self.look.show_pinned {
            group(self, &mut y, "Pinned");
            for (i, (name, glyph, color, loc)) in side.iter().enumerate() {
                let r = Rect::new(x, y, w, 32.0);
                let sel = self.side_selected(loc);
                if sel {
                    reveal = Some(r);
                }
                self.nav_bg(r, sel, Hit::Side(i), t);
                if *glyph == 0 {
                    let p = loc.path().map(|p| p.to_path_buf()).unwrap_or_default();
                    let (a, m) = self.dir_attrs(&p);
                    self.item_icon_a(&p, "", true, a, m, Kind::Folder, Rect::new(r.x + 10.0, r.cy() - 8.0, 16.0, 16.0), t);
                } else {
                    let c = if *color == 0 { t.text2 } else { C::hex(*color) };
                    let c = if t.dark && *color != 0 { c.mix(C::hex(0xFFFFFF), 0.25) } else { c };
                    self.gfx.glyph(*glyph, Rect::new(r.x + 10.0, r.y, 16.0, r.h), 16.0, c);
                }
                self.gfx.text(name, Rect::new(r.x + 38.0, r.y, r.w - 46.0, r.h), Font::Ui, fs, 400, Align::Left, t.text);
                if vis(r) {
                    self.hit(r, Hit::Side(i));
                }
                y += 32.0;
            }
            y += 6.0;
        }
        let recent = self.recent.clone();
        if self.look.show_recent && !recent.is_empty() {
            group(self, &mut y, "Recent");
            let base = base + self.drives.len();
            for (i, r0) in recent.iter().enumerate() {
                let r = Rect::new(x, y, w, 32.0);
                self.nav_bg(r, false, Hit::Side(base + i), t);
                ficons::draw(&self.gfx, t, ficons::kind_of(&r0.name, r0.is_dir), Rect::new(r.x + 10.0, r.cy() - 8.0, 16.0, 16.0));
                self.gfx.text(&r0.name, Rect::new(r.x + 38.0, r.y, r.w - 46.0, r.h), Font::Ui, fs, 400, Align::Left, t.text);
                if vis(r) {
                    self.hit(r, Hit::Side(base + i));
                }
                y += 32.0;
            }
        }
        self.gfx.unclip();
        let content = y + self.rail_scroll - y_top + 4.0;
        self.side_scrolled(area, content, reveal);
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

    /// After the sidebar (or rail) is drawn: keep its scroll in range, bring the selected place into view after a
    /// move, and show a thin scrollbar while the pointer is over it.
    fn side_scrolled(&mut self, area: Rect, content: f32, reveal: Option<Rect>) {
        let max = (content - area.h).max(0.0);
        self.rail_max = max;
        if self.rail_scroll > max {
            self.rail_scroll = max;
            self.side_sf.set(max);
        }
        if std::mem::take(&mut self.side_reveal)
            && let Some(r) = reveal
            && (r.y < area.y || r.b() > area.b())
        {
            let want = if r.y < area.y { self.rail_scroll - (area.y - r.y) - 8.0 } else { self.rail_scroll + (r.b() - area.b()) + 8.0 };
            if self.side_sf.settled() {
                self.side_sf.set(self.rail_scroll);
            }
            self.side_sf.to(want.clamp(0.0, max));
            self.anim();
        }
        if max > 0.0 && area.has(self.mx, self.my) {
            let th = (area.h * area.h / content).max(24.0);
            let ty = area.y + (area.h - th) * (self.rail_scroll / max);
            let c = if self.t.dark { C(1.0, 1.0, 1.0, 0.28) } else { C(0.0, 0.0, 0.0, 0.28) };
            self.gfx.fill_round(Rect::new(area.r() - 5.0, ty + 2.0, 3.0, th - 4.0), 1.5, c);
        }
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
        self.drop_light(r, h, R_CTRL, t);
    }

    /// The drop-target light of a place that files are dragged onto (fades in and out).
    pub(crate) fn drop_light(&mut self, r: Rect, h: Hit, rad: f32, t: &Theme) {
        let dk = self.drop_f.get(h);
        if dk > 0.001 {
            self.gfx.fill_round(r, rad, t.sel.a(t.sel.3 * dk));
            self.gfx.fill_round(r, rad, t.accent.a(0.10 * dk));
            self.gfx.stroke_round(r, rad, t.accent.a(dk), 1.5);
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
        // expand: E89F reads as an arrow out of the edge
        self.icon_btn(r, 0xE89F, 16.0, t.text2, Hit::Side(usize::MAX), R_CTRL, t);
        y += 38.0;
        // everything between the menu button and Settings scrolls when the window is short (nothing is dropped)
        let side = self.side.clone();
        let drives = self.drives.clone();
        let limit = self.h - 8.0 - 40.0;
        let area = Rect::new(0.0, y, x + 46.0, (limit - y).max(0.0));
        let n_d = if self.look.show_drives { drives.len() } else { 0 };
        let n_p = if self.look.show_pinned { side.len() } else { 0 };
        let content = 38.0 + 38.0 * n_d as f32 + if self.look.show_drives { 16.0 } else { 0.0 } + 38.0 * n_p as f32;
        let max = (content - area.h).max(0.0);
        self.side_scrolled(area, content, None);
        self.side_area = area;
        y -= self.rail_scroll;
        self.gfx.clip(area);
        let vis = |r: Rect| r.y >= area.y - 1.0 && r.b() <= area.b() + 1.0;
        // a row cut by the edge of the area fades with the part that shows (never a sliver of half an icon)
        let fade = |r: Rect| ((r.b().min(area.b()) - r.y.max(area.y)) / r.h).clamp(0.0, 1.0);
        let r = Rect::new(x, y, 40.0, 36.0);
        let sel = self.pane().loc == Loc::ThisPc;
        let f = fade(r);
        if f > 0.02 {
            if f < 0.99 {
                self.gfx.opacity_layer(r.inset(-2.0, -2.0), f * f);
            }
            self.nav_bg(r, sel, Hit::SideThisPc, t);
            self.gfx.glyph(0xE7F8, r, 16.0, t.text2);
            if f < 0.99 {
                self.gfx.pop_layer();
            }
        }
        if vis(r) {
            self.hit(r, Hit::SideThisPc);
        }
        y += 38.0;
        if self.look.show_drives {
            for (i, d) in drives.iter().enumerate() {
                let r = Rect::new(x, y, 40.0, 36.0);
                let loc = Loc::Folder(d.root.clone().into());
                let sel = self.side_selected(&loc);
                let f = fade(r);
                if f > 0.02 {
                    if f < 0.99 {
                        self.gfx.opacity_layer(r.inset(-2.0, -2.0), f * f);
                    }
                    self.nav_bg(r, sel, Hit::Side(side.len() + i), t);
                    let chip = Rect::new(r.cx() - 13.0, r.cy() - 10.0, 26.0, 20.0);
                    self.gfx.fill_round(chip, R_SMALL, t.layer2);
                    self.gfx.stroke_round(chip, R_SMALL, t.stroke, 1.0);
                    self.gfx.text(d.root.trim_end_matches('\\'), chip, Font::Ui, 12.0, 600, Align::Center, t.text2);
                    if f < 0.99 {
                        self.gfx.pop_layer();
                    }
                }
                if vis(r) {
                    self.hit(r, Hit::Side(side.len() + i));
                }
                y += 38.0;
            }
            y += 6.0;
            self.gfx.fill(Rect::new(x + 8.0, y, 24.0, 1.0), t.divider);
            y += 10.0;
        }
        if self.look.show_pinned {
            for (i, (_, glyph, color, loc)) in side.iter().enumerate() {
                let r = Rect::new(x, y, 40.0, 36.0);
                let sel = self.side_selected(loc);
                let f = fade(r);
                if f > 0.02 {
                    if f < 0.99 {
                        self.gfx.opacity_layer(r.inset(-2.0, -2.0), f * f);
                    }
                    self.nav_bg(r, sel, Hit::Side(i), t);
                    if *glyph == 0 {
                        let p = loc.path().map(|p| p.to_path_buf()).unwrap_or_default();
                        let (a, m) = self.dir_attrs(&p);
                        self.item_icon_a(&p, "", true, a, m, Kind::Folder, Rect::new(r.cx() - 8.0, r.cy() - 8.0, 16.0, 16.0), t);
                    } else {
                        let c = if *color == 0 { t.text2 } else { C::hex(*color) };
                        self.gfx.glyph(*glyph, r, 16.0, c);
                    }
                    if f < 0.99 {
                        self.gfx.pop_layer();
                    }
                }
                if vis(r) {
                    self.hit(r, Hit::Side(i));
                }
                y += 38.0;
            }
        }
        self.gfx.unclip();
        // a soft hint that more is above / below
        if self.rail_scroll > 0.5 {
            self.gfx.glyph(0xE70E, Rect::new(x, area.y - 2.0, 40.0, 10.0), 8.0, t.text3);
        }
        if self.rail_scroll < max - 0.5 {
            self.gfx.glyph(0xE70D, Rect::new(x, area.b() - 8.0, 40.0, 10.0), 8.0, t.text3);
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
        self.gfx.fill(m, t.layer.a(self.panel_alpha()));
        self.gfx.fill(Rect::new(m.x, m.y, 1.0, m.h), t.stroke);
        let cur = self.cur;
        if self.pane().loc == Loc::Settings {
            self.paint_settings(t);
        } else if self.pane().loc == Loc::Cleanup {
            self.paint_cleanup(content_rect(self), t);
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
            // a folder whose size is not known yet: no total (never a wrong "0 bytes")
            if sel.iter().any(|i| i.size.is_none() && i.is_dir) {
                format!("{} selected", sel.len())
            } else {
                let bytes: u64 = sel.iter().filter_map(|i| i.size).sum();
                format!("{} selected · {}", sel.len(), fmt::size(bytes))
            }
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
        // busy after an empty answer: the strip stays as it was (no "Searching..." flash per typed word)
        let (pc, ft, fs, fd, total, busy) = (se.pc, se.ftype, se.fsize, se.fdate, se.total, se.busy && !se.quiet());
        let note = se.note.clone();
        let ai = se.ai.clone();
        let here = pane.loc.title();
        let only_pc = pane.loc == Loc::ThisPc || pane.dir().is_none();
        let searching = pane.searching();
        self.gfx.fill(Rect::new(s.x, s.b() - 1.0, s.w, 1.0), t.divider);
        // scope segmented control (one chip when the place IS This PC)
        let w1 = if only_pc { 0.0 } else { self.gfx.measure(&here, Font::Ui, 13.0, 400).min(160.0) + 28.0 };
        let w2 = self.gfx.measure("This PC", Font::Ui, 13.0, 600) + 28.0;
        let seg = Rect::new(s.x + 16.0, s.y + 8.0, w1 + w2 + if only_pc { 4.0 } else { 6.0 }, 32.0);
        self.gfx.fill_round(seg, R_CTRL, t.layer2);
        self.gfx.stroke_round(seg, R_CTRL, t.stroke, 1.0);
        for (i, (label, w)) in [(here.as_str(), w1), ("This PC", w2)].iter().enumerate() {
            if only_pc && i == 0 {
                continue;
            }
            let x = seg.x + 2.0 + if i == 0 || only_pc { 0.0 } else { w1 + 2.0 };
            let r = Rect::new(x, seg.y + 2.0, *w, 28.0);
            let on = only_pc || (i == 1) == pc;
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
        // system & temp places: Settings > Search only (the hint row in the results still shows them for one search)
        let count = if !searching {
            String::new()
        } else if busy && !note.is_empty() {
            // "Asking AI..."
            note
        } else if busy && total == 0 {
            "Searching...".into()
        } else if !note.is_empty() {
            note
        } else if total == 0 && ai.is_none() && self.ai_on {
            // nothing by name: the empty list says what to do (Sajjad, 2026-11-23: no text here)
            String::new()
        } else if let Some(a) = &ai {
            // what the AI looked for, then the count
            // the count first: a Persian note at the end keeps its own direction
            format!("{} result{}  \u{00b7}  AI: {a}", fmt::thousands(total as u64), if total == 1 { "" } else { "s" })
        } else {
            format!("{} result{}", fmt::thousands(total as u64), if total == 1 { "" } else { "s" })
        };
        // no AI Search button here: Enter in the search box runs it (Sajjad, 2026-11-23)
        let right = s.r() - 16.0;
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
            let bin = self.tabs[cur].panes[p].loc == Loc::RecycleBin;
            if let Some((x, w)) = cp.date {
                heads.push((if bin { "Date deleted" } else { "Date modified" }, x, w, SortCol::Date, false, 1));
            }
            if let Some((x, w)) = cp.ty {
                heads.push((if bin { "Original location" } else { "Type" }, x, w, SortCol::Type, false, 3));
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
                if self.tabs[cur].panes[p].search.as_ref().map(|s| s.busy && !s.quiet()).unwrap_or(false) { String::new() } else { "No results".into() }
            } else {
                if self.tabs[cur].panes[p].loc == Loc::RecycleBin { "The Recycle Bin is empty".into() } else { "This folder is empty".into() }
            };
            self.gfx.text(&msg, Rect::new(list.x, list.y + 40.0, list.w, 24.0), Font::Ui, 13.0, 400, Align::Center, t.text3);
            // a hint of what to do next (empty folder / no results)
            let hint = if error.is_some() || loading {
                ""
            } else if pane_searching {
                let s = self.tabs[cur].panes[p].search.as_ref();
                if s.map(|s| s.busy && !s.quiet()).unwrap_or(true) {
                    ""
                } else if self.ai_on && s.map(|s| s.ai.is_none() && !s.field.text.trim().is_empty()).unwrap_or(false) {
                    // under "No results": plain words work, then the big "Search with AI" button (Sajjad, 2026-11-23)
                    "Describe what you need in your own words, in any language"
                } else if s.map(|s| s.ftype + s.fsize + s.fdate > 0).unwrap_or(false) {
                    "Try fewer filters"
                } else if !s.map(|s| s.pc).unwrap_or(true) && self.tabs[cur].panes[p].loc != Loc::ThisPc {
                    "Try This PC to search everywhere"
                } else {
                    "Check the spelling or try fewer words"
                }
            } else if self.tabs[cur].panes[p].loc == Loc::RecycleBin {
                ""
            } else {
                "Drop files here, or press Ctrl+Shift+N for a new folder"
            };
            if !hint.is_empty() {
                self.gfx.text(hint, Rect::new(list.x, list.y + 64.0, list.w, 20.0), Font::Ui, 12.0, 400, Align::Center, t.text3.a(0.8));
            }
            // nothing by name: describe it in your own words with AI Search (or turn AI on first)
            let no_hits = pane_searching && self.tabs[cur].panes[p].search.as_ref().map(|s| (!s.busy || s.quiet()) && s.ai.is_none() && !s.field.text.trim().is_empty()).unwrap_or(false);
            if no_hits && error.is_none() && !loading {
                // AI on: one big "Search with AI" button (Enter does the same); AI off: offer to turn it on
                if self.ai_on {
                    let label = "Search with AI";
                    let lw = self.gfx.measure(label, Font::Ui, 15.0, 600);
                    let kw = self.gfx.measure("Enter", Font::Ui, 12.0, 400) + 12.0;
                    let bw = 20.0 + 16.0 + 10.0 + lw + 14.0 + kw + 16.0;
                    let br = Rect::new((list.x + (list.w - bw) / 2.0).round(), list.y + 100.0, bw, 44.0);
                    self.round_button(br, "", 0, Btn::Accent, Hit::Ask, R_CTRL, t);
                    let fg = t.on_accent;
                    let mut x = br.x + 20.0;
                    self.gfx.glyph(0xF4A5, Rect::new(x, br.y, 16.0, br.h), 16.0, fg);
                    x += 26.0;
                    self.gfx.text(label, Rect::new(x, br.y, lw + 2.0, br.h), Font::Ui, 15.0, 600, Align::Left, fg);
                    x += lw + 14.0;
                    // the key that does the same
                    let kr = Rect::new(x, br.y + 12.0, kw, 20.0);
                    self.gfx.stroke_round(kr, 4.0, fg.a(0.45), 1.0);
                    self.gfx.text("Enter", kr, Font::Ui, 12.0, 400, Align::Center, fg.a(0.85));
                } else {
                    let (line, label) = ("Turn on AI features to search in your own words, in any language", "Turn on AI Search");
                    self.gfx.text(line, Rect::new(list.x, list.y + 96.0, list.w, 20.0), Font::Ui, 12.0, 400, Align::Center, t.text2);
                    let bw = 14.0 + 13.0 + 8.0 + self.gfx.measure(label, Font::Ui, 13.0, 400) + 14.0;
                    let br = Rect::new(list.x + (list.w - bw) / 2.0, list.y + 124.0, bw, 32.0);
                    self.button(br, label, 0xF4A5, Btn::Accent, Hit::AiOffer, t);
                }
            }
        }
        let menu_rows: Vec<std::path::PathBuf> = self.menu.as_ref().map(|m| m.paths.clone()).unwrap_or_default();
        let sizing = self.tabs[cur].panes[p].dir().map(|d| self.sizes_busy.contains(&d)).unwrap_or(false);
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
            // a folder that files are dragged onto: an accent fill and ring that fade in and out (Explorer)
            let dk = if self.shooting && drop_target.as_ref() == Some(&it.path) { 1.0 } else { self.drop_f.get(h) };
            let target = dk > 0.5;
            if is_sel && !target {
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
            if dk > 0.001 {
                self.gfx.fill_round(r, rad, t.sel.a(t.sel.3 * dk));
                self.gfx.fill_round(r, rad, t.accent.a(0.10 * dk));
                self.gfx.stroke_round(r, rad, t.accent.a(dk), 1.5);
            }
            if focus == Some(i) {
                self.gfx.stroke_round(r, rad, t.accent, 1.0);
            }
            // hidden items are drawn at half strength (as in Explorer; system-only folders, e.g. ones with their own
            // icon, are not)
            let dim = it.attrs & 0x2 != 0 && !pc;
            if dim {
                self.gfx.opacity_layer(r, 0.5);
            }
            if let Some(l) = pl {
                if i < l.n_d {
                    self.paint_drive_tile(r, it, t);
                } else {
                    self.item_icon_a(&it.path, &it.name, true, it.attrs, it.modified, Kind::Folder, Rect::new(r.x + 12.0, r.cy() - 16.0, 32.0, 32.0), t);
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
                    self.gfx.bitmap_round(&b, th, src, R_CTRL);
                    self.gfx.stroke_round(th, R_CTRL, t.stroke, 1.0);
                } else {
                    self.item_icon_a(&it.path, &it.name, it.is_dir, it.attrs, it.modified, it.kind, Rect::new(th.cx() - 32.0, th.cy() - 32.0, 64.0, 64.0), t);
                }
                if let Some((_, f)) = inline.as_ref().filter(|x| x.0 == it.path) {
                    // rename in place: the label becomes an edit box (F2, or a slow second click on the name)
                    let fr = Rect::new(r.x + 2.0, r.y + 110.0, r.w - 4.0, 26.0);
                    self.inline_rect = fr;
                    self.field_box(fr, true, t);
                    self.field_text(FieldId::Inline, f, Rect::new(fr.x + 6.0, fr.y, fr.w - 12.0, fr.h), 12.5, true, t, &[]);
                } else {
                    self.gfx.para(&self.display_name(it), Rect::new(r.x + 6.0, r.y + 108.0, 128.0, 34.0), Font::Ui, 12.0, 400, Align::Center, t.text, 16.0, false, 2);
                }
            } else {
                let rh = r.h;
                self.item_icon_a(&it.path, &it.name, it.is_dir, it.attrs, it.modified, it.kind, Rect::new(r.x + 10.0, r.cy() - isz / 2.0, isz, isz), t);
                let cp = col_pos(self, r, r.x + 10.0 + isz + 10.0);
                let (nx, nw) = cp.name;
                self.name_w.push((h, nw));
                let name = self.display_name(it);
                if let Some((_, f)) = inline.as_ref().filter(|x| x.0 == it.path) {
                    let fr = Rect::new(nx - 6.0, r.y + (rh - 26.0) / 2.0, nw + 6.0, 26.0);
                    self.inline_rect = fr;
                    self.field_box(fr, true, t);
                    self.field_text(FieldId::Inline, f, Rect::new(fr.x + 6.0, fr.y, fr.w - 12.0, fr.h), 13.0, true, t, &[]);
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
                    let ty = if it.recycled.is_some() {
                        it.folder.clone()
                    } else if it.kind == Kind::Drive {
                        "Drive".to_string()
                    } else {
                        ficons::type_label(&it.name, it.is_dir)
                    };
                    self.gfx.text(&ty, Rect::new(x, r.y, w, rh), Font::Ui, fs2, 400, Align::Left, t.text2);
                }
                if let Some((x, w)) = cp.size {
                    // a folder: its size when known; a verified empty folder says so (never "0 bytes" for unknown)
                    let (size, c) = match it.size {
                        Some(0) if it.empty => ("Empty".to_string(), t.text3),
                        Some(s) => (fmt::size(s), t.text2),
                        // a folder still being measured: "…" (never "0 bytes" for unknown)
                        None if it.is_dir && sizing => ("\u{2026}".to_string(), t.text3),
                        None => (String::new(), t.text2),
                    };
                    self.gfx.text(&size, Rect::new(x, r.y, w, rh), Font::Ui, fs2, 400, Align::Right, c);
                }
            }
            if dim {
                self.gfx.pop_layer();
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
        self.drive_slab(r.x + 12.0, r.cy() - 14.0, &root, d.as_ref().map_or("fixed", |d| d.kind), t);
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

    /// Drive image: a slab with a lighter top face and a status light.
    pub(crate) fn drive_slab(&mut self, ix: f32, iy: f32, root: &str, kind: &str, t: &Theme) {
        let body = Rect::new(ix, iy + 10.0, 40.0, 16.0);
        self.gfx.fill_round(Rect::new(ix + 2.0, iy + 4.0, 36.0, 12.0), 3.0, t.page_stroke.mix(t.page, 0.55));
        self.gfx.fill_round(body, 3.5, t.page_stroke.mix(t.text2, 0.25));
        self.gfx.fill_round(body.inset(1.0, 1.0), 3.0, t.page.mix(t.page_stroke, 0.35));
        self.gfx.ellipse(body.x + 7.0, body.cy(), 1.6, t.ok, true, 1.0);
        // each drive type has its own mark: Windows logo on the system drive, a plug on USB drives, a disc on
        // CD / DVD drives, the network glyph on mapped shares
        let sys = std::env::var("SystemDrive").map(|s| root.to_lowercase().starts_with(&s.to_lowercase())).unwrap_or(false);
        if sys {
            let (lx, ly) = (body.r() - 12.0, iy - 1.0);
            for (dx, dy) in [(0.0, 0.0), (5.0, 0.0), (0.0, 5.0), (5.0, 5.0)] {
                self.gfx.fill(Rect::new(lx + dx, ly + dy, 4.0, 4.0), t.accent);
            }
        }
        match kind {
            "network" => self.gfx.glyph(0xE8CE, Rect::new(ix + 22.0, iy - 2.0, 16.0, 16.0), 12.0, t.text2),
            "removable" => {
                self.gfx.fill_round(Rect::new(body.r() - 1.0, body.cy() - 3.5, 6.0, 7.0), 1.5, t.page_stroke);
                self.gfx.fill(Rect::new(body.r() + 1.5, body.cy() - 1.5, 2.0, 1.0), t.page);
                self.gfx.fill(Rect::new(body.r() + 1.5, body.cy() + 0.5, 2.0, 1.0), t.page);
            }
            "cdrom" => {
                self.gfx.ellipse(body.r() - 10.0, iy + 2.0, 7.0, t.page.mix(t.accent, 0.25), true, 1.0);
                self.gfx.ellipse(body.r() - 10.0, iy + 2.0, 7.0, t.page_stroke, false, 1.0);
                self.gfx.ellipse(body.r() - 10.0, iy + 2.0, 1.8, t.page_stroke, true, 1.0);
            }
            _ => {}
        }
    }

    pub fn display_name(&self, it: &Item) -> String {
        // shortcuts never show .lnk / .url (as in Explorer)
        let shortcut = !it.is_dir && it.kind == Kind::Lnk;
        if (shortcut || !self.ui.ext) && !it.is_dir && it.kind != Kind::Drive {
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
        if !matches!(pane.loc, Loc::Settings | Loc::Cleanup) {
            let n = pane.rows().len();
            let items = if pane.searching() { String::new() } else { format!("{} item{}", fmt::thousands(n as u64), if n == 1 { "" } else { "s" }) };
            let sel = pane.selected();
            let selected = if sel.is_empty() {
                String::new()
            } else if sel.iter().any(|i| i.size.is_none() && i.is_dir) {
                // a folder whose size is not known yet: no total (never a wrong "0 bytes")
                format!("{} selected", sel.len())
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
        // Explorer's two view toggles at the bottom right: Details and Grid, the one in use lit; the View menu at the
        // top keeps every view (only where a view applies: not This PC, Settings, Clean up or search results)
        let mut right = r.r() - 8.0;
        let pane = self.pane();
        if !matches!(pane.loc, Loc::Settings | Loc::Cleanup | Loc::ThisPc) && !pane.searching() {
            // two separate view buttons (Sajjad, 2026-11-26: back from the segmented button to this first look)
            let view = self.tab().view;
            for (k, glyph, on) in [(1u8, 0xF0E2u32, view == View::Grid), (0u8, 0xE8FDu32, view == View::Details)] {
                let br = Rect::new(right - 28.0, y + 3.0, 28.0, 24.0);
                let h = Hit::ViewQuick(k);
                let (hk, pk) = (self.hk(h), self.pk(h));
                if on {
                    self.gfx.fill_round(br, R_SMALL, t.navsel);
                    self.gfx.fill_round(Rect::new(br.cx() - 6.0, br.b() - 3.0, 12.0, 2.0), 1.0, t.accent);
                }
                self.gfx.fill_round(br, R_SMALL, t.hover.a(t.hover.3 * hk));
                self.gfx.fill_round(br, R_SMALL, t.press.a(t.press.3 * pk * 0.7));
                self.gfx.glyph(glyph, br, 14.0, if on { t.text } else { t.text2 });
                self.hit(br, h);
                right -= 30.0;
            }
            right -= 6.0;
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
            let xr = Rect::new(right - w, y + 3.0, w, 24.0);
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
        self.gfx.fill(r, t.layer.a(self.panel_alpha()));
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
        self.item_icon_a(&item.path, &item.name, item.is_dir, item.attrs, item.modified, item.kind, Rect::new(r.x + 16.0, y, 40.0, 40.0), t);
        let tx = r.x + 16.0 + 40.0 + 12.0;
        let tw = r.r() - 16.0 - tx;
        let nh = self.gfx.para(&item.name, Rect::new(tx, y, tw, 40.0), Font::Ui, 13.0, 600, Align::Left, t.text, 18.0, false, 3);
        let meta = format!("{}{}", ficons::type_label(&item.name, item.is_dir), match item.size {
            Some(0) if item.empty => " · Empty".to_string(),
            Some(s) => format!(" · {}", fmt::size(s)),
            None => String::new(),
        });
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
        self.gfx.fill_round(r, R_CARD, t.card.a(self.panel_alpha()));
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

    /// Settings > Search: the rules for This PC searches (system & temp, kinds, only-in and never-in folders).
    /// Returns the y below the card.
    fn paint_search_rules(&mut self, x: f32, y: f32, w: f32, t: &Theme) -> f32 {
        let sp = self.sp.clone();
        // the kind chips wrap inside the card
        let (cx0, cmax) = (x + 46.0, x + w - 16.0);
        let mut chips = vec![];
        let (mut cx, mut cy) = (cx0, 0.0f32);
        for (i, name) in crate::model::KINDS.iter().enumerate() {
            let cw = self.gfx.measure(name, Font::Ui, 12.0, 400) + 40.0;
            if cx + cw > cmax && cx > cx0 {
                cx = cx0;
                cy += 36.0;
            }
            chips.push((i, *name, cx, cy, cw));
            cx += cw + 6.0;
        }
        let chips_h = cy + 30.0 + 12.0;
        // the folder tree: drives, then the folders opened below them
        let mut rows: Vec<(std::path::PathBuf, usize)> = vec![];
        fn walk(a: &App, p: &std::path::PathBuf, depth: usize, rows: &mut Vec<(std::path::PathBuf, usize)>) {
            rows.push((p.clone(), depth));
            if a.place_open.contains(p)
                && let Some(kids) = a.place_kids.get(p)
            {
                for k in kids {
                    walk(a, k, depth + 1, rows);
                }
            }
        }
        let roots: Vec<std::path::PathBuf> = self.drives.iter().filter(|d| d.ready && d.kind != "cdrom").map(|d| std::path::PathBuf::from(&d.root)).collect();
        for r in &roots {
            walk(self, r, 0, &mut rows);
        }
        let tree_h = 30.0 + 22.0 + 32.0 * rows.len() as f32 + 10.0;
        let h = 30.0 + 30.0 + chips_h + tree_h + 52.0 + 4.0;
        let r = Rect::new(x, y, w, h);
        self.set_card(r, t);
        self.gfx.text("Search", Rect::new(r.x + 16.0, r.y + 12.0, 120.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        self.gfx.text("For searches of This PC", Rect::new(r.x + 80.0, r.y + 12.0, r.w - 96.0, 20.0), Font::Ui, 12.0, 400, Align::Right, t.text3);
        let mut ry = r.y + 30.0;
        let sub = |a: &mut App, ry: &mut f32, title: &str| {
            // the first section sits right under the card title: no line there
            if *ry > r.y + 31.0 {
                a.gfx.fill(Rect::new(r.x, *ry, r.w, 1.0), t.divider);
            }
            a.gfx.text(title, Rect::new(r.x + 46.0, *ry + 8.0, r.w - 62.0, 20.0), Font::Ui, 12.0, 600, Align::Left, t.text3);
            *ry += 30.0;
        };
        sub(self, &mut ry, "Kinds of results");
        for (i, name, cx, cy, cw) in chips {
            let cr = Rect::new(cx, ry + cy, cw, 30.0);
            let h = Hit::Set(101 + i as u8);
            let on = sp.kind_on(i);
            let hk = self.hk(h);
            if on {
                self.gfx.fill_round(cr, R_CTRL, t.sel);
                self.gfx.glyph(0xE73E, Rect::new(cr.x + 10.0, cr.y, 14.0, cr.h), 11.0, t.accent_text);
            } else {
                self.gfx.fill_round(cr, R_CTRL, t.layer2.a(0.4 + 0.4 * hk));
            }
            self.gfx.stroke_round(cr, R_CTRL, if on { t.accent_text.a(0.35) } else { t.stroke }, 1.0);
            self.gfx.text(name, Rect::new(cr.x + 28.0, cr.y, cw - 32.0, cr.h), Font::Ui, 12.0, 400, Align::Left, if on { t.text } else { t.text2 });
            self.hit(cr, h);
        }
        ry += chips_h;
        // places: tick the drives and folders to search (like Windows' Indexing Options)
        sub(self, &mut ry, "Places to search");
        self.gfx.text("Ticked folders are searched. Open a drive to pick single folders.", Rect::new(r.x + 46.0, ry - 6.0, r.w - 62.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.text3);
        ry += 22.0;
        self.place_rows = rows.iter().map(|r| r.0.clone()).collect();
        for (i, (p, depth)) in rows.iter().enumerate() {
            let rx0 = r.x + 16.0 + 20.0 * *depth as f32;
            let row = Rect::new(r.x + 8.0, ry, r.w - 16.0, 32.0);
            let open = self.place_open.contains(p);
            let has_kids = self.place_kids.get(p).map(|k| !k.is_empty()).unwrap_or(true);
            if has_kids {
                let er = Rect::new(rx0, ry + 4.0, 24.0, 24.0);
                let hk = self.hk(Hit::PlaceExp(i));
                if hk > 0.0 {
                    self.gfx.fill_round(er, R_SMALL, t.hover.a(t.hover.3 * hk));
                }
                self.gfx.glyph(if open { 0xE70D } else { 0xE76C }, er, 9.0, t.text2);
                self.hit(er, Hit::PlaceExp(i));
            }
            // tick box: ticked, part ticked (a folder below differs) or empty
            let cb = Rect::new(rx0 + 28.0, ry + 8.0, 16.0, 16.0);
            let (on, mixed) = (sp.searched(p), sp.mixed(p));
            let hk = self.hk(Hit::PlaceChk(i));
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
            let name = match p.file_name() {
                Some(n) => n.to_string_lossy().into_owned(),
                None => {
                    let root = p.to_string_lossy().trim_end_matches('\\').to_string();
                    let label = self.drives.iter().find(|d| d.root.trim_end_matches('\\').eq_ignore_ascii_case(&root)).map(|d| d.label.clone()).unwrap_or_default();
                    if label.is_empty() { format!("Local Disk ({root})") } else { format!("{label} ({root})") }
                }
            };
            self.gfx.glyph(if *depth == 0 { 0xEDA2 } else { 0xE8B7 }, Rect::new(cb.r() + 10.0, ry, 16.0, 32.0), 14.0, t.text2);
            let tx = cb.r() + 34.0;
            self.gfx.text(&name, Rect::new(tx, ry, row.r() - tx - 8.0, 32.0), Font::Ui, 13.0, 400, Align::Left, if on || mixed { t.text } else { t.text3 });
            // the name ticks too
            self.hit(Rect::new(cb.x - 4.0, ry, row.r() - cb.x + 4.0, 32.0), Hit::PlaceChk(i));
            ry += 32.0;
        }
        // rarely wanted, so it sits last
        ry += 10.0;
        self.set_row(r.x, ry, r.w, 52.0, 0xE770, "Show system and temp files", "Windows, Program Files, AppData, temp, caches", false, t);
        self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry, 40.0, 52.0), sp.junk, Hit::Set(100), t);
        r.b() + 12.0
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
        // Default file manager: the text wraps and the buttons go to a second line when the column is narrow, so
        // nothing ever leaves the card
        let st = fmcore::defaultfm::status();
        let (all, any) = (st.all(), st.parts().iter().any(|p| p.1));
        let sub = if all {
            "Rust File Explorer opens folders, This PC and Win+E"
        } else if any {
            "Partly set: press Set again to add the rest (Win+E)"
        } else {
            "Windows Explorer opens folders and Win+E now"
        };
        let tx = lx + 70.0;
        let tw = (colw - 70.0 - 16.0).max(40.0);
        let lines = (self.gfx.measure(sub, Font::Ui, 12.0, 400) / tw).ceil().clamp(1.0, 3.0);
        let b1l = if tw < 260.0 { "Set as default" } else { "Set as default file manager" };
        let b1w = (self.gfx.measure(b1l, Font::Ui, 13.0, 400) + 26.0).min(tw);
        let b2w = (self.gfx.measure("Restore Explorer", Font::Ui, 13.0, 400) + 26.0).min(tw);
        let two_rows = b1w + 8.0 + b2w > tw;
        let by = 16.0 + 22.0 + 17.0 * lines + 14.0;
        let r = Rect::new(lx, y, colw, by + 32.0 + if two_rows { 40.0 } else { 0.0 } + 16.0);
        self.set_card(r, t);
        let big = Rect::new(r.x + 16.0, r.y + 16.0, 40.0, 40.0);
        self.gfx.fill_round(big, R_CTRL, t.sel);
        self.gfx.glyph(0xEC50, big, 20.0, t.accent_text);
        self.gfx.text("Default file manager", Rect::new(tx, r.y + 16.0, tw, 20.0), Font::Ui, 14.0, 600, Align::Left, t.text);
        self.gfx.para(sub, Rect::new(tx, r.y + 38.0, tw, 17.0 * lines), Font::Ui, 12.0, 400, Align::Left, t.text2, 17.0, false, 3);
        self.button(Rect::new(tx, r.y + by, b1w, 32.0), b1l, 0, if all { Btn::Disabled } else { Btn::Accent }, Hit::Set(1), t);
        let (b2x, b2y) = if two_rows { (tx, r.y + by + 40.0) } else { (tx + b1w + 8.0, r.y + by) };
        self.button(Rect::new(b2x, b2y, b2w, 32.0), "Restore Explorer", 0, if any { Btn::Std } else { Btn::Disabled }, Hit::Set(2), t);
        y = r.b() + 12.0;
        // Personalize: theme, background, transparency, accent, corners
        let lk = self.look.clone();
        let rows_h = 38.0 + 48.0 * 4.0 + 48.0 + 44.0 + if self.accent_field.is_some() { PICK_H } else { 0.0 } + 48.0 + 4.0;
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
        if self.can_mica && lk.backdrop != 3 {
            dd(self, ry, 0xE7B3, "Transparency", crate::look::TRANSPARENCY[lk.transparency as usize % 4], 21, false);
        } else {
            // Solid: nothing to see through
            self.set_row(r.x, ry, r.w, 48.0, 0xE7B3, "Transparency", "Needs Mica or Acrylic", false, t);
            let dr = Rect::new(r.r() - 16.0 - dw, ry + 8.0, dw, 32.0);
            self.gfx.fill_round(dr, R_CTRL, t.field.a(0.5));
            self.gfx.stroke_round(dr, R_CTRL, t.field_border.a(0.6), 1.0);
            self.gfx.text("Off", Rect::new(dr.x + 12.0, dr.y, dr.w - 40.0, dr.h), Font::Ui, 13.0, 400, Align::Left, t.text_dis);
            self.gfx.glyph(0xE70D, Rect::new(dr.r() - 30.0, dr.y, 20.0, dr.h), 10.0, t.text_dis);
        }
        ry += 48.0;
        // the accent colours the whole window; this is how strongly (the chroma of the panel tones)
        dd(self, ry, 0xE2B1, "Colour strength", crate::look::STRENGTHS[lk.strength as usize % 3], 34, false);
        ry += 48.0;
        // accent: Windows, presets, custom
        // the one colour the whole window follows: Windows, Follow desktop (the wallpaper), a preset or custom
        self.set_row(r.x, ry, r.w, 48.0, 0xE790, "Accent color", match (lk.desktop, lk.accent) {
            (true, _) => "Follow desktop (wallpaper colour)",
            (_, None) => "From Windows",
            (_, Some(c)) => crate::look::ACCENTS.iter().find(|a| a.1 == c).map(|a| a.0).unwrap_or("Custom"),
        }, false, t);
        ry += 48.0;
        let mut sx = r.x + 46.0;
        let sel_i: u8 = match (lk.desktop, lk.accent) {
            (true, _) => 52,
            (_, None) => 40,
            (_, Some(c)) => crate::look::ACCENTS.iter().position(|a| a.1 == c).map(|i| 41 + i as u8).unwrap_or(50),
        };
        let wa = self.windows_accent();
        let order: Vec<u8> = [40u8, 52].into_iter().chain(41..=50).collect();
        for id in order {
            if (42..50).contains(&id) && sx + 30.0 * 2.0 > r.r() - 12.0 && id != 50 {
                continue;
            }
            let col = match id {
                40 => wa,
                52 => self.desk.unwrap_or(t.layer2),
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
            if id == 40 {
                // the Windows accent: a small four-pane Windows mark in white (as in Task Manager)
                let ink = C(1.0, 1.0, 1.0, 0.92);
                for (dx, dy) in [(-4.0f32, -4.0f32), (0.5, -4.0), (-4.0, 0.5), (0.5, 0.5)] {
                    self.gfx.fill(Rect::new(sr.cx() + dx, sr.cy() + dy, 3.5, 3.5), ink);
                }
            }
            if id == 52 {
                // a small picture: the colour comes from the wallpaper
                self.gfx.glyph(0xE91B, sr, 11.0, if self.desk.is_some() { C::hex(0xFFFFFF).a(0.9) } else { t.text2 });
            }
            self.hit(sr, h);
            sx += 32.0;
        }
        ry += 44.0;
        if let Some(f) = self.accent_field.clone() {
            // Custom: a colour field (every hue across, full colour at the top to grey at the bottom), picked by
            // click or drag; the hex field and Apply beside it are for an exact colour
            let fw = 120.0;
            let hx = r.r() - 16.0 - fw;
            let pr = Rect::new(r.x + 46.0, ry, (hx - 12.0 - r.x - 46.0).max(60.0), PICK_H - 12.0);
            self.pick_rect = pr;
            self.gfx.spectrum(pr, 6.0);
            self.gfx.stroke_round(pr, 6.0, t.text.a(0.14), 1.0);
            let (h, s) = self.pick;
            let (kx, ky) = (pr.x + pr.w * h / 360.0, pr.y + pr.h * (1.0 - s));
            self.gfx.ellipse(kx, ky, 8.0, C::hex(crate::theme::pick_rgb(h, s)), true, 0.0);
            self.gfx.ellipse(kx, ky, 8.0, C(1.0, 1.0, 1.0, 1.0), false, 2.0);
            self.gfx.ellipse(kx, ky, 9.5, C(0.0, 0.0, 0.0, 0.35), false, 1.0);
            self.hit(pr.inset(-6.0, -6.0), Hit::Set(53));
            let fr = Rect::new(hx, ry, fw, 32.0);
            self.field_box(fr, true, t);
            self.field_text(FieldId::None, &f, Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), 13.0, true, t, &[]);
            self.hit(fr, Hit::Set(50));
            self.button(Rect::new(hx, ry + 40.0, fw, 32.0), "Apply", 0, Btn::Accent, Hit::Set(51), t);
            ry += PICK_H;
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
        let sw_label = if lk.side_collapsed { "Icons only".to_string() } else { crate::look::SIDEBAR_WIDTHS.iter().find(|w| (w.1 - lk.side_w).abs() < 1.0).map(|w| w.0.to_string()).unwrap_or_else(|| format!("{} px", lk.side_w.round())) };
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
        // right column: search and AI (below the left one on a narrow window)
        let left_end = y;
        let mut y = if one { left_end } else { c.y + 16.0 - scroll };
        let ev = match self.set.ev_running {
            None => "Checking...".to_string(),
            Some(true) => format!("Running · {} items", fmt::thousands(self.set.ev_total as u64)),
            Some(false) => "Not running (search walks the folder instead)".into(),
        };
        let ai_switch = crate::ai::switch_on();
        let has_key = crate::ai::has_key();

        let note_h = self.gfx.para_height(crate::ai::SENT_NOTE, colw - 46.0 - 72.0, Font::Ui, 12.0, 16.0, false);
        let ai_h = 30.0 + note_h + 12.0;
        // the key and relay rows show only once AI Search is switched on (they are what it needs)
        let ai_rows = crate::ai::switch_on();
        let rows_h = 38.0 + 52.0 + ai_h + if ai_rows { 96.0 + 122.0 } else { 0.0 } + 4.0;
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
        // AI Search: switch + what it does and exactly what is sent
        self.gfx.fill(Rect::new(r.x, ry, r.w, 1.0), t.divider);
        self.gfx.glyph(0xF4A5, Rect::new(r.x + 16.0, ry + 8.0, 18.0, 20.0), 16.0, t.text2);
        self.gfx.text("AI features", Rect::new(r.x + 46.0, ry + 8.0, r.w - 120.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
        self.gfx.para(crate::ai::SENT_NOTE, Rect::new(r.x + 46.0, ry + 30.0, r.w - 46.0 - 72.0, note_h + 2.0), Font::Ui, 12.0, 400, Align::Left, t.text2, 16.0, false, 0);
        self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry + 4.0, 40.0, 28.0), ai_switch, Hit::Set(9), t);
        ry += ai_h;
        if ai_rows {
        // API key (masked) + Save + Test key + link
        self.gfx.fill(Rect::new(r.x, ry, r.w, 1.0), t.divider);
        self.gfx.glyph(0xE8D7, Rect::new(r.x + 16.0, ry + 8.0, 18.0, 20.0), 16.0, t.text2);
        self.gfx.text("Gemini API key", Rect::new(r.x + 46.0, ry + 8.0, 200.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
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
        self.field_text(FieldId::None, &mf, Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), 13.0, focus, t, &[]);
        if masked.is_empty() && !focus {
            self.gfx.text("Paste your Gemini API key", Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
        }
        self.hit(fr, Hit::Set(15));
        let can_save = !self.key_field.text.trim().is_empty();
        self.button(Rect::new(fr.r() + 8.0, fy, sw_, 32.0), "Save", 0, if can_save { Btn::Accent } else { Btn::Disabled }, Hit::Set(16), t);
        self.button(Rect::new(fr.r() + 8.0 + sw_ + 8.0, fy, tw_, 32.0), "Test key", 0, if has_key || can_save { Btn::Std } else { Btn::Disabled }, Hit::Set(17), t);
        let status = self.set.msg.clone().unwrap_or_else(|| if crate::ai::key_from_env() { "Using the key from GEMINI_API_KEY (environment)".into() } else if has_key { "Key saved (encrypted for this Windows user)".into() } else { "No key saved".into() });
        let sc = if self.set.msg_ok { t.ok } else if self.set.msg.is_some() { t.warn } else { t.text3 };
        self.gfx.text(&status, Rect::new(r.x + 46.0, fy + 36.0, r.w - 62.0, 18.0), Font::Ui, 12.0, 400, Align::Left, sc);
        ry += 96.0;
        // relay (optional): where Google is blocked, the address of a relay that keeps Google's paths, the header
        // that carries its secret, and the secret (masked)
        self.gfx.fill(Rect::new(r.x, ry, r.w, 1.0), t.divider);
        self.gfx.glyph(0xE774, Rect::new(r.x + 16.0, ry + 8.0, 18.0, 20.0), 16.0, t.text2);
        self.gfx.text("Relay (optional)", Rect::new(r.x + 46.0, ry + 8.0, 160.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.text);
        self.gfx.text("Where Google is blocked", Rect::new(r.x + 200.0, ry + 8.0, r.w - 216.0, 20.0), Font::Ui, 12.0, 400, Align::Right, t.text3);
        let (_, _, has_secret) = crate::ai::relay_saved();
        let fx = r.x + 46.0;
        let fw = r.w - 46.0 - 16.0;
        let rf = self.relay_fields.clone();
        let rfocus = self.relay_focus;
        let field = |a: &mut App, fr: Rect, i: usize, shown: &crate::fmt::Field, hint: &str| {
            let focus = rfocus == i as u8 + 1;
            a.field_box(fr, focus, t);
            a.field_text(if i < 2 { FieldId::Relay(i as u8) } else { FieldId::None }, shown, Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), 13.0, focus, t, &[]);
            if shown.text.is_empty() && !focus {
                a.gfx.text(hint, Rect::new(fr.x + 10.0, fr.y, fr.w - 20.0, fr.h), Font::Ui, 13.0, 400, Align::Left, t.text3);
            }
            a.hit(fr, Hit::Set(90 + i as u8));
        };
        field(self, Rect::new(fx, ry + 34.0, fw, 32.0), 0, &rf[0], "https://your-relay.example.com");
        let sw2 = self.gfx.measure("Save", Font::Ui, 13.0, 400) + 26.0;
        let hw = 140.0f32.min(fw * 0.35);
        field(self, Rect::new(fx, ry + 74.0, hw, 32.0), 1, &rf[1], "x-relay-secret");
        let masked: String = if rf[2].text.is_empty() && rfocus != 3 && has_secret { "\u{2022}".repeat(16) } else { "\u{2022}".repeat(rf[2].text.chars().count()) };
        let mut mf = crate::fmt::Field::new(&masked);
        mf.caret = masked.chars().count();
        mf.anchor = mf.caret;
        let sx = fx + hw + 8.0;
        field(self, Rect::new(sx, ry + 74.0, fw - hw - 8.0 - sw2 - 8.0, 32.0), 2, &mf, "Secret");
        self.button(Rect::new(fx + fw - sw2, ry + 74.0, sw2, 32.0), "Save", 0, Btn::Std, Hit::Set(93), t);
        let _ = ry + 122.0;
        }
        y = r.b() + 12.0;
        y = self.paint_search_rules(rx, y, colw, t);
        // Files and folders (right column, below Search)
        let rows: [(u32, &str, &str, bool, u8); 4] = [
            (0xE8B7, "Show folder sizes", "", self.ui.sizes, 4),
            (0xE8AC, "Show file extensions", "", self.ui.ext, 5),
            (0xE7B3, "Show hidden files", "", self.ui.hidden, 6),
            (0xE74D, "Ask before delete", "Delete goes to the Recycle Bin and can be undone", self.ui.ask_delete, 7),
        ];
        let r = Rect::new(rx, y, colw, 38.0 + 4.0 * 48.0 + 52.0 + 48.0 + 4.0);
        self.set_card(r, t);
        self.gfx.text("Files and folders", Rect::new(r.x + 16.0, r.y + 12.0, 200.0, 20.0), Font::Ui, 13.0, 600, Align::Left, t.text);
        let mut ry = r.y + 38.0;
        for (i, (g, title, sub, on, id)) in rows.iter().enumerate() {
            let h = if sub.is_empty() { 48.0 } else { 52.0 };
            self.set_row(r.x, ry, r.w, h, *g, title, sub, i == 0, t);
            self.toggle(Rect::new(r.r() - 16.0 - 40.0, ry, 40.0, h), *on, Hit::Set(*id), t);
            ry += h;
        }
        // where a double-clicked picture opens: the built-in viewer (default) or the Windows default app
        self.set_row(r.x, ry, r.w, 48.0, 0xE91B, "Open pictures in", "", false, t);
        let pw = 214.0f32.min(r.w * 0.5);
        let full = crate::input::PICTURE_OPENERS[if self.ui.viewer { 0 } else { 1 }];
        let label = if self.ui.viewer && self.gfx.measure(full, Font::Ui, 13.0, 400) > pw - 50.0 { "Built-in viewer" } else { full };
        self.dropdown(Rect::new(r.r() - 16.0 - pw, ry + 8.0, pw, 32.0), label, Hit::Set(35), t, Font::Ui);
        ry += 48.0;
        self.paint_cleanup_hidden(Rect::new(r.x, ry, r.w, 48.0), t);
        // the longer column decides how far Settings scrolls
        self.settings_h = r.b().max(left_end - 12.0) + 16.0 + scroll - c.y;
        self.gfx.unclip();
    }

    // ------------------------------------------------------------------ overlays
    /// Address bar suggestions: folders that complete the typed path, under the bar.
    fn paint_sug(&mut self, t: &Theme) {
        if self.sug.is_empty() || self.path_edit.is_none() {
            return;
        }
        let o = self.omni_rect();
        let items = self.sug.clone();
        let r = Rect::new(o.x, o.b() + 4.0, o.w.min(640.0), 8.0 + 32.0 * items.len() as f32);
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        for (i, p) in items.iter().enumerate() {
            let rr = Rect::new(r.x + 4.0, r.y + 4.0 + 32.0 * i as f32, r.w - 8.0, 32.0);
            let h = Hit::Sug(i);
            let on = self.sug_sel == Some(i);
            self.gfx.fill_round(rr, R_CTRL, if on { t.sel } else { t.hover.a(t.hover.3 * self.hk(h)) });
            ficons::draw(&self.gfx, t, Kind::Folder, Rect::new(rr.x + 10.0, rr.cy() - 8.0, 16.0, 16.0));
            self.gfx.text_mid(p, Rect::new(rr.x + 36.0, rr.y, rr.w - 46.0, rr.h), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.hit(rr, h);
        }
    }

    /// One small tooltip under the pointer (names and keys of icon buttons, full text of cut names).
    fn paint_tip(&mut self, t: &Theme) {
        let Some((_, text, x, y)) = self.tip.clone() else { return };
        // never over a menu, dialog, palette or preview
        if self.menu.is_some() || self.drop.is_some() || self.sub.is_some() || self.palette.is_some() || self.confirm.is_some() || self.cleanup.modal() || self.pv.is_some() || self.rename.is_some() {
            self.tip = None;
            return;
        }
        let tw = self.gfx.measure(&text, Font::Ui, 12.0, 400).min(self.w - 40.0) + 20.0;
        let mut r = Rect::new(x - 8.0, y + 22.0, tw, 28.0);
        if r.b() > self.h - 4.0 {
            r.y = y - 8.0 - r.h;
        }
        r.x = r.x.clamp(4.0, (self.w - r.w - 4.0).max(4.0));
        self.card(r, R_SMALL, t, false);
        self.gfx.text_mid(&text, r.inset(10.0, 0.0), Font::Ui, 12.0, 400, Align::Left, t.text);
    }

    fn paint_overlays(&mut self, t: &Theme) {
        self.paint_name_tip(t);
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
        if self.cleanup.modal() {
            self.paint_cleanup_confirm(t);
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

    /// Explorer's tip under a rename box that refused a character: names cannot hold \ / : * ? " < > |
    fn paint_name_tip(&mut self, t: &Theme) {
        let Some(at) = self.inline_tip else { return };
        if self.inline.is_none() || self.inline_rect.w <= 0.0 || (at.elapsed().as_secs_f32() > 4.0 && !self.shooting) {
            return;
        }
        let msg = "A file name can't contain any of the following characters:";
        let chars = "\\ / : * ? \" < > |";
        let w = (self.gfx.measure(msg, Font::Ui, 12.0, 400).max(self.gfx.measure(chars, Font::Ui, 12.0, 600)) + 46.0).min(self.w - 16.0);
        let fr = self.inline_rect;
        let x = fr.x.min(self.w - w - 8.0).max(8.0);
        let y = if fr.b() + 6.0 + 52.0 < self.h - 8.0 { fr.b() + 6.0 } else { fr.y - 6.0 - 52.0 };
        let r = Rect::new(x, y, w, 52.0);
        self.card(r, R_MENU, t, false);
        self.gfx.glyph(0xE7BA, Rect::new(r.x + 10.0, r.y + 6.0, 16.0, 18.0), 12.0, t.danger);
        self.gfx.text(msg, Rect::new(r.x + 30.0, r.y + 5.0, w - 36.0, 20.0), Font::Ui, 12.0, 400, Align::Left, t.text);
        self.gfx.text(chars, Rect::new(r.x + 30.0, r.y + 26.0, w - 36.0, 20.0), Font::Ui, 12.0, 600, Align::Left, t.text);
    }

    fn paint_toast(&mut self, t: &Theme) {
        let Some(to) = self.toast.as_ref() else { return };
        let age = to.at.elapsed().as_secs_f32();
        if !self.shooting && age > 6.0 && !to.hover {
            self.toast = None;
            return;
        }
        let (text, bold, undo, err) = (to.text.clone(), to.bold.clone(), to.undo, to.err);
        let k = prog(to.at, 200.0, self.shooting);
        let m = self.main_rect();
        let mut tw = self.gfx.measure(&text, Font::Ui, 13.0, 400);
        let mut bw = self.gfx.measure(&bold, Font::Ui, 13.0, 600);
        let uw = if undo { self.gfx.measure("Undo", Font::Ui, 13.0, 400) + 12.0 + 14.0 + 8.0 + 12.0 } else { 0.0 };
        let fixed = 16.0 + 16.0 + 12.0 + 12.0 + uw + 8.0 + 28.0 + 6.0;
        // never wider than the window: the text is cut with an ellipsis
        let room = (self.w - m.x.max(8.0) - 24.0 - 16.0 - fixed).max(60.0);
        if tw + bw > room {
            bw = bw.min(room * 0.5);
            tw = (room - bw).max(30.0);
        }
        let w = fixed + tw + bw;
        let x0 = if self.w - w - 16.0 < m.x + 24.0 { 8.0f32.max(self.w - w - 8.0) } else { m.x + 24.0 };
        // it sits at the bottom, or at the top when that would cover the rename box (or its tip)
        let bottom = self.h - 42.0 - 48.0;
        let ib = self.inline_rect;
        let covers = |y: f32| self.inline.is_some() && ib.w > 0.0 && y < ib.b() + 52.0 && y + 48.0 > ib.y - 8.0 && x0 < ib.r() + 8.0 && x0 + w > ib.x - 8.0;
        let y = if covers(bottom) { TOP + 12.0 - (1.0 - k) * 12.0 } else { bottom + (1.0 - k) * 12.0 };
        let r = Rect::new(x0, y, w, 48.0);
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), k.clamp(0.05, 1.0));
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        if err {
            self.gfx.glyph(0xEA39, Rect::new(r.x + 16.0, r.y, 16.0, r.h), 14.0, t.danger);
        } else {
            self.gfx.glyph(0xE73E, Rect::new(r.x + 16.0, r.y, 16.0, r.h), 14.0, t.ok);
        }
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
        let mut h = 8.0 + if m.bg { 0.0 } else { QUICK_H + 1.0 };
        for it in &m.items {
            h += 32.0 + if it.sep { 9.0 } else { 0.0 };
        }
        let w = 292.0;
        let x = m.x.min(self.w - w - 8.0).max(4.0);
        // below the pointer when it fits, else above it; when neither fits, as low as the window allows, so the
        // tabs and the address bar stay visible whenever there is room
        let y = if m.y + h <= self.h - 8.0 {
            m.y
        } else if m.y - h >= TOP {
            m.y - h
        } else {
            (self.h - 8.0 - h).max(TITLE_H)
        };
        Some(Rect::new(x, y, w, h))
    }

    fn menu_rows(&mut self, r: Rect, items: &[MItem], y0: f32, hit_of: impl Fn(usize) -> Hit, t: &Theme) {
        let mut y = y0;
        let mut pending_tip: Option<(Rect, String, f32)> = None;
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
                let focused = h == Hit::Drop(i) && self.drop.as_ref().is_some_and(|d| d.kind == DropKind::CleanupScope)
                    && self.cleanup.scope_focus == Some(i);
                let hk = if open_sub { 1.0 } else { self.hk(h) };
                self.gfx.fill_round(ir, R_CTRL, t.hover.a(t.hover.3 * hk));
                self.gfx.fill_round(ir, R_CTRL, t.press.a(t.press.3 * self.pk(h) * 0.8));
                if focused {
                    self.gfx.stroke_round(ir.inset(1.0, 1.0), R_CTRL, t.sel_stroke, 1.0);
                }
            }
            let fg = if it.enabled { t.text } else { t.text_dis };
            let ib = Rect::new(ir.x + 10.0, ir.cy() - 8.0, 16.0, 16.0);
            if let Some(c) = it.swatch {
                let col = if c == u32::MAX { self.windows_accent() } else { C::hex(c) };
                self.gfx.ellipse(ib.cx(), ib.cy(), 7.5, col, true, 1.0);
                self.gfx.ellipse(ib.cx(), ib.cy(), 7.5, t.text.a(0.18), false, 1.0);
            }
            // every row of the context menu and its submenus has an icon: the shell's own, else ours, else a glyph
            // that fits the words (print, star, rotate, history, Bluetooth, documents ...), else a neutral one
            let ctx_row = matches!(h, Hit::MenuItem(_) | Hit::SubItem(_));
            let mut drawn = it.swatch.is_some();
            if drawn {
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
                    drawn = true;
                }
            }
            if !drawn && (it.glyph != 0 || ctx_row) {
                let g = if it.glyph != 0 { it.glyph } else { menu_glyph(it.shell.as_deref().unwrap_or(""), &it.label) };
                self.gfx.glyph(g, Rect::new(ir.x + 10.0, ir.y, 16.0, ir.h), 16.0, if it.enabled { t.text2 } else { t.text_dis });
            }
            let tx = if any_icon || ctx_row { ir.x + 38.0 } else { ir.x + 12.0 };
            let kw = if it.keys.is_empty() { 0.0 } else { self.gfx.measure(&it.keys, Font::Ui, 12.0, 400) + 4.0 };
            self.gfx.text(&it.label, Rect::new(tx, ir.y, ir.r() - tx - kw - 30.0, ir.h), Font::Ui, 13.0, if it.bold { 600 } else { 400 }, Align::Left, fg);
            let app_row = it.pin.is_some();
            if !it.keys.is_empty() {
                // an "Open with" default: check mark + "Default" in the accent colour
                let c = if app_row && it.checked { t.accent_text } else { t.text3 };
                self.gfx.text(&it.keys, Rect::new(ir.r() - 10.0 - kw, ir.y, kw, ir.h), Font::Ui, 12.0, 400, Align::Right, c);
                if app_row && it.checked {
                    self.gfx.glyph(0xE73E, Rect::new(ir.r() - 10.0 - kw - 18.0, ir.y, 14.0, ir.h), 11.0, t.accent_text);
                }
            }
            if it.sub {
                self.gfx.glyph(0xE76C, Rect::new(ir.r() - 24.0, ir.y, 14.0, ir.h), 10.0, t.text2);
            }
            if it.checked && !app_row {
                self.gfx.glyph(0xE73E, Rect::new(ir.r() - 26.0, ir.y, 16.0, ir.h), 12.0, t.accent_text);
            }
            self.hit(ir, h);
            // the pin action of an app row: shown on hover, one click = always open this type with this app
            if app_row && !it.checked
                && let Hit::SubItem(si) = h
            {
                let ph = Hit::SubPin(si);
                let k = self.hk(h).max(self.hk(ph));
                if k > 0.01 {
                    let pr = Rect::new(ir.r() - 34.0, ir.y + 4.0, 28.0, 24.0);
                    self.gfx.fill_round(pr, R_SMALL, t.hover.a(t.hover.3 * 1.6 * self.hk(ph)));
                    self.gfx.glyph(0xE718, pr, 13.0, t.text2.a(k));
                    self.hit(pr, ph);
                    if self.hk(ph) > 0.01 {
                        pending_tip = Some((pr, it.pin.clone().unwrap_or_default(), self.hk(ph)));
                    }
                }
            }
            y += 32.0;
        }
        if let Some((pr, tip, k)) = pending_tip {
            // tooltip under the pin
            let tw = self.gfx.measure(&tip, Font::Ui, 12.0, 400) + 20.0;
            let tr = Rect::new((pr.r() - tw).max(4.0), pr.b() + 6.0, tw, 28.0);
            self.gfx.opacity_layer(tr.inset(-20.0, -20.0), k.clamp(0.0, 1.0));
            self.card(tr, R_SMALL, t, false);
            self.gfx.text(&tip, tr.inset(10.0, 0.0), Font::Ui, 12.0, 400, Align::Left, t.text);
            self.gfx.pop_layer();
        }
    }
    /// The Windows accent (for the "Windows accent" swatch).
    pub fn windows_accent(&self) -> C {
        crate::theme::make(self.t.dark, None, self.tint(), self.look.strength).accent
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
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), op.max(0.02));
        self.gfx.push_scale(ax, ay, sc, 0.0, dy);
        self.card(r, R_MENU, t, false);
        self.hit(r, Hit::Block);
        let mut y = r.y + 4.0;
        if !bg {
            // Cut, Copy, Rename, Delete: four equal cells with the label under the icon and thin dividers between
            // them; the whole cell is the button (hover and press light all of it)
            let qs = [(0u8, 0xE8C6u32, "Cut"), (1, 0xE8C8, "Copy"), (2, 0xE8AC, "Rename"), (3, 0xE74D, "Delete")];
            let (x0, w) = (r.x + 4.0, r.w - 8.0);
            let cw = w / 4.0;
            let ch = QUICK_H - 6.0;
            for (i, (id, g, label)) in qs.iter().enumerate() {
                let cell = Rect::new(x0 + cw * i as f32, y, cw, ch);
                let h = Hit::MenuQuick(*id);
                let (hk, pk) = (self.hk(h), self.pk(h));
                let inner = cell.inset(2.0, 0.0);
                self.gfx.fill_round(inner, R_CTRL, t.hover.a(t.hover.3 * hk));
                self.gfx.fill_round(inner, R_CTRL, t.press.a(t.press.3 * pk * 0.8));
                if i > 0 {
                    // a divider fades where a neighbouring cell is lit
                    let q = 1.0 - hk.max(self.hk(Hit::MenuQuick(qs[i - 1].0)));
                    self.gfx.fill(Rect::new(cell.x - 0.5, cell.y + 10.0, 1.0, ch - 20.0), t.divider.a(t.divider.3 * q));
                }
                self.gfx.glyph(*g, Rect::new(cell.x, cell.y + 9.0, cell.w, 18.0), 16.0, t.text);
                self.gfx.text(label, Rect::new(cell.x + 2.0, cell.y + 30.0, cell.w - 4.0, 16.0), Font::Ui, 11.5, 400, Align::Center, t.text2);
                self.hit(cell, h);
            }
            y += QUICK_H;
            self.gfx.fill(Rect::new(r.x, y - 1.0, r.w, 1.0), t.divider);
            y += 4.0;
        }
        self.menu_rows(r, &items, y, Hit::MenuItem, t);
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
        self.field_text(FieldId::Palette, &field, Rect::new(ir.x + 44.0, ir.y, ir.w - 110.0, ir.h), 15.0, true, t, &[]);
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
        // AI is only for AI Search now (Sajjad, 2026-11-20): no "Suggest names" here
        if false {
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
                self.field_text(FieldId::Ren(0), &pattern, Rect::new(pr.x + 10.0, pr.y, pr.w - 80.0, pr.h), 13.0, focus == 0, t, &spans);
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
                self.field_text(FieldId::Ren(1), &start, Rect::new(sr.x + 10.0, sr.y, sr.w - 20.0, sr.h), 13.0, focus == 1, t, &[]);
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
                self.field_text(FieldId::Ren(2), &find, Rect::new(a.x + 10.0, a.y, a.w - 20.0, a.h), 13.0, focus == 0, t, &[]);
                self.hit(a, Hit::Dlg(20));
                let b = Rect::new(fx + hw + 12.0, y, hw, 34.0);
                self.field_box(b, focus == 1, t);
                self.field_text(FieldId::Ren(3), &repl, Rect::new(b.x + 10.0, b.y, b.w - 20.0, b.h), 13.0, focus == 1, t, &[]);
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
        self.item_icon(&path, &name, is_dir, ficons::kind_of(&name, is_dir), Rect::new(hr.x + 18.0, hr.cy() - 12.0, 24.0, 24.0), t);
        let mut bx = hr.r() - 10.0 - 36.0;
        self.icon_btn(Rect::new(bx, hr.y + 10.0, 36.0, 36.0), 0xE711, 12.0, t.text, Hit::Pv(0), R_CTRL, t);
        bx -= 12.0;
        self.gfx.fill(Rect::new(bx, hr.y + 18.0, 1.0, 20.0), t.divider);
        bx -= 12.0;
        // a program is not "opened": the button says what it does
        let program = matches!(fmcore::util::ext_of(&name).as_str(), "exe" | "msi" | "bat" | "cmd" | "com" | "scr" | "ps1" | "vbs");
        let open_label = if program { "Run" } else { "Open" };
        let ow = self.gfx.measure(open_label, Font::Ui, 13.0, 400) + 30.0;
        bx -= ow;
        self.button(Rect::new(bx, hr.y + 12.0, ow, 32.0), open_label, 0, Btn::Accent, Hit::Pv(1), t);
        bx -= 12.0;
        // a folder has no apps to open it with
        if !is_dir {
            let ww = self.gfx.measure("Open with", Font::Ui, 13.0, 400) + 12.0 + 9.0 + 6.0 + 12.0;
            bx -= ww;
            let owr = Rect::new(bx, hr.y + 12.0, ww, 32.0);
            self.button(owr, "", 0, Btn::Std, Hit::Pv(2), t);
            self.gfx.text("Open with", Rect::new(owr.x + 12.0, owr.y, ww - 24.0, owr.h), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.gfx.glyph(0xE70D, Rect::new(owr.r() - 21.0, owr.y, 9.0, owr.h), 9.0, t.text2);
        }
        if data.as_ref().map(|d| d.kind == PvKind::Text).unwrap_or(false) {
            let cw = 12.0 + 14.0 + 8.0 + self.gfx.measure("Copy", Font::Ui, 13.0, 400) + 12.0;
            bx -= 12.0 + cw;
            self.button(Rect::new(bx, hr.y + 12.0, cw, 32.0), "Copy", 0xE8C8, Btn::Ghost, Hit::Pv(3), t);
        }
        // the same name as in the list (shortcuts without .lnk / .url, extensions as set)
        let shown = {
            let it = Item { name: name.clone(), path: path.clone(), is_dir, size: None, modified: 0, attrs: 0, kind: ficons::kind_of(&name, is_dir), folder: String::new(), hl: vec![], recycled: None, empty: false };
            self.display_name(&it)
        };
        self.gfx.text(&shown, Rect::new(hr.x + 54.0, hr.y + 8.0, bx - hr.x - 66.0, 20.0), Font::Ui, 14.0, 600, Align::Left, t.text);
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
                    self.item_icon(&path, &name, is_dir, ficons::kind_of(&name, is_dir), Rect::new(body.cx() - 48.0, body.cy() - 48.0, 96.0, 96.0), t);
                }
                if d.kind == PvKind::Other {
                    let hint = if is_dir {
                        "A folder. Open shows what is inside it."
                    } else if matches!(fmcore::util::ext_of(&name).as_str(), "exe" | "msi" | "bat" | "cmd" | "com" | "scr" | "ps1" | "vbs") {
                        "A program: no preview. Run starts it."
                    } else {
                        "No preview for this type. Open opens it in its app."
                    };
                    self.gfx.text(hint, Rect::new(body.x, body.cy() + 96.0, body.w, 20.0), Font::Ui, 12.0, 400, Align::Center, t.text3);
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
        // the source's own drag image is drawn by the shell, with "Move to ..." under it: no second card
        if eff == 0 || d.image {
            return;
        }
        let target = d.dest.clone().or_else(|| self.pane().dir()).map(|p| folder_title(&p)).unwrap_or_default();
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

/// Outline of the active tab: rounded top corners, straight sides down to the toolbar and small inverted curves
/// ("feet") where it meets it, so the tab and the toolbar read as one surface (Windows 11 Explorer). `closed` fills
/// it; open, it is the outline without a bottom edge.
fn tab_shape(r: Rect, rad: f32, foot: f32, closed: bool) -> Vec<crate::gfx::P> {
    use crate::gfx::P;
    // cubic bezier handle of a quarter circle
    const K: f32 = 0.5523;
    let (x0, x1, top, bot) = (r.x, r.r(), r.y, r.b());
    let mut v = vec![
        P::M(x0 - foot, bot),
        P::B(x0 - foot + K * foot, bot, x0, bot - foot + K * foot, x0, bot - foot),
        P::L(x0, top + rad),
        P::B(x0, top + rad - K * rad, x0 + rad - K * rad, top, x0 + rad, top),
        P::L(x1 - rad, top),
        P::B(x1 - rad + K * rad, top, x1, top + rad - K * rad, x1, top + rad),
        P::L(x1, bot - foot),
        P::B(x1, bot - foot + K * foot, x1 + foot - K * foot, bot, x1 + foot, bot),
    ];
    if closed {
        v.push(P::Z);
    }
    v
}

/// Height of the context menu's Cut / Copy / Rename / Delete cells (with the divider under them).
const QUICK_H: f32 = 62.0;

/// A glyph for a menu row that brought no icon of its own (shell verbs such as Print, Add to Favorites, Rotate,
/// Restore previous versions, Send to > Bluetooth / Documents, and anything unknown), in the style of the app's
/// other menu glyphs (Segoe Fluent Icons). Looks at the verb and the words of the row.
pub fn menu_glyph(verb: &str, label: &str) -> u32 {
    let v = verb.to_lowercase();
    let t = label.to_lowercase();
    // a group row "Send to > Documents": the entry's own words decide
    let last = t.rsplit('\u{203A}').next().unwrap_or(&t).trim().to_string();
    let has = |k: &str| v.contains(k) || last.contains(k);
    if last.starts_with("loading") {
        0xE895 // sync
    } else if last.starts_with("nothing") {
        0xE946 // info
    } else if has("print") || has("fax") {
        0xE749 // print
    } else if has("favorite") || has("favourite") || has("star") {
        0xE734 // star
    } else if has("rotate") && (has("left") || has("counter") || has("ccw")) {
        0xE7A7 // turn left
    } else if has("rotate") {
        0xE7A6 // turn right
    } else if has("previous version") || has("history") || has("restore") {
        0xE81C // history (clock)
    } else if has("bluetooth") {
        0xE702 // bluetooth
    } else if has("document") {
        0xE8A5 // document
    } else if has("background") || has("wallpaper") || has("lock screen") {
        0xE91B // picture
    } else if has("desktop") {
        0xE7F4 // screen
    } else if has("zip") || has("compress") || has("archive") || has("extract") || has("7-zip") || has("rar") {
        0xE7B8 // package
    } else if has("mail") {
        0xE715 // mail
    } else if has("scan") || has("defender") || has("virus") || has("runas") || has("administrator") {
        0xEA18 // shield
    } else if has("share") || has("give access") {
        0xE72D // share
    } else if has("cast") || has("play to") {
        0xE7F4 // screen
    } else if has("edit") {
        0xE70F // pen
    } else if has("unpin") {
        0xE77A // unpin
    } else if has("pin") {
        0xE718 // pin
    } else if has("troubleshoot") || has("compatib") {
        0xE90F // repair
    } else if has("play") || has("playlist") || has("media") {
        0xE768 // play
    } else if has("encrypt") || has("lock") || has("password") || has("protect") {
        0xE72E // lock
    } else if has("usb") || has("removable") {
        0xE88E // usb drive
    } else if last.contains(":)") || has("drive") {
        0xEDA2 // drive
    } else if has("folder") || has("library") {
        0xE8B7 // folder
    } else if has("copy") || has("move") {
        0xE8C8 // copy
    } else if has("new") || has("create") {
        0xE710 // add
    } else if has("open") || has("run") || has("launch") || has("start") {
        0xE8A7 // open in window
    } else if has("properties") || has("info") || has("details") {
        0xE946 // info
    } else if has("send") {
        0xE724 // send
    } else {
        0xEA86 // puzzle: a verb some extension added
    }
}
