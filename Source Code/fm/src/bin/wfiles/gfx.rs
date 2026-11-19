//! Thin Direct2D / DirectWrite layer (copied from the Task Manager's gui/gfx.rs and extended): one render target,
//! one brush, cached text formats, rich text runs, caret hit tests, simple path geometry, and Vazirmatn loaded
//! from memory (bundled in the exe) for the Persian AI answer card.
use fmcore::util::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{Interface, PCWSTR};
use windows_numerics::Vector2;

static VAZIR: &[u8] = include_bytes!("../../../assets/Vazirmatn-Regular.ttf");

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct C(pub f32, pub f32, pub f32, pub f32);

impl C {
    pub const fn hex(v: u32) -> C {
        C(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0)
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> C {
        C(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
    }
    pub const fn a(self, a: f32) -> C {
        C(self.0, self.1, self.2, a)
    }
    pub fn mix(self, o: C, t: f32) -> C {
        C(self.0 + (o.0 - self.0) * t, self.1 + (o.1 - self.1) * t, self.2 + (o.2 - self.2) * t, self.3 + (o.3 - self.3) * t)
    }
    fn d2(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.0, g: self.1, b: self.2, a: self.3 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn r(&self) -> f32 {
        self.x + self.w
    }
    pub fn b(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
    pub fn has(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.r() && y >= self.y && y < self.b()
    }
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.r() && o.x < self.r() && self.y < o.b() && o.y < self.b()
    }
    fn d2(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.x + self.w, bottom: self.y + self.h }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Font {
    Ui,
    Display,
    Icon,
    Mono,
    Fa,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FmtKey(Font, u32, u32, Align, bool, bool);

/// One coloured / weighted range of a text run (UTF-16 offsets).
#[derive(Clone, Copy)]
pub struct Span {
    pub start: u32,
    pub len: u32,
    pub color: C,
    pub weight: u32,
    pub italic: bool,
}

/// Path drawing commands in DIPs.
pub enum P {
    M(f32, f32),
    L(f32, f32),
    /// cubic bezier: c1, c2, end
    B(f32, f32, f32, f32, f32, f32),
    /// quadratic: control, end
    Q(f32, f32, f32, f32),
    Z,
}

pub struct Gfx {
    pub d2: ID2D1Factory,
    pub dw: IDWriteFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    base: Option<ID2D1RenderTarget>,
    over: Option<ID2D1RenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    formats: RefCell<HashMap<FmtKey, IDWriteTextFormat>>,
    hwnd: HWND,
    pub software: bool,
    pub dpi: f32,
    /// transparent clear (Mica shows through) when true
    pub alpha_target: bool,
    fa: Option<IDWriteFontCollection>,
    icon_family: String,
}

impl Gfx {
    pub fn new(hwnd: HWND, software: bool, alpha_target: bool) -> windows::core::Result<Gfx> {
        unsafe {
            let d2: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let has = |name: &str| -> bool {
                let mut coll = None;
                if dw.GetSystemFontCollection(&mut coll, false).is_err() {
                    return false;
                }
                let Some(c) = coll else { return false };
                let w = wide(name);
                let mut idx = 0u32;
                let mut exists = windows::core::BOOL(0);
                c.FindFamilyName(PCWSTR(w.as_ptr()), &mut idx, &mut exists).is_ok() && exists.as_bool()
            };
            let icon_family = if has("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" }.to_string();
            Ok(Gfx { d2, dw, rt: None, base: None, over: None, brush: None, formats: RefCell::new(HashMap::new()), hwnd, software, dpi: 96.0, alpha_target, fa: None, icon_family })
        }
    }

    /// Vazirmatn from the bytes inside the exe (no install). Loaded on first use only.
    fn fa_collection(&mut self) -> Option<IDWriteFontCollection> {
        if self.fa.is_some() {
            return self.fa.clone();
        }
        unsafe {
            let f5: IDWriteFactory5 = self.dw.cast().ok()?;
            let loader = f5.CreateInMemoryFontFileLoader().ok()?;
            f5.RegisterFontFileLoader(&loader).ok()?;
            let file = loader.CreateInMemoryFontFileReference(&f5, VAZIR.as_ptr() as *const _, VAZIR.len() as u32, None).ok()?;
            let b: IDWriteFontSetBuilder1 = f5.CreateFontSetBuilder().ok()?.cast().ok()?;
            b.AddFontFile(&file).ok()?;
            let set = b.CreateFontSet().ok()?;
            let coll = f5.CreateFontCollectionFromFontSet(&set).ok()?;
            self.fa = coll.cast::<IDWriteFontCollection>().ok();
            self.fa.clone()
        }
    }

    pub fn scale(&self) -> f32 {
        self.dpi / 96.0
    }

    pub fn ensure(&mut self, px_w: u32, px_h: u32) -> bool {
        if self.over.is_some() {
            return true;
        }
        unsafe {
            if let Some(rt) = &self.rt {
                let s = rt.GetPixelSize();
                if s.width != px_w || s.height != px_h {
                    let _ = rt.Resize(&D2D_SIZE_U { width: px_w, height: px_h });
                }
                rt.SetDpi(self.dpi, self.dpi);
                return true;
            }
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: if self.software { D2D1_RENDER_TARGET_TYPE_SOFTWARE } else { D2D1_RENDER_TARGET_TYPE_DEFAULT },
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: if self.alpha_target { D2D1_ALPHA_MODE_PREMULTIPLIED } else { D2D1_ALPHA_MODE_IGNORE } },
                dpiX: self.dpi,
                dpiY: self.dpi,
                ..Default::default()
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd: self.hwnd, pixelSize: D2D_SIZE_U { width: px_w, height: px_h }, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
            let Ok(rt) = self.d2.CreateHwndRenderTarget(&props, &hp) else { return false };
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.brush = rt.CreateSolidColorBrush(&C(1.0, 1.0, 1.0, 1.0).d2(), None).ok();
            self.base = rt.cast::<ID2D1RenderTarget>().ok();
            self.rt = Some(rt);
            true
        }
    }

    pub fn begin(&self) {
        unsafe { self.rt().BeginDraw() };
    }

    pub fn end(&mut self) -> bool {
        let r = unsafe { self.rt().EndDraw(None, None) };
        if r.is_err() && self.over.is_none() {
            self.rt = None;
            self.base = None;
            self.brush = None;
            return false;
        }
        true
    }

    fn rt(&self) -> &ID2D1RenderTarget {
        self.over.as_ref().or(self.base.as_ref()).unwrap()
    }

    pub fn offscreen(&self) -> bool {
        self.over.is_some()
    }

    /// Draw the next frames into a memory DC instead of the window (for `wfm gui shot`).
    pub fn begin_offscreen(&mut self, dc: windows::Win32::Graphics::Gdi::HDC, px_w: u32, px_h: u32) -> bool {
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: self.dpi,
                dpiY: self.dpi,
                ..Default::default()
            };
            let Ok(dcrt) = self.d2.CreateDCRenderTarget(&props) else { return false };
            let rc = windows::Win32::Foundation::RECT { left: 0, top: 0, right: px_w as i32, bottom: px_h as i32 };
            if dcrt.BindDC(dc, &rc).is_err() {
                return false;
            }
            dcrt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.brush = dcrt.CreateSolidColorBrush(&C(1.0, 1.0, 1.0, 1.0).d2(), None).ok();
            self.over = dcrt.cast::<ID2D1RenderTarget>().ok();
            self.over.is_some()
        }
    }

    pub fn end_offscreen(&mut self) {
        self.over = None;
        self.brush = self.base.as_ref().and_then(|b| unsafe { b.CreateSolidColorBrush(&C(1.0, 1.0, 1.0, 1.0).d2(), None).ok() });
    }

    fn b(&self, c: C) -> &ID2D1SolidColorBrush {
        let b = self.brush.as_ref().unwrap();
        unsafe { b.SetColor(&c.d2()) };
        b
    }

    pub fn clear(&self, c: C) {
        unsafe { self.rt().Clear(Some(&c.d2())) };
    }

    pub fn fill(&self, r: Rect, c: C) {
        if c.3 <= 0.0 {
            return;
        }
        unsafe { self.rt().FillRectangle(&r.d2(), self.b(c)) };
    }

    pub fn fill_round(&self, r: Rect, rad: f32, c: C) {
        if c.3 <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        unsafe { self.rt().FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2(), radiusX: rad, radiusY: rad }, self.b(c)) };
    }

    pub fn stroke_round(&self, r: Rect, rad: f32, c: C, w: f32) {
        if c.3 <= 0.0 {
            return;
        }
        let r = r.inset(w / 2.0, w / 2.0);
        unsafe { self.rt().DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r.d2(), radiusX: rad, radiusY: rad }, self.b(c), w, None) };
    }

    /// Soft drop shadow under a rounded card (a few widening translucent layers).
    pub fn shadow(&self, r: Rect, rad: f32, dy: f32, spread: f32, alpha: f32) {
        // concentric layers, fading out: a soft edge without a blur effect
        let n = 12;
        for i in (0..n).rev() {
            let t = (i + 1) as f32 / n as f32;
            let e = spread * t * 0.6;
            let rr = Rect::new(r.x - e, r.y - e * 0.7 + dy * 0.6, r.w + e * 2.0, r.h + e * 2.0);
            self.fill_round(rr, rad + e, C(0.06, 0.09, 0.16, alpha * 0.16 * (1.0 - t) * (1.0 - t)));
        }
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: C, w: f32) {
        unsafe { self.rt().DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.b(c), w, None) };
    }

    pub fn ellipse(&self, cx: f32, cy: f32, r: f32, c: C, fill: bool, w: f32) {
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: r, radiusY: r };
        unsafe {
            if fill {
                self.rt().FillEllipse(&e, self.b(c));
            } else {
                self.rt().DrawEllipse(&e, self.b(c), w, None);
            }
        }
    }

    fn geometry(&self, cmds: &[P], ox: f32, oy: f32, k: f32) -> Option<ID2D1PathGeometry> {
        unsafe {
            let path = self.d2.CreatePathGeometry().ok()?;
            let sink = path.Open().ok()?;
            let v = |x: f32, y: f32| Vector2 { X: ox + x * k, Y: oy + y * k };
            let mut open = false;
            for c in cmds {
                match *c {
                    P::M(x, y) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        }
                        sink.BeginFigure(v(x, y), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    P::L(x, y) => sink.AddLine(v(x, y)),
                    P::B(a, b, c2, d, e, f) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v(a, b), point2: v(c2, d), point3: v(e, f) }),
                    P::Q(a, b, e, f) => sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: v(a, b), point2: v(e, f) }),
                    P::Z => {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close().ok()?;
            Some(path)
        }
    }

    /// Fill a path given in a local unit box, placed at (ox, oy) and scaled by k.
    pub fn fill_path(&self, cmds: &[P], ox: f32, oy: f32, k: f32, c: C) {
        if let Some(g) = self.geometry(cmds, ox, oy, k) {
            unsafe { self.rt().FillGeometry(&g, self.b(c), None) };
        }
    }

    pub fn stroke_path(&self, cmds: &[P], ox: f32, oy: f32, k: f32, c: C, w: f32) {
        if let Some(g) = self.geometry(cmds, ox, oy, k) {
            unsafe {
                let style = self
                    .d2
                    .CreateStrokeStyle(
                        &D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, miterLimit: 4.0, dashStyle: D2D1_DASH_STYLE_SOLID, dashOffset: 0.0 },
                        None,
                    )
                    .ok();
                self.rt().DrawGeometry(&g, self.b(c), w, style.as_ref());
            }
        }
    }

    pub fn clip(&self, r: Rect) {
        unsafe { self.rt().PushAxisAlignedClip(&r.d2(), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    pub fn unclip(&self) {
        unsafe { self.rt().PopAxisAlignedClip() };
    }

    pub fn opacity_layer(&self, r: Rect, opacity: f32) {
        unsafe {
            let params = D2D1_LAYER_PARAMETERS {
                contentBounds: r.d2(),
                geometricMask: std::mem::ManuallyDrop::new(None),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: windows_numerics::Matrix3x2::identity(),
                opacity,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.rt().PushLayer(&params, None);
        }
    }

    pub fn pop_layer(&self) {
        unsafe { self.rt().PopLayer() };
    }

    pub fn translate(&self, dx: f32, dy: f32) {
        unsafe { self.rt().SetTransform(&windows_numerics::Matrix3x2::translation(dx, dy)) };
    }

    pub fn reset_transform(&self) {
        unsafe { self.rt().SetTransform(&windows_numerics::Matrix3x2::identity()) };
    }

    fn format(&mut self, font: Font, size: f32, weight: u32, align: Align, wrap: bool, rtl: bool) -> IDWriteTextFormat {
        let key = FmtKey(font, (size * 10.0) as u32, weight, align, wrap, rtl);
        if let Some(f) = self.formats.borrow().get(&key) {
            return f.clone();
        }
        let coll = if font == Font::Fa { self.fa_collection() } else { None };
        let family = match font {
            Font::Ui => "Segoe UI Variable Text",
            Font::Display => "Segoe UI Variable Display",
            Font::Icon => &self.icon_family,
            Font::Mono => "Cascadia Mono",
            Font::Fa => {
                if coll.is_some() {
                    "Vazirmatn"
                } else {
                    "Segoe UI"
                }
            }
        };
        let fam = wide(family);
        let loc = wide(if font == Font::Fa { "fa-IR" } else { "en-us" });
        unsafe {
            let f = self.dw.CreateTextFormat(PCWSTR(fam.as_ptr()), coll.as_ref(), DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, PCWSTR(loc.as_ptr())).unwrap();
            if rtl {
                let _ = f.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT);
            }
            let _ = f.SetTextAlignment(match (align, rtl) {
                (Align::Left, false) | (Align::Right, true) => DWRITE_TEXT_ALIGNMENT_LEADING,
                (Align::Center, _) => DWRITE_TEXT_ALIGNMENT_CENTER,
                _ => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = f.SetParagraphAlignment(if wrap { DWRITE_PARAGRAPH_ALIGNMENT_NEAR } else { DWRITE_PARAGRAPH_ALIGNMENT_CENTER });
            if !wrap {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                    let _ = f.SetTrimming(&trim, &sign);
                }
            } else {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            }
            self.formats.borrow_mut().insert(key, f.clone());
            f
        }
    }

    /// Single-line text, vertically centred in `r`, trimmed with an ellipsis.
    pub fn text(&mut self, s: &str, r: Rect, font: Font, size: f32, weight: u32, align: Align, c: C) {
        if s.is_empty() || r.w <= 1.0 {
            return;
        }
        let f = self.format(font, size, weight, align, false, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe { self.rt().DrawText(&w, &f, &r.d2(), self.b(c), D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL) };
    }

    /// Single line with coloured / bold ranges (search matches, tokens, tabular parts).
    pub fn text_spans(&mut self, s: &str, r: Rect, font: Font, size: f32, weight: u32, c: C, spans: &[Span]) {
        if s.is_empty() || r.w <= 1.0 {
            return;
        }
        let f = self.format(font, size, weight, Align::Left, false, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, r.w, r.h) else { return };
            let mut keep = vec![];
            for sp in spans {
                let range = DWRITE_TEXT_RANGE { startPosition: sp.start, length: sp.len };
                if sp.weight != weight {
                    let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT(sp.weight as i32), range);
                }
                if sp.italic {
                    let _ = layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range);
                }
                if let Ok(b) = self.rt().CreateSolidColorBrush(&sp.color.d2(), None) {
                    let _ = layout.SetDrawingEffect(&b, range);
                    keep.push(b);
                }
            }
            self.rt().DrawTextLayout(Vector2 { X: r.x, Y: r.y }, &layout, self.b(c), D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
    }

    /// Wrapped paragraph from the top of `r`; returns the height used. `rtl` = right-to-left paragraph.
    pub fn para(&mut self, s: &str, r: Rect, font: Font, size: f32, weight: u32, align: Align, c: C, line_h: f32, rtl: bool, max_lines: u32) -> f32 {
        let f = self.format(font, size, weight, align, true, rtl);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let max_h = if max_lines > 0 && line_h > 0.0 { line_h * max_lines as f32 + 1.0 } else { 10000.0 };
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, r.w, max_h) else { return 0.0 };
            if line_h > 0.0 {
                let _ = layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_h, line_h * 0.78);
            }
            if max_lines > 0 {
                let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                    let _ = layout.SetTrimming(&trim, &sign);
                }
                let _ = layout.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK);
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            self.rt().DrawTextLayout(Vector2 { X: r.x, Y: r.y }, &layout, self.b(c), D2D1_DRAW_TEXT_OPTIONS_CLIP);
            m.height.min(max_h)
        }
    }

    pub fn para_height(&mut self, s: &str, w_: f32, font: Font, size: f32, line_h: f32, rtl: bool) -> f32 {
        let f = self.format(font, size, 400, Align::Left, true, rtl);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, w_, 10000.0) else { return 0.0 };
            if line_h > 0.0 {
                let _ = layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_h, line_h * 0.78);
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.height
        }
    }

    pub fn measure(&mut self, s: &str, font: Font, size: f32, weight: u32) -> f32 {
        let f = self.format(font, size, weight, Align::Left, false, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, 10000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }

    /// x offset of the caret before UTF-16 index `idx` (handles mixed Persian / Latin text).
    pub fn caret_x(&mut self, s: &str, font: Font, size: f32, weight: u32, idx: usize) -> f32 {
        let f = self.format(font, size, weight, Align::Left, false, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, 10000.0, 100.0) else { return 0.0 };
            let (mut x, mut y) = (0f32, 0f32);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let trailing = idx >= w.len() && !w.is_empty();
            let pos = if trailing { w.len() - 1 } else { idx };
            let _ = layout.HitTestTextPosition(pos as u32, trailing, &mut x, &mut y, &mut m);
            x
        }
    }

    /// UTF-16 index nearest to x.
    pub fn index_at(&mut self, s: &str, font: Font, size: f32, weight: u32, x: f32) -> usize {
        let f = self.format(font, size, weight, Align::Left, false, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, 10000.0, 100.0) else { return 0 };
            let mut trailing = windows::core::BOOL(0);
            let mut inside = windows::core::BOOL(0);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = layout.HitTestPoint(x, 5.0, &mut trailing, &mut inside, &mut m);
            (m.textPosition + if trailing.as_bool() { m.length } else { 0 }) as usize
        }
    }

    pub fn glyph(&mut self, code: u32, r: Rect, size: f32, c: C) {
        let s = char::from_u32(code).map(|c| c.to_string()).unwrap_or_default();
        self.text(&s, r, Font::Icon, size, 400, Align::Center, c);
    }

    pub fn bitmap(&self, bmp: &ID2D1Bitmap, r: Rect, opacity: f32) {
        unsafe { self.rt().DrawBitmap(bmp, Some(&r.d2()), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
    }

    /// Draw part `src` (in bitmap pixels) of a bitmap into `dst`.
    pub fn bitmap_src(&self, bmp: &ID2D1Bitmap, dst: Rect, src: Rect, opacity: f32) {
        unsafe { self.rt().DrawBitmap(bmp, Some(&dst.d2()), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, Some(&src.d2())) };
    }

    /// Left-to-right gradient fill (the Mica fallback).
    pub fn gradient(&self, r: Rect, a: C, b: C) {
        unsafe {
            let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: a.d2() }, D2D1_GRADIENT_STOP { position: 1.0, color: b.d2() }];
            let Ok(coll) = self.rt().CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP) else { return };
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: r.x, Y: r.y }, endPoint: Vector2 { X: r.r(), Y: r.b() } };
            if let Ok(br) = self.rt().CreateLinearGradientBrush(&props, None, &coll) {
                self.rt().FillRectangle(&r.d2(), &br);
            }
        }
    }

    /// 32-bit premultiplied BGRA pixels -> bitmap on the current target.
    pub fn make_bitmap(&self, w: u32, h: u32, bgra: &[u8]) -> Option<ID2D1Bitmap> {
        let rt = self.rt();
        let props = D2D1_BITMAP_PROPERTIES { pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0, dpiY: 96.0 };
        unsafe { rt.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(bgra.as_ptr() as *const _), w * 4, &props).ok() }
    }

    pub fn target_id(&self) -> usize {
        self.over.as_ref().or(self.base.as_ref()).map(|r| r.as_raw() as usize).unwrap_or(0)
    }
}
