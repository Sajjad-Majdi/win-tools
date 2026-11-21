//! Color tokens from design/v2/design-notes.md (light and dark) plus the system accent color.
use super::gfx::C;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub mica: C,
    pub layer: C,
    pub layer_s: C,
    pub card: C,
    pub card_s: C,
    pub fly: C,
    pub fly_s: C,
    pub hov: C,
    pub prs: C,
    pub sel: C,
    pub line: C,
    pub t1: C,
    pub t2: C,
    pub t3: C,
    pub acc: C,
    pub acc_h: C,
    pub on_acc: C,
    pub acc_t: C,
    pub acc_soft: C,
    pub ctl: C,
    pub ctl_s: C,
    pub ctl_b: C,
    pub ctl_h: C,
    pub inp: C,
    pub inp_s: C,
    pub inp_b: C,
    pub tag: C,
    pub warn: C,
    pub warn_bg: C,
    pub warn_s: C,
    pub warn_row: C,
    pub crit: C,
    pub crit_bg: C,
    pub crit_btn: C,
    pub on_crit: C,
    pub ok: C,
    pub ok_bg: C,
    pub cpu: C,
    pub mem: C,
    pub disk: C,
    pub net: C,
    pub gpu: C,
    pub track: C,
    pub smoke: C,
    pub code: C,
    pub foot: C,
    /// shadow strength for flyouts and dialogs
    pub sh: f32,
    /// panel alpha: how opaque the window, title bar, sidebar, content, cards and side pane are over the
    /// Mica / Acrylic backdrop (1 = opaque; Settings > Transparency)
    pub pa: f32,
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> C {
    C(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

pub const LIGHT: Theme = Theme {
    dark: false,
    mica: C::hex(0xEBEEF4),
    layer: C::hex(0xF7F9FC),
    layer_s: C::hex(0xE0E4EA),
    card: C::hex(0xFBFCFE),
    card_s: C::hex(0xE1E5EB),
    fly: C::hex(0xF9FAFC),
    fly_s: C::hex(0xD6DBE2),
    hov: C::hex(0xECEFF4),
    prs: C::hex(0xE5E9EF),
    sel: C::hex(0xE6ECF5),
    line: C::hex(0xE4E7EC),
    t1: C::hex(0x1A1E24),
    t2: C::hex(0x59626E),
    t3: C::hex(0x7C8591),
    acc: C::hex(0x005FB8),
    acc_h: C::hex(0x1A6EBE),
    on_acc: C::hex(0xF7FAFF),
    acc_t: C::hex(0x005FB8),
    acc_soft: rgba(0, 95, 184, 0.11),
    ctl: C::hex(0xFAFBFD),
    ctl_s: C::hex(0xDADEE4),
    ctl_b: C::hex(0xC5CAD2),
    ctl_h: C::hex(0xF2F4F7),
    inp: C::hex(0xFAFBFD),
    inp_s: C::hex(0xD7DBE1),
    inp_b: C::hex(0x878D96),
    tag: C::hex(0xEAEDF2),
    warn: C::hex(0x8F5600),
    warn_bg: C::hex(0xFBF1DA),
    warn_s: C::hex(0xEEDDB2),
    warn_row: C::hex(0xFCF5E4),
    crit: C::hex(0xC42B1C),
    crit_bg: C::hex(0xFBE8E6),
    crit_btn: C::hex(0xC42B1C),
    on_crit: C::hex(0xFFF7F6),
    ok: C::hex(0x107C10),
    ok_bg: C::hex(0xE2F1E0),
    cpu: C::hex(0x2C7BD6),
    mem: C::hex(0x8A5BD2),
    disk: C::hex(0x25916A),
    net: C::hex(0xC07216),
    gpu: C::hex(0xC24F84),
    track: C::hex(0xE2E6EB),
    smoke: rgba(24, 30, 40, 0.30),
    code: C::hex(0xF0F3F7),
    foot: C::hex(0xF0F2F6),
    sh: 1.0,
    pa: 1.0,
};

pub const DARK: Theme = Theme {
    dark: true,
    mica: C::hex(0x1B1E25),
    layer: C::hex(0x23262D),
    layer_s: C::hex(0x30343C),
    card: C::hex(0x2A2E36),
    card_s: C::hex(0x363B44),
    fly: C::hex(0x2B2F37),
    fly_s: C::hex(0x40454F),
    hov: C::hex(0x2F333B),
    prs: C::hex(0x2A2D34),
    sel: C::hex(0x313744),
    line: C::hex(0x33373F),
    t1: C::hex(0xF0F2F5),
    t2: C::hex(0xBAC1CA),
    t3: C::hex(0x8C939D),
    acc: C::hex(0x60CDFF),
    acc_h: C::hex(0x7AD5FF),
    on_acc: C::hex(0x06202D),
    acc_t: C::hex(0x7AD5FF),
    acc_soft: rgba(96, 205, 255, 0.15),
    ctl: C::hex(0x30343C),
    ctl_s: C::hex(0x3A3F48),
    ctl_b: C::hex(0x343840),
    ctl_h: C::hex(0x363A43),
    inp: C::hex(0x1F2228),
    inp_s: C::hex(0x3A3F48),
    inp_b: C::hex(0x9AA1AA),
    tag: C::hex(0x343841),
    warn: C::hex(0xF2C14E),
    warn_bg: C::hex(0x3A3122),
    warn_s: C::hex(0x544429),
    warn_row: C::hex(0x302B22),
    crit: C::hex(0xFF99A4),
    crit_bg: C::hex(0x45282C),
    crit_btn: C::hex(0xFF99A4),
    on_crit: C::hex(0x2A0A0E),
    ok: C::hex(0x6CCB5F),
    ok_bg: C::hex(0x25372A),
    cpu: C::hex(0x66B2FF),
    mem: C::hex(0xB796F5),
    disk: C::hex(0x5CC796),
    net: C::hex(0xE5A257),
    gpu: C::hex(0xEE8CB6),
    track: C::hex(0x3A3E47),
    smoke: rgba(0, 0, 0, 0.48),
    code: C::hex(0x1F2228),
    foot: C::hex(0x262930),
    sh: 2.9,
    pa: 1.0,
};

/// Windows app theme: HKCU ...\Themes\Personalize AppsUseLightTheme (missing = light).
pub fn system_dark() -> bool {
    let v = crate::reg::read_dword(crate::reg::HKCU, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme");
    v == Some(0)
}

/// The system accent palette (Explorer\Accent\AccentPalette: 8 RGBA entries, light3 .. dark3).
/// Returns (dark1, light2, light3) or None.
fn accent_palette() -> Option<(C, C, C)> {
    let b = crate::reg::read_binary(crate::reg::HKCU, r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent", "AccentPalette")?;
    if b.len() < 32 {
        return None;
    }
    let at = |i: usize| C(b[i * 4] as f32 / 255.0, b[i * 4 + 1] as f32 / 255.0, b[i * 4 + 2] as f32 / 255.0, 1.0);
    Some((at(4), at(1), at(0)))
}

fn mix(a: C, b: C, f: f32) -> C {
    C(a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, a.2 + (b.2 - a.2) * f, 1.0)
}

fn luma(c: C) -> f32 {
    0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
}

/// Panel alpha for Settings > Transparency: Off, Low, Medium, High.
pub const PANEL_ALPHA: [f32; 4] = [1.0, 0.75, 0.5, 0.3];

/// Accent presets offered in Settings (base colours; light and dark shades are derived).
pub const PRESETS: [(&str, u32); 8] = [
    ("Blue", 0x2563EB),
    ("Purple", 0x8764B8),
    ("Pink", 0xC239B3),
    ("Red", 0xD13438),
    ("Orange", 0xCA5010),
    ("Gold", 0xB8860B),
    ("Green", 0x107C10),
    ("Teal", 0x038387),
];

fn to_hsl(c: C) -> (f32, f32, f32) {
    let (r, g, b) = (c.0, c.1, c.2);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx - mn < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r { (g - b) / d + if g < b { 6.0 } else { 0.0 } } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h * 60.0, s, l)
}

pub fn from_hsl(h: f32, s: f32, l: f32) -> C {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    C(r + m, g + m, b + m, 1.0)
}

/// The old Blue preset (an azure that read as sky blue); saved settings that still hold it get today's Blue.
pub const OLD_BLUE: u32 = 0x0078D4;

/// Accent picker: where a colour sits in the colour field (hue 0..360, colourfulness 0..1).
pub fn hs_of(rgb: u32) -> (f32, f32) {
    let (h, s, _) = to_hsl(C::hex(rgb));
    (h, s)
}

/// Accent picker: the colour at a place in the colour field (what `Gfx::spectrum` draws there).
pub fn pick_rgb(h: f32, s: f32) -> u32 {
    let c = from_hsl(h, s.clamp(0.0, 1.0), 0.5);
    ((c.0 * 255.0).round() as u32) << 16 | ((c.1 * 255.0).round() as u32) << 8 | (c.2 * 255.0).round() as u32
}

/// Hue of an RGB colour (0..360), for the custom accent picker.
pub fn hue_of(rgb: u32) -> f32 {
    to_hsl(C::hex(rgb)).0
}

/// The accent shades for one base colour: (light-theme accent, dark-theme accent). Lightness is fixed so text
/// on the accent and the accent on the window stay readable whatever colour is picked.
pub fn accent_pair(rgb: u32) -> (C, C) {
    let (h, s, _) = to_hsl(C::hex(rgb));
    let s2 = s.min(0.92);
    // blues turn pale (periwinkle) at the shared dark-theme lightness: they go a little deeper, so they still read as blue
    let deep = 0.07 * (1.0 - (h - 230.0).abs() / 45.0).max(0.0);
    (from_hsl(h, s2.max(0.35), 0.37), from_hsl(h, s2.max(0.45), 0.72 - deep))
}

pub fn hex(c: C) -> String {
    format!("#{:02X}{:02X}{:02X}", (c.0 * 255.0).round() as u8, (c.1 * 255.0).round() as u8, (c.2 * 255.0).round() as u8)
}

/// The theme for light/dark, with the accent (`None` = the system accent) and the surface transparency
/// (0 off .. 3 high) applied. The defaults keep the exact design values.
pub fn make(dark: bool, accent: Option<u32>, transp: u8) -> Theme {
    make_toned(dark, accent, transp, None)
}

/// Like `make`, with every neutral surface re-toned to one base hue (`base` = hue in degrees, chroma 0..1): one
/// tonal palette for the whole window, steps differing mainly in lightness. Text is then checked for WCAG AA.
pub fn make_toned(dark: bool, accent: Option<u32>, transp: u8, base: Option<(f32, f32)>) -> Theme {
    let mut t = make_sys(dark);
    if let Some(rgb) = accent {
        let (l, d) = accent_pair(rgb);
        let a = if dark { d } else { l };
        t.acc = a;
        t.acc_t = if dark { mix(a, C::hex(0xFFFFFF), 0.15) } else { a };
        t.acc_h = if dark { mix(a, C::hex(0xFFFFFF), 0.12) } else { mix(a, C::hex(0xFFFFFF), 0.1) };
        t.on_acc = if dark { mix(a, C::hex(0x000000), 0.85) } else { C::hex(0xF7FAFF) };
        t.acc_soft = t.acc.a(if dark { 0.15 } else { 0.11 });
    }
    if let Some((hue, chroma)) = base {
        tone(&mut t, hue, chroma);
    }
    if transp > 0 {
        // every panel (window, title bar, sidebar, content, cards, side pane) is the palette colour at one alpha,
        // so the backdrop shows through evenly; text stays opaque. Hover / press / selection become tints that
        // work on any surface. The window paints panels with a copy blend, so nested panels never add up.
        let pa = PANEL_ALPHA[transp.min(3) as usize];
        t.pa = pa;
        for x in [&mut t.mica, &mut t.layer, &mut t.card, &mut t.foot, &mut t.code] {
            *x = x.a(pa);
        }
        let ink = if dark { C(1.0, 1.0, 1.0, 1.0) } else { C(0.0, 0.0, 0.0, 1.0) };
        // borders: a tint, not an opaque grey, so they read on a see-through panel
        t.layer_s = ink.a(if dark { 0.09 } else { 0.08 });
        t.card_s = ink.a(if dark { 0.10 } else { 0.09 });
        t.line = ink.a(if dark { 0.08 } else { 0.07 });
        t.hov = ink.a(if dark { 0.06 } else { 0.045 });
        t.prs = ink.a(if dark { 0.035 } else { 0.03 });
        t.sel = t.acc.a(if dark { 0.16 } else { 0.10 });
        t.tag = ink.a(if dark { 0.08 } else { 0.06 });
        t.track = ink.a(if dark { 0.13 } else { 0.10 });
    }
    t
}

/// Light/dark with the system accent applied (default blue keeps the exact design values).
fn make_sys(dark: bool) -> Theme {
    let mut t = if dark { DARK } else { LIGHT };
    if let Some((dark1, light2, light3)) = accent_palette() {
        // the stock blue palette stays on the hand-tuned design values
        let stock = (dark1.0 * 255.0).round() as u8 == 0 && ((dark1.2 * 255.0).round() as i32 - 0xB8).abs() <= 4;
        if !stock {
            if dark {
                t.acc = light2;
                t.acc_h = mix(light2, light3, 0.5);
                t.acc_t = mix(light2, light3, 0.5);
                t.on_acc = if luma(light2) > 0.45 { C::hex(0x06202D) } else { C::hex(0xF7FAFF) };
            } else {
                t.acc = dark1;
                t.acc_h = mix(dark1, C::hex(0xFFFFFF), 0.1);
                t.acc_t = dark1;
                t.on_acc = if luma(dark1) > 0.5 { C::hex(0x1A1E24) } else { C::hex(0xF7FAFF) };
            }
            t.acc_soft = t.acc.a(if dark { 0.15 } else { 0.11 });
        }
    }
    t
}

/// The system accent as a base colour (for the "System" swatch).
pub fn system_accent(dark: bool) -> C {
    make_sys(dark).acc
}

/// Corner radius for a drawn shape. The design values (4 controls, 8 cards and flyouts) are the inputs; the
/// "Rounded" look is rounder than Windows: controls, inputs, pills and row highlights 4 -> 8, cards, panels,
/// flyouts, menus and the dialog 8 -> 12. Hairlines (< 3) and half-height pills (> 8) keep their radius.
/// "Square" draws every corner square.
pub fn radius(r: f32, rounded: bool) -> f32 {
    if !rounded {
        return 0.0;
    }
    if r < 3.0 {
        r
    } else if r <= 4.5 {
        r * 2.0
    } else if r <= 8.0 {
        r + 4.0
    } else {
        r
    }
}

/// Re-tone the neutral tokens to `hue` with low chroma (clamped), keeping each token's lightness, then make the
/// text colours meet WCAG AA (4.5:1) on every surface.
pub fn tone(t: &mut Theme, hue: f32, chroma: f32) {
    // `chroma` is absolute (0..1 of the full colour range), so light and dark tones get the same visible tint:
    // near-white and near-black tokens have little room for colour, so they take a higher saturation
    let c = chroma.clamp(0.0, 0.14);
    let re = |x: C, k: f32| -> C {
        let (_, _, l) = to_hsl(x);
        let room = 1.0 - (2.0 * l - 1.0).abs();
        let sat = if room > 1e-3 { (c * k / room).min(1.0) } else { 0.0 };
        from_hsl(hue, sat, l).a(x.3)
    };
    for x in [&mut t.mica, &mut t.layer, &mut t.card, &mut t.fly, &mut t.hov, &mut t.prs, &mut t.ctl, &mut t.ctl_h, &mut t.inp, &mut t.code, &mut t.foot, &mut t.tag, &mut t.track] {
        *x = re(*x, 1.0);
    }
    for x in [&mut t.layer_s, &mut t.card_s, &mut t.fly_s, &mut t.line, &mut t.ctl_s, &mut t.ctl_b, &mut t.inp_s, &mut t.inp_b] {
        *x = re(*x, 0.8);
    }
    t.sel = re(t.sel, 1.6);
    for x in [&mut t.t1, &mut t.t2, &mut t.t3] {
        *x = re(*x, 0.35);
    }
    let surfaces = [t.mica, t.layer, t.card, t.fly, t.hov, t.sel];
    t.t1 = readable(t.t1, &surfaces, 7.0, t.dark);
    t.t2 = readable(t.t2, &surfaces, 4.5, t.dark);
    t.t3 = readable(t.t3, &surfaces, 4.5, t.dark);
}

fn lum(c: C) -> f32 {
    let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
}

/// WCAG contrast ratio of two opaque colours.
pub fn contrast(a: C, b: C) -> f32 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Darken (light theme) or lighten (dark theme) `fg` until it reaches `ratio` on every surface.
fn readable(fg: C, on: &[C], ratio: f32, dark: bool) -> C {
    let (h, s, mut l) = to_hsl(fg);
    let mut c = fg;
    for _ in 0..60 {
        if on.iter().all(|b| contrast(c, b.a(1.0)) >= ratio) {
            break;
        }
        l = if dark { (l + 0.01).min(1.0) } else { (l - 0.01).max(0.0) };
        c = from_hsl(h, s, l);
    }
    c.a(fg.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_sets_one_panel_alpha_and_keeps_text_opaque() {
        for (tr, want) in [(0u8, 1.0f32), (1, 0.75), (2, 0.5), (3, 0.3)] {
            let t = make_toned(true, None, tr, Some((25.0, 0.14)));
            assert_eq!(t.pa, want);
            for s in [t.mica, t.layer, t.card] {
                assert!((s.3 - want).abs() < 1e-6);
            }
            assert!(t.t1.3 == 1.0 && t.t2.3 == 1.0 && t.fly.3 == 1.0);
        }
    }

    #[test]
    fn toned_palettes_keep_text_readable() {
        for dark in [false, true] {
            for hue in [25.0f32, 210.0, 120.0, 300.0] {
                let t = make_toned(dark, None, 0, Some((hue, 0.3)));
                for s in [t.mica, t.layer, t.card, t.fly, t.sel] {
                    assert!(contrast(t.t2, s) >= 4.5 && contrast(t.t3, s) >= 4.5, "dark {dark} hue {hue}");
                }
                // one hue: the window background and the cards share it
                let (h1, _, _) = to_hsl(t.mica);
                let (h2, _, _) = to_hsl(t.card);
                assert!((h1 - h2).abs() < 2.0);
            }
        }
    }
}
