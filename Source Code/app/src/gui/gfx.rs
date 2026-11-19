//! Thin Direct2D / DirectWrite layer: one render target, one brush, cached text formats, a few draw helpers.
//! Everything is in DIPs; the render target DPI follows the window.
//! Vazirmatn (Regular + SemiBold, subset to Arabic + Latin) is embedded and loaded as an in-memory font collection;
//! it is also the fallback font for Arabic-script runs inside the Segoe UI text (Persian process names).
use crate::sys::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{Interface, PCWSTR};
use windows_numerics::Vector2;

static FA_REGULAR: &[u8] = include_bytes!("../../assets/Vazirmatn-Regular.ttf");
static FA_SEMIBOLD: &[u8] = include_bytes!("../../assets/Vazirmatn-SemiBold.ttf");

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct C(pub f32, pub f32, pub f32, pub f32);

impl C {
    pub const fn hex(v: u32) -> C {
        C(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0, 1.0)
    }
    pub const fn a(self, a: f32) -> C {
        C(self.0, self.1, self.2, a)
    }
    /// multiply the alpha
    pub fn fade(self, f: f32) -> C {
        C(self.0, self.1, self.2, self.3 * f)
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
    pub fn off(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
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
    /// Vazirmatn, right-to-left paragraphs
    Fa,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FmtKey(Font, u32, u32, Align, bool);

pub struct Gfx {
    pub d2: ID2D1Factory,
    pub dw: IDWriteFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    base: Option<ID2D1RenderTarget>,
    /// offscreen target used for screenshots (drawn instead of the window while set)
    over: Option<ID2D1RenderTarget>,
    brush: Option<ID2D1SolidColorBrush>,
    formats: RefCell<HashMap<FmtKey, IDWriteTextFormat>>,
    hwnd: HWND,
    pub software: bool,
    /// window target keeps per-pixel alpha (Mica shows through transparent pixels)
    pub alpha: bool,
    pub dpi: f32,
    fa: Option<IDWriteFontCollection>,
    fallback: Option<IDWriteFontFallback>,
    icon_family: String,
    mono_family: String,
}

pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Bez(f32, f32, f32, f32, f32, f32),
}

impl Gfx {
    pub fn new(hwnd: HWND, software: bool) -> windows::core::Result<Gfx> {
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
            let mono_family = if has("Cascadia Mono") { "Cascadia Mono" } else { "Consolas" }.to_string();
            let mut g = Gfx { d2, dw, rt: None, base: None, over: None, brush: None, formats: RefCell::new(HashMap::new()), hwnd, software, alpha: false, dpi: 96.0, fa: None, fallback: None, icon_family, mono_family };
            g.load_fa();
            Ok(g)
        }
    }

    /// Embedded Vazirmatn as an in-memory collection (Windows 10 1703+), plus an Arabic-script fallback that uses it.
    fn load_fa(&mut self) {
        unsafe {
            let Ok(f5) = self.dw.cast::<IDWriteFactory5>() else { return };
            let Ok(loader) = f5.CreateInMemoryFontFileLoader() else { return };
            if f5.RegisterFontFileLoader(&loader).is_err() {
                return;
            }
            let Ok(builder) = f5.CreateFontSetBuilder() else { return };
            for data in [FA_REGULAR, FA_SEMIBOLD] {
                if let Ok(file) = loader.CreateInMemoryFontFileReference(&self.dw, data.as_ptr() as *const _, data.len() as u32, None) {
                    let _ = builder.AddFontFile(&file);
                }
            }
            let Ok(set) = builder.CreateFontSet() else { return };
            let Ok(coll) = f5.CreateFontCollectionFromFontSet(&set) else { return };
            let Ok(coll) = coll.cast::<IDWriteFontCollection>() else { return };
            // fallback: Arabic script -> Vazirmatn, everything else -> the system fallback
            if let Ok(fb) = f5.CreateFontFallbackBuilder() {
                let ranges = [
                    DWRITE_UNICODE_RANGE { first: 0x0600, last: 0x06FF },
                    DWRITE_UNICODE_RANGE { first: 0xFB50, last: 0xFDFF },
                    DWRITE_UNICODE_RANGE { first: 0xFE70, last: 0xFEFF },
                ];
                let name = wide("Vazirmatn");
                let names = [name.as_ptr()];
                let _ = fb.AddMapping(&ranges, &names, &coll, PCWSTR::null(), PCWSTR::null(), 1.0);
                if let Ok(sys) = f5.GetSystemFontFallback() {
                    let _ = fb.AddMappings(&sys);
                }
                self.fallback = fb.CreateFontFallback().ok();
            }
            self.fa = Some(coll);
        }
    }

    pub fn scale(&self) -> f32 {
        self.dpi / 96.0
    }

    /// Create (or resize) the render target for the window's pixel size.
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
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: if self.alpha { D2D1_ALPHA_MODE_PREMULTIPLIED } else { D2D1_ALPHA_MODE_IGNORE } },
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

    /// Drop the window target (for example when Mica is switched on or off).
    pub fn reset(&mut self) {
        self.rt = None;
        self.base = None;
        self.brush = None;
    }

    pub fn offscreen(&self) -> bool {
        self.over.is_some()
    }

    pub fn begin(&self) {
        unsafe { self.rt().BeginDraw() };
    }

    /// Returns false when the device was lost (target is dropped and rebuilt next frame).
    pub fn end(&mut self) -> bool {
        let r = unsafe { self.rt().EndDraw(None, None) };
        if let Err(e) = &r {
            super::app::trace(&format!("EndDraw failed (offscreen={}): {e:?}", self.over.is_some()));
        }
        if r.is_err() && self.over.is_none() {
            self.reset();
            return false;
        }
        true
    }

    fn rt(&self) -> &ID2D1RenderTarget {
        self.over.as_ref().or(self.base.as_ref()).unwrap()
    }

    /// Draw the next frames into a memory DC instead of the window (for `wtm gui shot`).
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
        if c.3 <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
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

    /// Fill + 1px border (+ optional darker bottom border, like Fluent controls).
    pub fn card(&self, r: Rect, rad: f32, fill: C, border: C) {
        self.fill_round(r, rad, fill);
        self.stroke_round(r, rad, border, 1.0);
    }

    /// Soft drop shadow under a rounded rect: a few expanding layers at low alpha (approximates a blur).
    pub fn shadow(&self, r: Rect, rad: f32, dy: f32, blur: f32, alpha: f32) {
        // box-blur approximation: N stacked layers from blur/2 inside to blur/2 outside the offset rect,
        // so the edge sits at half strength and the falloff is linear (close to a CSS box-shadow)
        // one layer per ~2 px keeps the steps invisible
        let n = ((blur / 2.0) as usize).clamp(4, 32);
        for i in 0..n {
            let grow = -blur * 0.5 + blur * (i as f32 + 0.5) / n as f32;
            let rr = Rect::new(r.x - grow, r.y + dy - grow, r.w + grow * 2.0, r.h + grow * 2.0);
            if rr.w <= 0.0 || rr.h <= 0.0 {
                continue;
            }
            self.fill_round(rr, (rad + grow).max(0.0), C(0.0, 0.0, 0.0, alpha / n as f32));
        }
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: C, w: f32) {
        unsafe { self.rt().DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.b(c), w, None) };
    }

    /// 1px horizontal line on the pixel grid
    pub fn hline(&self, x0: f32, x1: f32, y: f32, c: C) {
        self.fill(Rect::new(x0, y, x1 - x0, 1.0), c);
    }

    pub fn vline(&self, x: f32, y0: f32, y1: f32, c: C) {
        self.fill(Rect::new(x, y0, 1.0, y1 - y0), c);
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

    pub fn clip(&self, r: Rect) {
        unsafe { self.rt().PushAxisAlignedClip(&r.d2(), D2D1_ANTIALIAS_MODE_ALIASED) };
    }

    pub fn unclip(&self) {
        unsafe { self.rt().PopAxisAlignedClip() };
    }

    /// Transform for the next draws (slide / scale animations). `None` resets.
    pub fn transform(&self, t: Option<(f32, f32, f32, f32, f32)>) {
        // (dx, dy, scale, cx, cy)
        let m = match t {
            None => windows_numerics::Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 },
            Some((dx, dy, s, cx, cy)) => windows_numerics::Matrix3x2 { M11: s, M12: 0.0, M21: 0.0, M22: s, M31: cx - cx * s + dx, M32: cy - cy * s + dy },
        };
        unsafe { self.rt().SetTransform(&m) };
    }

    fn format(&self, font: Font, size: f32, weight: u32, align: Align, wrap: bool) -> IDWriteTextFormat {
        let key = FmtKey(font, (size * 10.0) as u32, weight, align, wrap);
        if let Some(f) = self.formats.borrow().get(&key) {
            return f.clone();
        }
        let fa = font == Font::Fa && self.fa.is_some();
        let family = match font {
            Font::Ui => "Segoe UI Variable Text",
            Font::Display => "Segoe UI Variable Display",
            Font::Icon => &self.icon_family,
            Font::Mono => &self.mono_family,
            Font::Fa => {
                if fa {
                    "Vazirmatn"
                } else {
                    "Segoe UI"
                }
            }
        };
        let fam = wide(family);
        let loc = wide(if font == Font::Fa { "fa-IR" } else { "en-us" });
        unsafe {
            let coll = if fa { self.fa.as_ref() } else { None };
            let f = self.dw.CreateTextFormat(PCWSTR(fam.as_ptr()), coll, DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, PCWSTR(loc.as_ptr())).unwrap();
            let rtl = font == Font::Fa;
            // in a right-to-left paragraph "leading" is the right edge
            let _ = f.SetTextAlignment(match (align, rtl) {
                (Align::Left, false) | (Align::Right, true) => DWRITE_TEXT_ALIGNMENT_LEADING,
                (Align::Center, _) => DWRITE_TEXT_ALIGNMENT_CENTER,
                _ => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = f.SetParagraphAlignment(if wrap { DWRITE_PARAGRAPH_ALIGNMENT_NEAR } else { DWRITE_PARAGRAPH_ALIGNMENT_CENTER });
            if rtl {
                let _ = f.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT);
            }
            if font != Font::Icon
                && let Some(fb) = &self.fallback
                && let Ok(f1) = f.cast::<IDWriteTextFormat1>()
            {
                let _ = f1.SetFontFallback(fb);
            }
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
    pub fn text(&self, s: &str, r: Rect, font: Font, size: f32, weight: u32, align: Align, c: C) {
        if s.is_empty() || r.w <= 1.0 || c.3 <= 0.0 {
            return;
        }
        let f = self.format(font, size, weight, align, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe { self.rt().DrawText(&w, &f, &r.d2(), self.b(c), D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL) };
    }

    /// Text that starts at x (left aligned, vertically centred in [y, y+h]); returns its width.
    pub fn text_at(&self, s: &str, x: f32, y: f32, h: f32, max_w: f32, font: Font, size: f32, weight: u32, c: C) -> f32 {
        let w = self.measure(s, font, size, weight).min(max_w.max(0.0));
        self.text(s, Rect::new(x, y, max_w.max(0.0) + 1.0, h), font, size, weight, Align::Left, c);
        w
    }

    fn layout(&self, s: &str, w: f32, font: Font, size: f32, weight: u32, align: Align, line_h: f32) -> Option<IDWriteTextLayout> {
        let f = self.format(font, size, weight, align, true);
        let ws: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let layout = self.dw.CreateTextLayout(&ws, &f, w.max(1.0), 10000.0).ok()?;
            if line_h > 0.0 {
                let _ = layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_h, line_h * 0.76);
            }
            Some(layout)
        }
    }

    /// Wrapped paragraph from the top of `r`; returns the height used.
    pub fn para(&self, s: &str, r: Rect, font: Font, size: f32, weight: u32, align: Align, c: C, line_h: f32) -> f32 {
        let Some(layout) = self.layout(s, r.w, font, size, weight, align, line_h) else { return 0.0 };
        unsafe {
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            self.rt().DrawTextLayout(Vector2 { X: r.x, Y: r.y }, &layout, self.b(c), D2D1_DRAW_TEXT_OPTIONS_NONE);
            m.height
        }
    }

    /// Height a wrapped paragraph needs at width `w`.
    pub fn para_h(&self, s: &str, w: f32, font: Font, size: f32, weight: u32, line_h: f32) -> f32 {
        let Some(layout) = self.layout(s, w, font, size, weight, Align::Left, line_h) else { return 0.0 };
        unsafe {
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.height
        }
    }

    pub fn measure(&self, s: &str, font: Font, size: f32, weight: u32) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let f = self.format(font, size, weight, Align::Left, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, 10000.0, 100.0) else { return 0.0 };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }

    /// Where a character range of a single-line text is drawn: (x, width) pieces, bidi-correct
    /// (a Persian run inside an English name is laid out right to left). `start`/`len` are in UTF-16 units.
    pub fn range_x(&self, s: &str, font: Font, size: f32, weight: u32, start: u32, len: u32) -> Vec<(f32, f32)> {
        let f = self.format(font, size, weight, Align::Left, false);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(layout) = self.dw.CreateTextLayout(&w, &f, 10000.0, 100.0) else { return vec![] };
            let mut buf = [DWRITE_HIT_TEST_METRICS::default(); 8];
            let mut n = 0u32;
            if layout.HitTestTextRange(start, len, 0.0, 0.0, Some(&mut buf), &mut n).is_err() {
                return vec![];
            }
            buf[..(n as usize).min(8)].iter().map(|m| (m.left, m.width)).collect()
        }
    }

    /// Shorten `s` from the start ("…\bin\xray.exe") so it fits `w`.
    pub fn elide_start(&self, s: &str, w: f32, font: Font, size: f32, weight: u32) -> String {
        if self.measure(s, font, size, weight) <= w {
            return s.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let t: String = std::iter::once('…').chain(chars[mid..].iter().copied()).collect();
            if self.measure(&t, font, size, weight) <= w { hi = mid } else { lo = mid + 1 }
        }
        std::iter::once('…').chain(chars[lo.min(chars.len())..].iter().copied()).collect()
    }

    /// Shorten `s` in the middle, keeping everything from `keep_from` (char index) to the end visible.
    pub fn elide_mid(&self, s: &str, keep_from: usize, w: f32, font: Font, size: f32, weight: u32) -> String {
        if self.measure(s, font, size, weight) <= w {
            return s.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        let keep_from = keep_from.min(chars.len());
        let tail: String = chars[keep_from..].iter().collect();
        let (mut lo, mut hi) = (0usize, keep_from);
        // longest head that still fits
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let t: String = chars[..mid].iter().collect::<String>() + "…" + &tail;
            if self.measure(&t, font, size, weight) <= w { lo = mid } else { hi = mid - 1 }
        }
        if lo == 0 {
            return self.elide_start(s, w, font, size, weight);
        }
        chars[..lo].iter().collect::<String>() + "…" + &tail
    }

    pub fn glyph(&self, code: u32, r: Rect, size: f32, c: C) {
        let s = char::from_u32(code).map(|c| c.to_string()).unwrap_or_default();
        self.text(&s, r, Font::Icon, size, 400, Align::Center, c);
    }

    /// Sparkline / graph: `vals` scaled 0..`max` across `r` (oldest left): area at `fill_alpha` + line.
    pub fn spark(&self, r: Rect, vals: &[f32], max: f32, c: C, fill_alpha: f32, lw: f32) {
        if vals.len() < 2 {
            return;
        }
        let max = if max <= 0.0 { 1.0 } else { max };
        let n = vals.len();
        let pt = |i: usize| Vector2 { X: r.x + r.w * i as f32 / (n - 1) as f32, Y: r.b() - 1.0 - (r.h - 2.0) * (vals[i] / max).clamp(0.0, 1.0) };
        unsafe {
            let Ok(path) = self.d2.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            sink.BeginFigure(Vector2 { X: r.x, Y: r.b() }, D2D1_FIGURE_BEGIN_FILLED);
            for i in 0..n {
                sink.AddLine(pt(i));
            }
            sink.AddLine(Vector2 { X: r.r(), Y: r.b() });
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = sink.Close();
            self.rt().FillGeometry(&path, self.b(c.a(fill_alpha * c.3)), None);
            let Ok(lp) = self.d2.CreatePathGeometry() else { return };
            let Ok(ls) = lp.Open() else { return };
            ls.BeginFigure(pt(0), D2D1_FIGURE_BEGIN_HOLLOW);
            for i in 1..n {
                ls.AddLine(pt(i));
            }
            ls.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = ls.Close();
            let style = self.d2.CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() }, None).ok();
            self.rt().DrawGeometry(&lp, self.b(c), lw, style.as_ref());
        }
    }

    /// Fill a path made of segments, in a 16x16 design box scaled to `size` at (x, y).
    pub fn path16(&self, segs: &[Seg], x: f32, y: f32, size: f32, c: C) {
        let k = size / 16.0;
        let p = |a: f32, b: f32| Vector2 { X: x + a * k, Y: y + b * k };
        unsafe {
            let Ok(path) = self.d2.CreatePathGeometry() else { return };
            let Ok(sink) = path.Open() else { return };
            let mut open = false;
            for s in segs {
                match *s {
                    Seg::Move(a, b) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        }
                        sink.BeginFigure(p(a, b), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    Seg::Line(a, b) => sink.AddLine(p(a, b)),
                    Seg::Bez(a, b, c2, d, e, f) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: p(a, b), point2: p(c2, d), point3: p(e, f) }),
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            let _ = sink.Close();
            self.rt().FillGeometry(&path, self.b(c), None);
        }
    }

    /// The "What is this?" sparkle (two four-point stars).
    pub fn sparkle(&self, x: f32, y: f32, size: f32, c: C) {
        use Seg::*;
        let segs = [
            Move(7.0, 1.5),
            Bez(7.4, 4.4, 9.1, 6.1, 12.0, 6.5),
            Bez(9.1, 6.9, 7.4, 8.6, 7.0, 11.5),
            Bez(6.6, 8.6, 4.9, 6.9, 2.0, 6.5),
            Bez(4.9, 6.1, 6.6, 4.4, 7.0, 1.5),
            Move(12.5, 9.5),
            Bez(12.7, 10.8, 13.4, 11.5, 14.7, 11.7),
            Bez(13.4, 11.9, 12.7, 12.6, 12.5, 13.9),
            Bez(12.3, 12.6, 11.6, 11.9, 10.3, 11.7),
            Bez(11.6, 11.5, 12.3, 10.8, 12.5, 9.5),
        ];
        self.path16(&segs, x, y, size, c);
    }

    pub fn bitmap(&self, bmp: &ID2D1Bitmap, r: Rect, opacity: f32) {
        unsafe { self.rt().DrawBitmap(bmp, Some(&r.d2()), opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
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

    /// Is the embedded Persian font loaded?
    pub fn has_fa(&self) -> bool {
        self.fa.is_some()
    }
}
