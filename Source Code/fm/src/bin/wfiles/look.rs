//! Settings > Personalize: how the window looks. Kept in its own file, `look.json`, next to fm.json
//! (%LOCALAPPDATA%\WinTools, or WFM_DATA_DIR). Every value applies live.
use serde_json::{Value, json};

pub const BACKDROPS: [&str; 4] = ["Mica", "Mica Alt", "Acrylic", "Solid"];
pub const TRANSPARENCY: [&str; 4] = ["Off", "Low", "Medium", "High"];
pub const THEMES: [&str; 3] = ["Same as Windows", "Light", "Dark"];
pub const DENSITY: [&str; 2] = ["Comfortable", "Compact"];
pub const SIZES3: [&str; 3] = ["Small", "Medium", "Large"];
pub const CORNERS: [&str; 2] = ["Rounded", "Square"];
/// Accent presets (name, colour). The first entry of the picker is "Windows accent".
pub const ACCENTS: [(&str, u32); 9] = [
    ("Blue", 0x0067C0),
    ("Teal", 0x00897B),
    ("Green", 0x2E8B3A),
    ("Purple", 0x7B4FC9),
    ("Pink", 0xC2389A),
    ("Red", 0xC8323C),
    ("Orange", 0xD9661A),
    ("Gold", 0xB8860B),
    ("Graphite", 0x5D6B7E),
];
/// sidebar width presets (the edge can also be dragged)
pub const SIDEBAR_WIDTHS: [(&str, f32); 5] = [("Icons only", 0.0), ("Narrow", 200.0), ("Default", 232.0), ("Wide", 280.0), ("Wider", 330.0)];
pub const SIDE_MIN: f32 = 180.0;
pub const SIDE_MAX: f32 = 380.0;
/// column width limits; the Name column keeps at least NAME_MIN
pub const COL_MIN: f32 = 56.0;
pub const COL_MAX: f32 = 480.0;
pub const NAME_MIN: f32 = 120.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// 0 Mica, 1 Mica Alt, 2 Acrylic, 3 Solid
    pub backdrop: u8,
    /// 0 Off (opaque sidebar), 1 Low (the approved look), 2 Medium, 3 High
    pub transparency: u8,
    /// 0 system, 1 light, 2 dark
    pub theme: u8,
    /// None = the Windows accent
    pub accent: Option<u32>,
    pub density: u8,
    pub font: u8,
    pub icons: u8,
    /// sidebar width; 0 = icons only (the rail)
    pub side_w: f32,
    pub square: bool,
    pub show_pinned: bool,
    pub show_drives: bool,
    pub show_recent: bool,
    pub col_date: bool,
    pub col_size: bool,
    pub col_type: bool,
    /// column widths (drag the header dividers; double-click fits)
    pub w_date: f32,
    pub w_size: f32,
    pub w_type: f32,
    /// "thispc", "last" or a folder path
    pub start: String,
    /// the folder open when the window last closed
    pub last: String,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            backdrop: 0,
            transparency: 1,
            theme: 0,
            accent: None,
            density: 0,
            font: 1,
            icons: 1,
            side_w: 232.0,
            square: false,
            show_pinned: true,
            show_drives: true,
            show_recent: true,
            col_date: true,
            col_size: true,
            col_type: false,
            w_date: 150.0,
            w_size: 92.0,
            w_type: 130.0,
            start: "thispc".into(),
            last: String::new(),
        }
    }
}

fn path() -> std::path::PathBuf {
    fmcore::util::data_dir().join("look.json")
}

impl Look {
    /// Read look.json; the theme falls back to the older fm.json "ui.theme".
    pub fn load() -> Look {
        let d = Look::default();
        let v: Value = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let u8_ = |k: &str, def: u8, max: u8| v[k].as_u64().map(|x| (x as u8).min(max)).unwrap_or(def);
        let b = |k: &str, def: bool| v[k].as_bool().unwrap_or(def);
        let theme = if v["theme"].is_u64() { u8_("theme", 0, 2) } else { fmcore::pins::config()["ui"]["theme"].as_u64().map(|x| (x as u8).min(2)).unwrap_or(0) };
        Look {
            backdrop: u8_("backdrop", d.backdrop, 3),
            transparency: u8_("transparency", d.transparency, 3),
            theme,
            accent: v["accent"].as_str().and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok()).filter(|c| *c <= 0xFFFFFF),
            density: u8_("density", d.density, 1),
            font: u8_("font", d.font, 2),
            icons: u8_("icons", d.icons, 2),
            side_w: v["side_width"].as_f64().map(|x| if x < 1.0 { 0.0 } else { (x as f32).clamp(SIDE_MIN, SIDE_MAX) }).unwrap_or(d.side_w),
            square: v["corners"].as_str() == Some("square"),
            show_pinned: b("show_pinned", true),
            show_drives: b("show_drives", true),
            show_recent: b("show_recent", true),
            col_date: b("column_date", true),
            col_size: b("column_size", true),
            col_type: b("column_type", false),
            w_date: v["width_date"].as_f64().map(|x| (x as f32).clamp(COL_MIN, COL_MAX)).unwrap_or(d.w_date),
            w_size: v["width_size"].as_f64().map(|x| (x as f32).clamp(COL_MIN, COL_MAX)).unwrap_or(d.w_size),
            w_type: v["width_type"].as_f64().map(|x| (x as f32).clamp(COL_MIN, COL_MAX)).unwrap_or(d.w_type),
            start: v["start"].as_str().filter(|s| !s.is_empty()).unwrap_or("thispc").to_string(),
            last: v["last"].as_str().unwrap_or("").to_string(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "backdrop": self.backdrop, "transparency": self.transparency, "theme": self.theme,
            "accent": self.accent.map(|c| format!("#{c:06X}")),
            "density": self.density, "font": self.font, "icons": self.icons, "side_width": self.side_w.round(),
            "corners": if self.square { "square" } else { "rounded" },
            "show_pinned": self.show_pinned, "show_drives": self.show_drives, "show_recent": self.show_recent,
            "column_date": self.col_date, "column_size": self.col_size, "column_type": self.col_type,
            "width_date": self.w_date.round(), "width_size": self.w_size.round(), "width_type": self.w_type.round(),
            "start": self.start, "last": self.last,
        })
    }

    pub fn save(&self) {
        let p = path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(&self.to_json()).unwrap()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }

    /// List row height for the density.
    pub fn row_h(&self) -> f32 {
        if self.density == 1 { 26.0 } else { 32.0 }
    }
    pub fn list_row_h(&self) -> f32 {
        if self.density == 1 { 22.0 } else { 26.0 }
    }
    /// List text size for the font setting.
    pub fn text(&self) -> f32 {
        [12.0, 13.0, 14.5][self.font as usize % 3]
    }
    pub fn small_text(&self) -> f32 {
        [11.5, 12.0, 13.0][self.font as usize % 3]
    }
    /// Details-view icon size.
    pub fn icon(&self) -> f32 {
        [16.0, 20.0, 24.0][self.icons as usize % 3]
    }
    /// DWMWA_SYSTEMBACKDROP_TYPE value: 2 Mica, 4 Mica Alt (tabbed), 3 Acrylic (transient), 1 none.
    pub fn dwm_backdrop(&self) -> i32 {
        [2, 4, 3, 1][self.backdrop as usize % 4]
    }
    /// The backdrop shows through the sidebar and title bar.
    pub fn see_through(&self) -> bool {
        self.backdrop != 3 && self.transparency > 0
    }
    /// Opacity of the bars and content layers over the backdrop.
    pub fn layer_alpha(&self) -> f32 {
        if self.backdrop == 3 { 1.0 } else { [1.0, 1.0, 0.86, 0.72][self.transparency as usize % 4] }
    }

    /// Set one value by name (for `wfm gui look KEY VALUE` and tests). Returns false for an unknown key or value.
    pub fn set(&mut self, key: &str, val: &str) -> bool {
        let pick = |list: &[&str], v: &str| list.iter().position(|x| x.eq_ignore_ascii_case(v) || x.replace(' ', "-").eq_ignore_ascii_case(v)).or_else(|| v.parse::<usize>().ok().filter(|i| *i < list.len()));
        let on = |v: &str| matches!(v, "on" | "true" | "1" | "show" | "yes");
        match key {
            "backdrop" => self.backdrop = pick(&BACKDROPS, val).map(|x| x as u8).unwrap_or(self.backdrop),
            "transparency" => self.transparency = pick(&TRANSPARENCY, val).map(|x| x as u8).unwrap_or(self.transparency),
            "theme" => self.theme = match val { "system" => 0, "light" => 1, "dark" => 2, _ => return false },
            "accent" => {
                self.accent = if val == "system" || val == "windows" {
                    None
                } else if let Some((_, c)) = ACCENTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(val)) {
                    Some(*c)
                } else {
                    match u32::from_str_radix(val.trim_start_matches('#'), 16) {
                        Ok(c) if val.trim_start_matches('#').len() == 6 => Some(c),
                        _ => return false,
                    }
                }
            }
            "density" => self.density = pick(&DENSITY, val).map(|x| x as u8).unwrap_or(self.density),
            "font" => self.font = pick(&SIZES3, val).map(|x| x as u8).unwrap_or(self.font),
            "icons" => self.icons = pick(&SIZES3, val).map(|x| x as u8).unwrap_or(self.icons),
            "sidebar" | "side_width" => self.side_w = val.parse::<f32>().map(|w| if w < 1.0 { 0.0 } else { w.clamp(SIDE_MIN, SIDE_MAX) }).unwrap_or(self.side_w),
            "corners" => self.square = val == "square",
            "pinned" => self.show_pinned = on(val),
            "drives" => self.show_drives = on(val),
            "recent" => self.show_recent = on(val),
            "date" => self.col_date = on(val),
            "size" => self.col_size = on(val),
            "type" => self.col_type = on(val),
            "start" => self.start = val.to_string(),
            _ => return false,
        }
        true
    }
}
