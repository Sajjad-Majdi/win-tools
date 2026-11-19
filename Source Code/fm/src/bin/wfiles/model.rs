//! What the window shows: locations, items, panes (with history, selection, search), tabs.
use crate::ficons::{Kind, kind_of};
use crate::fmt::Field;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum Loc {
    Folder(PathBuf),
    ThisPc,
    RecycleBin,
    Settings,
}

impl Loc {
    pub fn title(&self) -> String {
        match self {
            Loc::Folder(p) => folder_title(p),
            Loc::ThisPc => "This PC".into(),
            Loc::RecycleBin => "Recycle Bin".into(),
            Loc::Settings => "Settings".into(),
        }
    }
    pub fn path(&self) -> Option<&Path> {
        match self {
            Loc::Folder(p) => Some(p),
            _ => None,
        }
    }
    pub fn display(&self) -> String {
        match self {
            Loc::Folder(p) => p.display().to_string(),
            other => other.title(),
        }
    }
}

pub fn folder_title(p: &Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => {
            let s = p.to_string_lossy();
            let letter = s.trim_end_matches('\\').to_string();
            drive_label(&letter)
        }
    }
}

/// "Local Disk (C:)" style name for a drive root ("C:").
pub fn drive_label(letter: &str) -> String {
    let root = format!("{}\\", letter.trim_end_matches('\\'));
    let w = fmcore::util::wide(&root);
    let mut name = [0u16; 128];
    let ok = unsafe { windows::Win32::Storage::FileSystem::GetVolumeInformationW(windows::core::PCWSTR(w.as_ptr()), Some(&mut name), None, None, None, None) }.is_ok();
    let label = if ok { String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)]) } else { String::new() };
    let l = letter.trim_end_matches('\\');
    if label.is_empty() { format!("Local Disk ({l})") } else { format!("{label} ({l})") }
}

#[derive(Clone, Debug)]
pub struct Item {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: u64,
    pub attrs: u32,
    pub kind: Kind,
    /// search results: the folder shown under the name
    pub folder: String,
    /// search results: UTF-16 ranges of the matched words in the name
    pub hl: Vec<(u32, u32)>,
    /// recycle bin: the record (for restore)
    pub recycled: Option<usize>,
}

impl Item {
    pub fn from_entry(dir: &Path, e: &fmcore::list::Entry) -> Item {
        let is_dir = e.is_dir();
        Item {
            kind: kind_of(&e.name, is_dir),
            path: dir.join(&e.name),
            name: e.name.clone(),
            is_dir,
            size: if is_dir { None } else { Some(e.size) },
            modified: e.modified,
            attrs: e.attrs,
            folder: String::new(),
            hl: vec![],
            recycled: None,
        }
    }
    pub fn hidden(&self) -> bool {
        self.attrs & fmcore::util::ATTR_HIDDEN != 0
    }
    pub fn ext(&self) -> String {
        fmcore::util::ext_of(&self.name)
    }
    pub fn is_archive(&self) -> bool {
        matches!(self.ext().as_str(), "zip" | "tar" | "tgz" | "gz" | "xz" | "7z" | "rar" | "bz2")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortCol {
    Name,
    Date,
    Size,
    Type,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Details,
    List,
    Grid,
}

impl View {
    pub fn name(&self) -> &'static str {
        match self {
            View::Details => "details",
            View::List => "list",
            View::Grid => "grid",
        }
    }
}

/// Search state of a pane (the omnibox is in search mode).
#[derive(Clone, Debug)]
pub struct Search {
    pub field: Field,
    pub pc: bool,
    pub ftype: usize,
    pub fsize: usize,
    pub fdate: usize,
    pub items: Vec<Item>,
    pub total: u32,
    pub seq: u64,
    pub busy: bool,
    pub meaning: bool,
    pub note: String,
}

pub const TYPES: [(&str, &str); 8] = [
    ("Any", ""),
    ("Folders", "folder:"),
    ("Documents", "ext:pdf;doc;docx;txt;md;rtf;odt;xls;xlsx;csv;ppt;pptx"),
    ("Images", "ext:png;jpg;jpeg;gif;webp;bmp;heic;svg;ico"),
    ("Video", "ext:mp4;mkv;mov;avi;webm;wmv"),
    ("Audio", "ext:mp3;wav;flac;m4a;ogg;opus;aac"),
    ("Archives", "ext:zip;rar;7z;tar;gz;tgz;xz;bz2;whl;iso"),
    ("Code", "ext:rs;py;js;ts;tsx;json;jsonl;toml;yml;yaml;c;cpp;h;cs;java;kt;go;html;css;ps1;sh;ipynb"),
];
pub const SIZES: [(&str, &str); 5] = [("Any", ""), ("Under 1 MB", "size:<1mb"), ("1 - 100 MB", "size:1mb..100mb"), ("Over 100 MB", "size:>100mb"), ("Over 1 GB", "size:>1gb")];
pub const DATES: [(&str, &str); 6] = [("Any", ""), ("Today", "dm:today"), ("This week", "dm:thisweek"), ("This month", "dm:thismonth"), ("This year", "dm:thisyear"), ("Older", "dm:<thisyear")];

impl Search {
    pub fn new(text: &str, pc: bool) -> Search {
        Search { field: Field::new(text), pc, ftype: 0, fsize: 0, fdate: 0, items: vec![], total: 0, seq: 0, busy: false, meaning: false, note: String::new() }
    }
    /// The Everything query for the current words, scope and filters.
    pub fn query(&self, here: Option<&Path>) -> String {
        let mut q = String::new();
        if !self.pc
            && let Some(h) = here
        {
            q += &fmcore::evipc::quote(&format!("{}\\", h.to_string_lossy().trim_end_matches('\\')));
            q.push(' ');
        }
        q += self.field.text.trim();
        for f in [TYPES[self.ftype].1, SIZES[self.fsize].1, DATES[self.fdate].1] {
            if !f.is_empty() {
                q.push(' ');
                q += f;
            }
        }
        q
    }
}

pub struct Pane {
    pub loc: Loc,
    pub items: Vec<Item>,
    pub sel: HashSet<PathBuf>,
    pub focus: Option<usize>,
    pub anchor: Option<usize>,
    pub scroll: f32,
    pub sort: (SortCol, bool),
    pub back: Vec<Loc>,
    pub fwd: Vec<Loc>,
    pub search: Option<Search>,
    pub seq: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub shown_at: Instant,
    pub back_anim: bool,
    pub watch: Option<fmcore::watch::Watcher>,
    pub hidden_count: usize,
    /// keyboard focus ring visible (set by arrow keys)
    pub kb: bool,
    /// item to select once the listing arrives (after "up", new folder, rename)
    pub want: Option<PathBuf>,
    /// smooth / momentum scrolling (follows its target), and the value it last wrote to `scroll`
    pub sf: crate::motion::Follow,
    pub sf_written: f32,
    /// selection as last painted, newly selected rows (fade in), a single selection's slide (from rect, when)
    pub sel_seen: HashSet<PathBuf>,
    pub sel_fade: std::collections::HashMap<PathBuf, Instant>,
    pub slide: Option<(crate::gfx::Rect, Instant)>,
}

impl Pane {
    pub fn new(loc: Loc) -> Pane {
        Pane {
            loc,
            items: vec![],
            sel: HashSet::new(),
            focus: None,
            anchor: None,
            scroll: 0.0,
            sort: (SortCol::Date, true),
            back: vec![],
            fwd: vec![],
            search: None,
            seq: 0,
            loading: false,
            error: None,
            shown_at: Instant::now() - std::time::Duration::from_secs(5),
            back_anim: false,
            watch: None,
            hidden_count: 0,
            kb: false,
            want: None,
            sf: crate::motion::Follow::default(),
            sf_written: 0.0,
            sel_seen: HashSet::new(),
            sel_fade: std::collections::HashMap::new(),
            slide: None,
        }
    }
    /// The rows currently shown: search results in search mode, else the folder.
    pub fn rows(&self) -> &Vec<Item> {
        match &self.search {
            Some(s) if !s.field.text.trim().is_empty() => &s.items,
            _ => &self.items,
        }
    }
    pub fn searching(&self) -> bool {
        self.search.as_ref().map(|s| !s.field.text.trim().is_empty()).unwrap_or(false)
    }
    pub fn selected(&self) -> Vec<Item> {
        self.rows().iter().filter(|i| self.sel.contains(&i.path)).cloned().collect()
    }
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.rows().iter().filter(|i| self.sel.contains(&i.path)).map(|i| i.path.clone()).collect()
    }
    pub fn dir(&self) -> Option<PathBuf> {
        self.loc.path().map(|p| p.to_path_buf())
    }
    pub fn sort_items(&mut self) {
        // This PC keeps its own order: drives by letter, then the pinned folders as in the sidebar
        if self.loc == Loc::ThisPc {
            return;
        }
        let (col, desc) = self.sort;
        let cmp = |a: &Item, b: &Item| {
            // folders before files always (reviewer fix), then the column
            let d = b.is_dir.cmp(&a.is_dir);
            if d != std::cmp::Ordering::Equal {
                return d;
            }
            let o = match col {
                SortCol::Name => fmcore::list::natural_cmp(&a.name, &b.name),
                SortCol::Date => a.modified.cmp(&b.modified),
                SortCol::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
                SortCol::Type => a.ext().cmp(&b.ext()),
            };
            let o = if o == std::cmp::Ordering::Equal { fmcore::list::natural_cmp(&a.name, &b.name) } else { o };
            if desc { o.reverse() } else { o }
        };
        self.items.sort_by(cmp);
    }
}

pub struct Tab {
    pub id: u64,
    pub panes: Vec<Pane>,
    pub active: usize,
    pub view: View,
    pub split_at: Instant,
}

impl Tab {
    pub fn pane(&self) -> &Pane {
        &self.panes[self.active.min(self.panes.len() - 1)]
    }
    pub fn pane_mut(&mut self) -> &mut Pane {
        let i = self.active.min(self.panes.len() - 1);
        &mut self.panes[i]
    }
    pub fn split(&self) -> bool {
        self.panes.len() > 1
    }
    pub fn title(&self) -> String {
        self.pane().loc.title()
    }
}

/// Recycle Bin entries as items.
pub fn recycle_items() -> (Vec<Item>, Vec<fmcore::recycle::Recycled>) {
    let recs = fmcore::recycle::list_all();
    let items = recs
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let name = r.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let md = std::fs::metadata(&r.data).ok();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            Item {
                kind: kind_of(&name, is_dir),
                name,
                path: r.data.clone(),
                is_dir,
                size: if is_dir { None } else { Some(fmcore::recycle::record_size(&r.record)) },
                modified: r.deleted,
                attrs: 0,
                folder: r.original.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                hl: vec![],
                recycled: Some(i),
            }
        })
        .collect();
    (items, recs)
}

/// Words of the search typed text matched inside `name` (case-insensitive), as UTF-16 ranges.
pub fn match_ranges(name: &str, words: &str) -> Vec<(u32, u32)> {
    let lower = name.to_lowercase();
    let mut out = vec![];
    for w in words.split_whitespace() {
        if w.contains(':') {
            continue;
        }
        let wl = w.to_lowercase().trim_matches('*').to_string();
        if wl.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(i) = lower[from..].find(&wl) {
            let b = from + i;
            if !lower.is_char_boundary(b) || lower.len() != name.len() {
                break;
            }
            let s16 = name[..b].encode_utf16().count() as u32;
            let l16 = name[b..b + wl.len()].encode_utf16().count() as u32;
            out.push((s16, l16));
            from = b + wl.len().max(1);
        }
    }
    out
}

/// Known folders for the sidebar.
pub fn known(id: &str) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let p = match id {
        "Desktop" => home.join("Desktop"),
        "Downloads" => home.join("Downloads"),
        "Documents" => home.join("Documents"),
        _ => return None,
    };
    p.is_dir().then_some(p)
}
