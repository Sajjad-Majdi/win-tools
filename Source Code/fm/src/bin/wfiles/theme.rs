//! Colour tokens from design-fm/design-notes.md (light and dark). The accent follows the Windows accent colour.
use crate::gfx::C;

/// Corner radii, in one place. Clearly rounder than Windows (owner's choice, 2026-11-19). Personalize > Corners
/// "Square" scales them down in Gfx; "Rounded" (these values) is the default.
/// Cards, panels and the This PC drive tiles.
pub const R_CARD: f32 = 12.0;
/// Buttons, inputs, pills, rows and selection highlights.
pub const R_CTRL: f32 = 8.0;
/// Menus, flyouts and dialogs.
pub const R_MENU: f32 = 12.0;
/// Small icon buttons inside other controls (tab close, breadcrumbs, key caps).
pub const R_SMALL: f32 = 6.0;

#[derive(Clone)]
pub struct Theme {
    pub dark: bool,
    pub mica_a: C,
    pub mica_b: C,
    pub bar: C,
    pub layer: C,
    pub layer2: C,
    pub card: C,
    pub field: C,
    pub field_border: C,
    pub field_bottom: C,
    pub stroke: C,
    pub divider: C,
    pub text: C,
    pub text2: C,
    pub text3: C,
    pub text_dis: C,
    pub hover: C,
    pub press: C,
    pub navsel: C,
    pub accent: C,
    pub accent2: C,
    pub on_accent: C,
    pub accent_text: C,
    pub sel: C,
    pub sel_stroke: C,
    pub ok: C,
    pub warn: C,
    pub warn_fill: C,
    pub warn_stroke: C,
    pub danger: C,
    pub track: C,
    pub scrim: C,
    pub glass: C,
    pub glass_stroke: C,
    pub folder: C,
    pub folder_back: C,
    pub page: C,
    pub page_stroke: C,
    pub code_bg: C,
    pub kbd: C,
    pub k: C,
    pub ty: C,
    pub st: C,
    pub cm: C,
    pub nu: C,
    pub shadow: f32,
}

pub fn light() -> Theme {
    Theme {
        dark: false,
        mica_a: C::hex(0xE8ECF3),
        mica_b: C::hex(0xEEEBF2),
        bar: C::hex(0xF6F7FA),
        layer: C::hex(0xF8F9FB),
        layer2: C::hex(0xF0F2F6),
        card: C::hex(0xFAFBFC),
        field: C::hex(0xFBFCFD),
        field_border: C::hex(0xD8DCE3),
        field_bottom: C::hex(0xB4BAC4),
        stroke: C::hex(0xE1E4EA),
        divider: C::hex(0xE8EBEF),
        text: C::hex(0x1A1D23),
        text2: C::hex(0x59606C),
        text3: C::hex(0x7F8692),
        text_dis: C::hex(0xA7ADB6),
        hover: C::rgba(25, 40, 70, 0.055),
        press: C::rgba(25, 40, 70, 0.09),
        navsel: C::hex(0xE2E7EF),
        accent: C::hex(0x005FB8),
        accent2: C::hex(0x1A6FC2),
        on_accent: C::hex(0xFFFFFF),
        accent_text: C::hex(0x005FB8),
        sel: C::hex(0xDCE9F7),
        sel_stroke: C::hex(0xA9C9EA),
        ok: C::hex(0x0F7B0F),
        warn: C::hex(0x8F5500),
        warn_fill: C::hex(0xFBF1D6),
        warn_stroke: C::hex(0xEBCF86),
        danger: C::hex(0xC42B1C),
        track: C::hex(0xDCE0E7),
        scrim: C::rgba(20, 28, 42, 0.28),
        glass: C::rgba(246, 248, 251, 0.92),
        glass_stroke: C::rgba(255, 255, 255, 0.75),
        folder: C::hex(0xF3C14E),
        folder_back: C::hex(0xDDA43A),
        page: C::hex(0xFBFCFD),
        page_stroke: C::hex(0x8E97A4),
        code_bg: C::hex(0xF3F5F9),
        kbd: C::hex(0xECEFF4),
        k: C::hex(0x8A2BB0),
        ty: C::hex(0x0B6E99),
        st: C::hex(0xA1460F),
        cm: C::hex(0x6B7480),
        nu: C::hex(0x0E7C4A),
        shadow: 0.14,
    }
}

pub fn dark() -> Theme {
    Theme {
        dark: true,
        mica_a: C::hex(0x1B1F27),
        mica_b: C::hex(0x201D26),
        bar: C::hex(0x262A32),
        layer: C::hex(0x272B34),
        layer2: C::hex(0x2C313B),
        card: C::hex(0x2D323C),
        field: C::hex(0x2F343E),
        field_border: C::hex(0x3C4350),
        field_bottom: C::hex(0x7D8695),
        stroke: C::hex(0x363C47),
        divider: C::hex(0x323842),
        text: C::hex(0xEEF1F5),
        text2: C::hex(0xB0B7C3),
        text3: C::hex(0x8A919D),
        text_dis: C::hex(0x5E6672),
        hover: C::rgba(255, 255, 255, 0.055),
        press: C::rgba(255, 255, 255, 0.085),
        navsel: C::hex(0x323844),
        accent: C::hex(0x60CDFF),
        accent2: C::hex(0x7AD6FF),
        on_accent: C::hex(0x06202D),
        accent_text: C::hex(0x6CD1FF),
        sel: C::hex(0x2B4054),
        sel_stroke: C::hex(0x3F6A8C),
        ok: C::hex(0x6CCB5F),
        warn: C::hex(0xF2C661),
        warn_fill: C::hex(0x3A3220),
        warn_stroke: C::hex(0x5F4E25),
        danger: C::hex(0xFF99A4),
        track: C::hex(0x3B424E),
        scrim: C::rgba(0, 0, 0, 0.46),
        glass: C::rgba(48, 54, 64, 0.92),
        glass_stroke: C::rgba(255, 255, 255, 0.08),
        folder: C::hex(0xE8B84A),
        folder_back: C::hex(0xC69234),
        page: C::hex(0xE8EBF0),
        page_stroke: C::hex(0x8C95A2),
        code_bg: C::hex(0x22262E),
        kbd: C::hex(0x353B46),
        k: C::hex(0xD7A1F0),
        ty: C::hex(0x6FD0F5),
        st: C::hex(0xE8B07A),
        cm: C::hex(0x8B94A1),
        nu: C::hex(0x86D9A6),
        shadow: 0.40,
    }
}

fn reg_dword(path: &str, name: &str) -> Option<u32> {
    reg_bin(path, name).and_then(|b| b.get(0..4).map(|s| u32::from_le_bytes(s.try_into().unwrap())))
}

fn reg_bin(path: &str, name: &str) -> Option<Vec<u8>> {
    use windows::Win32::System::Registry::*;
    let p = fmcore::util::wide(path);
    let n = fmcore::util::wide(name);
    let mut buf = vec![0u8; 64];
    let mut size = buf.len() as u32;
    unsafe {
        let r = RegGetValueW(HKEY_CURRENT_USER, windows::core::PCWSTR(p.as_ptr()), windows::core::PCWSTR(n.as_ptr()), RRF_RT_ANY, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size));
        if r.0 != 0 {
            return None;
        }
    }
    buf.truncate(size as usize);
    Some(buf)
}

/// Windows app theme: AppsUseLightTheme = 0 means dark.
pub fn system_dark() -> bool {
    reg_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme") == Some(0)
}

/// Theme for `dark`: one tonal palette from `tint`, then the user's chosen accent (`custom`), else the Windows accent (AccentPalette: light3, light2,
/// light1, base, dark1, dark2, dark3).
pub fn make(dark_mode: bool, custom: Option<u32>, tint: Tint, strength: u8) -> Theme {
    let mut t = if dark_mode { dark() } else { light() };
    tonal(&mut t, tint, strength);
    if let Some(c) = custom {
        set_accent(&mut t, C::hex(c));
        return t;
    }
    if let Some(p) = reg_bin(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent", "AccentPalette")
        && p.len() >= 28
    {
        let c = |i: usize| C::rgba(p[i * 4], p[i * 4 + 1], p[i * 4 + 2], 1.0);
        let base = c(3);
        let is_default_blue = p[12] == 0x00 && p[13] == 0x78 && p[14] == 0xD4;
        if !is_default_blue {
            if dark_mode {
                t.accent = c(1);
                t.accent2 = c(0);
                t.accent_text = c(0);
                t.on_accent = C::hex(0x0A0F14);
                t.sel = t.layer.mix(base, 0.30);
                t.sel_stroke = t.layer.mix(base, 0.55);
            } else {
                t.accent = c(4);
                t.accent2 = base;
                t.accent_text = c(4);
                t.sel = t.layer.mix(base, 0.14);
                t.sel_stroke = t.layer.mix(base, 0.38);
            }
        }
    }
    t
}

/// Accent tokens from one base colour: darker for light mode, lighter for dark mode (readable text on both).
pub fn set_accent(t: &mut Theme, base: C) {
    let white = C::hex(0xFFFFFF);
    let black = C::hex(0x000000);
    if t.dark {
        t.accent = base.mix(white, 0.42);
        t.accent2 = base.mix(white, 0.56);
        t.accent_text = base.mix(white, 0.50);
        t.on_accent = C::hex(0x0A0F14);
        t.sel = t.layer.mix(base, 0.30);
        t.sel_stroke = t.layer.mix(base, 0.55);
    } else {
        t.accent = base.mix(black, 0.12);
        t.accent2 = base;
        t.accent_text = base.mix(black, 0.18);
        t.on_accent = C::hex(0xFFFFFF);
        t.sel = t.layer.mix(base, 0.14);
        t.sel_stroke = t.layer.mix(base, 0.38);
    }
}

// ---------------------------------------------------------------------------------------------- one tinted palette
/// The base of the window's palette: a hue (degrees, OKLCH) and how colourful its source is (OKLCH chroma).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub hue: f32,
    pub chroma: f32,
}

impl Tint {
    pub const NEUTRAL: Tint = Tint { hue: 250.0, chroma: 0.0 };
    pub fn of(c: C) -> Tint {
        let (_, ch, h) = to_oklch(c);
        Tint { hue: h, chroma: ch }
    }
}

fn to_lin(x: f32) -> f32 {
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}
fn from_lin(x: f32) -> f32 {
    if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB -> OKLCH (L 0..1, chroma, hue in degrees).
pub fn to_oklch(c: C) -> (f32, f32, f32) {
    let (r, g, b) = (to_lin(c.0), to_lin(c.1), to_lin(c.2));
    oklab_lch(r, g, b)
}

/// Linear sRGB -> OKLCH.
pub fn oklab_lch(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ll = 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s;
    (ll, (a * a + bb * bb).sqrt(), bb.atan2(a).to_degrees().rem_euclid(360.0))
}

fn lch_rgb(l: f32, c: f32, h: f32) -> (f32, f32, f32) {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    (
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    )
}

/// OKLCH -> sRGB; the chroma shrinks until the colour fits the sRGB gamut.
pub fn oklch(l: f32, c: f32, h: f32) -> C {
    let mut c = c.max(0.0);
    for _ in 0..24 {
        let (r, g, b) = lch_rgb(l, c, h);
        if (-0.0005..=1.0005).contains(&r) && (-0.0005..=1.0005).contains(&g) && (-0.0005..=1.0005).contains(&b) || c < 0.0005 {
            return C(from_lin(r.clamp(0.0, 1.0)), from_lin(g.clamp(0.0, 1.0)), from_lin(b.clamp(0.0, 1.0)), 1.0);
        }
        c *= 0.85;
    }
    let (r, g, b) = lch_rgb(l, 0.0, h);
    C(from_lin(r.clamp(0.0, 1.0)), from_lin(g.clamp(0.0, 1.0)), from_lin(b.clamp(0.0, 1.0)), 1.0)
}

/// A colour as 0xRRGGBB.
pub fn to_hex(c: C) -> u32 {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    (b(c.0) << 16) | (b(c.1) << 8) | b(c.2)
}

/// WCAG relative luminance and contrast ratio.
fn lum(c: C) -> f32 {
    0.2126 * to_lin(c.0) + 0.7152 * to_lin(c.1) + 0.0722 * to_lin(c.2)
}
pub fn contrast(a: C, b: C) -> f32 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The surface chroma: low, whatever the source is (the window stays calm), and none for a grey source.
/// Chroma of the panel tones for a Colour strength (0 Subtle, 1 Medium, 2 Strong). Medium reads clearly as the
/// accent's hue; a grey accent (Graphite) stays nearly grey.
pub fn surface_chroma(t: Tint, dark: bool, strength: u8) -> f32 {
    let want: f32 = if dark { [0.016, 0.030, 0.048] } else { [0.012, 0.024, 0.040] }[strength.min(2) as usize];
    want.min(t.chroma * 0.6)
}

/// Rebuild every surface, border and text token as lightness steps of ONE hue with low chroma, so the window,
/// title bar, sidebar, content and cards read as one colour family (light and dark). Text keeps WCAG AA (4.5:1) on
/// the panels it sits on; the lightness moves until it does.
pub fn tonal(t: &mut Theme, tint: Tint, strength: u8) {
    let h = tint.hue;
    let cs = surface_chroma(tint, t.dark, strength);
    let ct = (cs * 0.4).min(0.014);
    let s = |l: f32| oklch(l, cs, h);
    // (window, bar, layer, layer2, card, field, field_border, stroke, divider, navsel, track, code_bg, kbd)
    let ls: [f32; 13] = if t.dark {
        [0.205, 0.238, 0.245, 0.272, 0.278, 0.288, 0.365, 0.322, 0.302, 0.315, 0.345, 0.222, 0.315]
    } else {
        [0.928, 0.962, 0.972, 0.950, 0.982, 0.988, 0.875, 0.905, 0.918, 0.920, 0.892, 0.958, 0.940]
    };
    t.mica_a = s(ls[0]);
    t.mica_b = oklch(ls[0], cs, h + 18.0);
    t.bar = s(ls[1]);
    t.layer = s(ls[2]);
    t.layer2 = s(ls[3]);
    t.card = s(ls[4]);
    t.field = s(ls[5]);
    t.field_border = s(ls[6]);
    t.stroke = s(ls[7]);
    t.divider = s(ls[8]);
    t.navsel = s(ls[9]);
    t.track = s(ls[10]);
    t.code_bg = s(ls[11]);
    t.kbd = s(ls[12]);
    t.glass = t.card.a(0.92);
    // hover and press are the same hue too (a light or dark veil of it), so nothing reads as neutral grey
    t.hover = if t.dark { oklch(0.80, cs * 2.0, h).a(0.075) } else { oklch(0.45, cs * 2.0, h).a(0.065) };
    t.press = if t.dark { oklch(0.80, cs * 2.0, h).a(0.11) } else { oklch(0.45, cs * 2.0, h).a(0.10) };
    // text: the darkest panel it sits on decides (layer2 in dark mode, the window colour in light mode)
    let worst = |a: C, b: C| if contrast(C(0.0, 0.0, 0.0, 1.0), a) < contrast(C(0.0, 0.0, 0.0, 1.0), b) { a } else { b };
    let bg = if t.dark { t.card } else { worst(t.mica_a, t.layer2) };
    let text = |l0: f32, need: f32| {
        let mut l = l0;
        for _ in 0..60 {
            let c = oklch(l, ct, h);
            if contrast(c, bg) >= need {
                return c;
            }
            l += if t.dark { 0.01 } else { -0.01 };
        }
        oklch(l, ct, h)
    };
    if t.dark {
        t.text = text(0.955, 7.0);
        t.text2 = text(0.80, 4.5);
        t.text3 = text(0.70, 4.5);
        t.text_dis = oklch(0.50, ct, h);
    } else {
        t.text = text(0.22, 7.0);
        t.text2 = text(0.45, 4.5);
        t.text3 = text(0.54, 4.5);
        t.text_dis = oklch(0.74, ct, h);
    }
}

/// The Windows accent colour (AccentPalette base), if set.
pub fn windows_accent_base() -> Option<C> {
    let p = reg_bin(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent", "AccentPalette")?;
    (p.len() >= 16).then(|| C::rgba(p[12], p[13], p[14], 1.0))
}

/// DWM's colorization colour (fast; used until the wallpaper tint is known).
pub fn dwm_tint() -> Option<Tint> {
    let (mut c, mut opaque) = (0u32, windows::core::BOOL(0));
    unsafe { windows::Win32::Graphics::Dwm::DwmGetColorizationColor(&mut c, &mut opaque) }.ok()?;
    Some(Tint::of(C::hex(c & 0xFFFFFF)))
}

/// The current wallpaper: (path, modified time). Empty path = a solid colour background.
pub fn wallpaper() -> (String, u64) {
    use windows::Win32::UI::WindowsAndMessaging::{SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW};
    let mut buf = [0u16; 520];
    let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr() as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_ok();
    let p = if ok { fmcore::util::from_wide(&buf) } else { String::new() };
    let m = std::fs::metadata(&p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    (p, m)
}

/// The wallpaper as a small grid of linear-light colours, plus its size (to place a window on it). Mica tints a
/// window with the part of the wallpaper behind it, so the tint follows the window's place on its monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub n: usize,
    pub grid: Vec<[f32; 3]>,
    pub img: (f32, f32),
}

const GRID: usize = 16;

impl Wall {
    /// The tint behind a window: `win` and `mon` are screen rectangles (left, top, right, bottom). The wallpaper is
    /// placed like Windows' default "Fill": scaled to cover the monitor, centred, cropped.
    pub fn tint_at(&self, win: (f32, f32, f32, f32), mon: (f32, f32, f32, f32)) -> Tint {
        let (mw, mh) = ((mon.2 - mon.0).max(1.0), (mon.3 - mon.1).max(1.0));
        let (iw, ih) = (self.img.0.max(1.0), self.img.1.max(1.0));
        let s = (mw / iw).max(mh / ih);
        let (vw, vh) = (mw / s / iw, mh / s / ih);
        let (cx, cy) = ((1.0 - vw) / 2.0, (1.0 - vh) / 2.0);
        let nx = |x: f32| cx + ((x - mon.0) / mw).clamp(0.0, 1.0) * vw;
        let ny = |y: f32| cy + ((y - mon.1) / mh).clamp(0.0, 1.0) * vh;
        let (u0, u1, v0, v1) = (nx(win.0), nx(win.2), ny(win.1), ny(win.3));
        let (mut r, mut g, mut b, mut k) = (0.0, 0.0, 0.0, 0.0);
        for j in 0..self.n {
            for i in 0..self.n {
                let (u, v) = ((i as f32 + 0.5) / self.n as f32, (j as f32 + 0.5) / self.n as f32);
                if u >= u0 - 0.5 / self.n as f32 && u <= u1 + 0.5 / self.n as f32 && v >= v0 - 0.5 / self.n as f32 && v <= v1 + 0.5 / self.n as f32 {
                    let c = self.grid[j * self.n + i];
                    r += c[0];
                    g += c[1];
                    b += c[2];
                    k += 1.0;
                }
            }
        }
        if k == 0.0 {
            for c in &self.grid {
                r += c[0];
                g += c[1];
                b += c[2];
                k += 1.0;
            }
        }
        let (_, c, h) = oklab_lch(r / k, g / k, b / k);
        Tint { hue: h, chroma: c }
    }
}

/// The cached wallpaper grid, when it still belongs to the current wallpaper (one small file; no image decode).
pub fn cached_wall() -> Option<Wall> {
    let (p, m) = wallpaper();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(fmcore::util::data_dir().join("tint.json")).ok()?).ok()?;
    if v["wallpaper"].as_str() != Some(p.as_str()) || v["mtime"].as_u64() != Some(m) {
        return None;
    }
    let n = v["n"].as_u64()? as usize;
    let flat: Vec<f32> = v["grid"].as_array()?.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
    if n == 0 || flat.len() != n * n * 3 {
        return None;
    }
    Some(Wall { n, grid: flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(), img: (v["w"].as_f64()? as f32, v["h"].as_f64()? as f32) })
}

/// Read the wallpaper into a 16 x 16 grid (WIC, scaled down), save it to tint.json. Off the UI thread. A solid colour
/// background gives a one-colour grid.
pub fn wallpaper_grid() -> Option<Wall> {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    let _c = fmcore::util::com();
    let (p, m) = wallpaper();
    let wall = if p.is_empty() {
        let c = unsafe { windows::Win32::Graphics::Gdi::GetSysColor(windows::Win32::Graphics::Gdi::COLOR_BACKGROUND) };
        let px = [to_lin(((c >> 16) & 255) as f32 / 255.0), to_lin(((c >> 8) & 255) as f32 / 255.0), to_lin((c & 255) as f32 / 255.0)];
        Wall { n: 1, grid: vec![[px[2], px[1], px[0]]], img: (1.0, 1.0) }
    } else {
        let w = fmcore::util::wide(&p);
        unsafe {
            let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let dec = f.CreateDecoderFromFilename(windows::core::PCWSTR(w.as_ptr()), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
            let frame = dec.GetFrame(0).ok()?;
            let (mut iw, mut ih) = (0u32, 0u32);
            frame.GetSize(&mut iw, &mut ih).ok()?;
            let sc = f.CreateBitmapScaler().ok()?;
            sc.Initialize(&frame, GRID as u32, GRID as u32, WICBitmapInterpolationModeFant).ok()?;
            let cv = f.CreateFormatConverter().ok()?;
            cv.Initialize(&sc, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
            let mut px = vec![0u8; GRID * GRID * 4];
            cv.CopyPixels(std::ptr::null(), (GRID * 4) as u32, &mut px).ok()?;
            let grid = px.chunks_exact(4).map(|q| [to_lin(q[2] as f32 / 255.0), to_lin(q[1] as f32 / 255.0), to_lin(q[0] as f32 / 255.0)]).collect();
            Wall { n: GRID, grid, img: (iw as f32, ih as f32) }
        }
    };
    let flat: Vec<f32> = wall.grid.iter().flat_map(|c| c.iter().map(|x| (x * 10000.0).round() / 10000.0)).collect();
    let _ = std::fs::create_dir_all(fmcore::util::data_dir());
    let _ = std::fs::write(
        fmcore::util::data_dir().join("tint.json"),
        serde_json::json!({"wallpaper": p, "mtime": m, "n": wall.n, "w": wall.img.0, "h": wall.img.1, "grid": flat}).to_string(),
    );
    Some(wall)
}
