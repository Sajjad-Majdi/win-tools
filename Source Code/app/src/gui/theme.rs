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

/// The theme for light/dark with the system accent applied (default blue keeps the exact design values).
pub fn make(dark: bool) -> Theme {
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
