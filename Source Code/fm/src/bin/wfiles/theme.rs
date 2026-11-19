//! Colour tokens from design-fm/design-notes.md (light and dark). The accent follows the Windows accent colour.
use crate::gfx::C;

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

/// Theme for `dark`, with the user's Windows accent (AccentPalette: light3, light2, light1, base, dark1, dark2, dark3).
pub fn make(dark_mode: bool) -> Theme {
    let mut t = if dark_mode { dark() } else { light() };
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
