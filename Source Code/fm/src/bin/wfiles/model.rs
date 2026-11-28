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
    Cleanup,
}

impl Loc {
    pub fn title(&self) -> String {
        match self {
            Loc::Folder(p) => folder_title(p),
            Loc::ThisPc => "This PC".into(),
            Loc::RecycleBin => "Recycle Bin".into(),
            Loc::Settings => "Settings".into(),
            Loc::Cleanup => "Cleanup".into(),
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

/// "Local Disk (C:)" style name for a drive root ("C:"), from the last drive list (painting never waits on a disk).
/// Before the first list only a local fixed disk is asked for its label (instant); a share root keeps its path.
pub fn drive_label(letter: &str) -> String {
    let l = letter.trim_end_matches('\\');
    let b = l.as_bytes();
    if b.len() != 2 || b[1] != b':' {
        return l.to_string();
    }
    if let Some(n) = crate::drives::cached_name(l) {
        return n;
    }
    let w = fmcore::util::wide(format!("{l}\\"));
    let kind = match unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(w.as_ptr())) } {
        2 => "removable",
        4 => "network",
        5 => "cdrom",
        6 => "ramdisk",
        _ => "fixed",
    };
    let mut name = [0u16; 128];
    let ok = kind == "fixed" && unsafe { windows::Win32::Storage::FileSystem::GetVolumeInformationW(windows::core::PCWSTR(w.as_ptr()), Some(&mut name), None, None, None, None) }.is_ok();
    let label = if ok { String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(0)]) } else { String::new() };
    if label.is_empty() { format!("{} ({l})", crate::drives::generic_name(kind, "")) } else { format!("{label} ({l})") }
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
    /// a folder with nothing in it at all (not "0 bytes": that can be empty files or folders)
    pub empty: bool,
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
            empty: false,
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
    Grid,
}

impl View {
    pub fn name(&self) -> &'static str {
        match self {
            View::Details => "details",
            View::Grid => "grid",
        }
    }
    pub fn from_name(s: &str) -> Option<View> {
        match s {
            "details" => Some(View::Details),
            "grid" | "thumbnails" => Some(View::Grid),
            _ => None,
        }
    }
    /// A view name from saved data: the "Compact list" view is gone (Sajjad, 2026-11-26), so an old "list" /
    /// "compact" entry loads as Details; anything unknown is None.
    pub fn from_saved(s: &str) -> Option<View> {
        match s {
            "list" | "compact" => Some(View::Details),
            _ => View::from_name(s),
        }
    }
}

/// The files a folder's Grid view preview shows, like Windows: its first two pictures (videos when it has none), in
/// the folder's own (name) order, never hidden ones. At most 400 entries are looked at, so a huge folder costs no more.
pub fn peek_files(dir: &Path) -> Vec<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let (mut pics, mut vids) = (vec![], vec![]);
    for e in rd.flatten().take(400) {
        let name = e.file_name().to_string_lossy().into_owned();
        let kind = kind_of(&name, false);
        if !matches!(kind, Kind::Img | Kind::Video) || e.metadata().map(|m| m.file_attributes() & 0x6 != 0 || m.is_dir()).unwrap_or(true) {
            continue;
        }
        if kind == Kind::Img {
            pics.push(e.path());
            if pics.len() == 2 {
                break;
            }
        } else if vids.len() < 2 {
            vids.push(e.path());
        }
    }
    if pics.is_empty() { vids } else { pics }
}

/// The view each folder (or drive) was last shown in, so it opens that way again: `views.json` in the data dir. A
/// folder never set shows Details, so Details is not stored; the oldest choice goes once VIEW_MAX are kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewMemory {
    /// (folder key, view), the most recent last
    list: Vec<(String, View)>,
}

pub const VIEW_MAX: usize = 400;

/// "c:\users\me" style key of a place that remembers its view ("c:" for a drive, "::bin" for the Recycle Bin).
fn view_key(loc: &Loc) -> Option<String> {
    match loc {
        Loc::Folder(p) => Some(p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase()).filter(|k| !k.is_empty()),
        Loc::RecycleBin => Some("::bin".into()),
        _ => None,
    }
}

impl ViewMemory {
    fn path() -> PathBuf {
        fmcore::util::data_dir().join("views.json")
    }
    pub fn load() -> ViewMemory {
        let v: serde_json::Value = std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Self::from_json(&v)
    }
    pub fn from_json(v: &serde_json::Value) -> ViewMemory {
        let list = v["views"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| Some((x[0].as_str()?.to_lowercase(), View::from_saved(x[1].as_str()?)?))).filter(|(_, v)| *v != View::Details).collect::<Vec<_>>())
            .unwrap_or_default();
        let skip = list.len().saturating_sub(VIEW_MAX);
        ViewMemory { list: list.into_iter().skip(skip).collect() }
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({"views": self.list.iter().map(|(k, v)| serde_json::json!([k, v.name()])).collect::<Vec<_>>()})
    }
    pub fn save(&self) {
        let p = Self::path();
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, self.to_json().to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
    /// The view this place opens in (None: a page with its own layout, This PC, Settings).
    pub fn get(&self, loc: &Loc) -> Option<View> {
        let k = view_key(loc)?;
        Some(self.list.iter().find(|(x, _)| *x == k).map(|(_, v)| *v).unwrap_or(View::Details))
    }
    /// Remember the view picked for this place; true when that changed what is kept.
    pub fn set(&mut self, loc: &Loc, v: View) -> bool {
        let Some(k) = view_key(loc) else { return false };
        let before = self.list.clone();
        self.list.retain(|(x, _)| *x != k);
        if v != View::Details {
            self.list.push((k, v));
            if self.list.len() > VIEW_MAX {
                self.list.remove(0);
            }
        }
        self.list != before
    }
}

/// Search state of a pane (the omnibox is in search mode).
#[derive(Clone, Debug)]
pub struct Search {
    /// the AI note when "AI Search" made the rows (typing again makes a normal search)
    pub ai: Option<String>,
    /// the words of the last finished query, and of the one on its way (for filtering locally while typing)
    pub done_words: String,
    pub pending_words: String,
    /// the words (spaces folded) of the last search asked for, plain or AI: an edit that leaves them as they are (a
    /// space, a caret move) asks nothing
    pub asked: String,
    /// the last plain answer found nothing: while the next answer is on its way that empty answer stays on screen as
    /// it is ("No results" and the AI Search button do not blink off and on per word)
    pub was_empty: bool,
    pub field: Field,
    pub pc: bool,
    pub ftype: usize,
    pub fsize: usize,
    pub fdate: usize,
    pub items: Vec<Item>,
    pub total: u32,
    pub seq: u64,
    pub busy: bool,
    pub note: String,
    /// every match Everything gave (ranked into `items`), "Include system & temp", and the opened "N more" rows
    pub all: Vec<Item>,
    pub junk: bool,
    /// the searched folder is itself a system / temp place (inside AppData or Temp): everything under it is shown
    pub scope_junk: bool,
    pub open: HashSet<String>,
    /// the user's own places (ranking), lower case with a trailing backslash
    pub own: Vec<String>,
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
/// The kinds a This PC search can leave out (Settings > Search): TYPES 1..=7, then every other file.
pub const KINDS: [&str; 8] = ["Folders", "Documents", "Images", "Video", "Audio", "Archives", "Code", "Other files"];

/// The user's own rules for This PC searches (Settings > Search), saved under "search" in the config.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchPrefs {
    /// show system and temp places (Windows, Program Files, AppData, temp, caches) instead of one hint row
    pub junk: bool,
    /// KINDS left out, one bit each
    pub kinds_off: u8,
    /// the folder tree's ticks: (folder or drive, searched). A folder follows its nearest listed parent; with none
    /// listed it is searched (the whole PC). Kept minimal: an entry always differs from its parent.
    pub places: Vec<(PathBuf, bool)>,
}

/// "c:\users\" style key: lower case, one trailing backslash.
fn pkey(p: &std::path::Path) -> String {
    format!("{}\\", p.to_string_lossy().trim_end_matches('\\').to_lowercase())
}

/// `a` is `b` or below it.
fn at_or_under(a: &std::path::Path, b: &std::path::Path) -> bool {
    pkey(a).starts_with(&pkey(b))
}

impl SearchPrefs {
    pub fn load(cfg: &serde_json::Value) -> SearchPrefs {
        let s = &cfg["search"];
        let places = s["places"].as_array().map(|a| a.iter().filter_map(|v| Some((PathBuf::from(v["path"].as_str()?), v["on"].as_bool()?))).collect()).unwrap_or_default();
        SearchPrefs { junk: s["system_temp"].as_bool().unwrap_or(false), kinds_off: s["kinds_off"].as_u64().unwrap_or(0) as u8, places }
    }

    pub fn save(&self) {
        let mut cfg = fmcore::pins::config();
        let places: Vec<serde_json::Value> = self.places.iter().map(|(p, on)| serde_json::json!({"path": p.to_string_lossy(), "on": on})).collect();
        cfg["search"] = serde_json::json!({"system_temp": self.junk, "kinds_off": self.kinds_off, "places": places});
        let _ = fmcore::pins::save_config(&cfg);
    }

    pub fn kind_on(&self, i: usize) -> bool {
        self.kinds_off & (1 << i) == 0
    }

    /// Is this folder searched? (its own entry, else the nearest listed parent's, else yes)
    pub fn searched(&self, p: &std::path::Path) -> bool {
        self.places.iter().filter(|(q, _)| at_or_under(p, q)).max_by_key(|(q, _)| pkey(q).len()).map(|(_, on)| *on).unwrap_or(true)
    }

    /// A whole-PC result passes the rules (after Everything's query, which leaves a ticked-out folder's own row in).
    pub fn keeps(&self, p: &std::path::Path) -> bool {
        self.places.is_empty() || self.searched(p)
    }

    /// Some folder below this one is ticked the other way (the tree shows a part tick).
    pub fn mixed(&self, p: &std::path::Path) -> bool {
        let on = self.searched(p);
        let k = pkey(p);
        self.places.iter().any(|(q, o)| *o != on && pkey(q).len() > k.len() && pkey(q).starts_with(&k))
    }

    /// Tick or untick a folder: it and everything below it follow (like Windows' Indexing Options).
    pub fn toggle(&mut self, p: &std::path::Path) {
        // a part-ticked folder becomes fully ticked; else it flips
        let on = self.mixed(p) || !self.searched(p);
        self.places.retain(|(q, _)| !at_or_under(q, p));
        let parent_on = p.parent().filter(|pp| pp != &p && !pp.as_os_str().is_empty()).map(|pp| self.searched(pp)).unwrap_or(true);
        if on != parent_on {
            self.places.push((p.to_path_buf(), on));
        }
        self.places.sort_by_key(|(q, _)| pkey(q));
    }

    /// Everything terms for these rules (empty when nothing is limited), with a trailing space.
    pub fn terms(&self) -> String {
        self.place_terms() + &self.kind_terms()
    }

    /// The places part (path terms: Everything checks them slower, so they go last in a query).
    pub fn place_terms(&self) -> String {
        let mut q = String::new();
        // the explicit entries form a tree; each one flips its parent's state
        let top: Vec<&(PathBuf, bool)> = self.places.iter().filter(|(p, _)| !self.places.iter().any(|(o, _)| o != p && at_or_under(p, o))).collect();
        if !top.is_empty() {
            q += &format!("!<{}> ", top.iter().map(|e| self.node(e)).collect::<Vec<_>>().join("|"));
        }
        q
    }

    /// The kinds part (name terms: cheap, they go first).
    pub fn kind_terms(&self) -> String {
        let mut q = String::new();
        if self.kinds_off != 0 {
            let mut alt: Vec<String> = (0..7).filter(|i| self.kind_on(*i)).map(|i| TYPES[i + 1].1.to_string()).collect();
            if self.kind_on(7) {
                // every other file: a file whose extension is none of the known kinds
                let known: Vec<&str> = TYPES[2..].iter().map(|t| t.1.trim_start_matches("ext:")).collect();
                alt.push(format!("<file: !ext:{}>", known.join(";")));
            }
            q += &match alt.len() {
                // every kind left out: nothing can match
                0 => "<folder: file:>".to_string(),
                1 => alt[0].clone(),
                _ => format!("<{}>", alt.join("|")),
            };
            q.push(' ');
        }
        q
    }

    /// One entry: the results below it that flip its parent's state (its own flipped children excepted).
    fn node(&self, e: &(PathBuf, bool)) -> String {
        let (p, _) = e;
        let own = p.to_string_lossy().trim_end_matches('\\').to_string();
        let dir = fmcore::evipc::quote(&format!("{own}\\"));
        // only what is inside the folder: a whole-path match (path:wfn:) for the folder itself cost ~45 ms per rule;
        // the folder's own row is put right afterwards by `keeps` (it follows its parent here)
        let under = dir;
        let kids: Vec<&(PathBuf, bool)> = self
            .places
            .iter()
            .filter(|(c, _)| c != p && at_or_under(c, p) && !self.places.iter().any(|(m, _)| m != c && m != p && at_or_under(c, m) && at_or_under(m, p)))
            .collect();
        if kids.is_empty() {
            under
        } else {
            format!("<{under} !<{}>>", kids.iter().map(|k| self.node(k)).collect::<Vec<_>>().join("|"))
        }
    }
}

pub const SIZES: [(&str, &str); 5] = [("Any", ""), ("Under 1 MB", "size:<1mb"), ("1 - 100 MB", "size:1mb..100mb"), ("Over 100 MB", "size:>100mb"), ("Over 1 GB", "size:>1gb")];
pub const DATES: [(&str, &str); 6] = [("Any", ""), ("Today", "dm:today"), ("This week", "dm:thisweek"), ("This month", "dm:thismonth"), ("This year", "dm:thisyear"), ("Older", "dm:<thisyear")];

impl Search {
    pub fn new(text: &str, pc: bool) -> Search {
        Search { ai: None, done_words: String::new(), pending_words: String::new(), asked: String::new(), was_empty: false, field: Field::new(text), pc, ftype: 0, fsize: 0, fdate: 0, items: vec![], total: 0, seq: 0, busy: false, note: String::new(), all: vec![], junk: false, scope_junk: false, open: HashSet::new(), own: vec![] }
    }
    pub fn next_request(&mut self) -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        self.seq = NEXT.fetch_add(1, Ordering::Relaxed);
        self.seq
    }
    /// Busy, but showing the last (empty) answer unchanged: no "Searching..." and no blank list for the new words.
    /// Not after a Backspace: fewer letters can find what more letters did not, so that answer is stale.
    pub fn quiet(&self) -> bool {
        let (new, old) = (self.pending_words.to_lowercase(), self.done_words.to_lowercase());
        let shorter = new.len() < old.len() && old.starts_with(&new);
        self.busy && self.was_empty && self.note.is_empty() && self.ai.is_none() && !shorter
    }

    /// Rank `all` into the rows shown (after a toggle or an opened "N more" row).
    pub fn rearrange(&mut self) {
        self.items = crate::rank::arrange(&self.all, self.field.text.trim(), &self.own, self.junk || self.scope_junk, &self.open, fmcore::util::now_ft());
    }

    /// The Everything query for the current words, scope and filters. `here` holds every real folder behind the
    /// place (the Desktop is the user's and the Public Desktop): they are OR-ed.
    pub fn query(&self, here: &[PathBuf]) -> String {
        self.query_with(here, &SearchPrefs::default())
    }

    /// The query with the user's own search settings: they apply to This PC searches (folders to search only /
    /// never, which kinds of files).
    pub fn query_with(&self, here: &[PathBuf], sp: &SearchPrefs) -> String {
        // Everything is faster with name terms first and path terms last (measured): the words, the filters and the
        // kinds, then the places rules, then the folder scope
        let mut parts: Vec<String> = vec![];
        let words = self.field.text.trim();
        if !words.is_empty() {
            parts.push(words.to_string());
        }
        for f in [TYPES[self.ftype].1, SIZES[self.fsize].1, DATES[self.fdate].1] {
            if !f.is_empty() {
                parts.push(f.to_string());
            }
        }
        // a whole-PC search: the This PC chip, or a place with no real folder behind it (This PC itself)
        if self.pc || here.is_empty() {
            for t in [sp.kind_terms(), sp.place_terms()] {
                if !t.trim().is_empty() {
                    parts.push(t.trim().to_string());
                }
            }
        }
        if !self.pc && !here.is_empty() {
            let ps: Vec<String> = here.iter().map(|h| fmcore::evipc::quote(&format!("{}\\", h.to_string_lossy().trim_end_matches('\\')))).collect();
            parts.push(if ps.len() == 1 { ps[0].clone() } else { format!("<{}>", ps.join("|")) });
        }
        parts.join(" ")
    }
}

pub struct HistoryEntry {
    pub loc: Loc,
    search: Option<Box<SearchView>>,
}

pub struct SearchView {
    search: Search,
    sel: HashSet<PathBuf>,
    focus: Option<usize>,
    anchor: Option<usize>,
    scroll: f32,
    kb: bool,
    saved_at: Instant,
}

pub struct Pane {
    pub loc: Loc,
    pub items: Vec<Item>,
    pub sel: HashSet<PathBuf>,
    pub focus: Option<usize>,
    pub anchor: Option<usize>,
    pub scroll: f32,
    pub sort: (SortCol, bool),
    pub back: Vec<HistoryEntry>,
    pub fwd: Vec<HistoryEntry>,
    pub search: Option<Search>,
    pub seq: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub shown_at: Instant,
    pub back_anim: bool,
    /// the folder's watchers (the Desktop also watches the Public Desktop)
    pub watch: Vec<crate::drives::Watch>,
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
            watch: vec![],
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
    fn history_entry(&mut self) -> HistoryEntry {
        let search = if self.searching() {
            let mut search = self.search.take().unwrap();
            if std::mem::take(&mut search.busy) {
                search.note.clear();
                search.pending_words.clear();
                search.done_words.clear();
                search.asked.clear();
                search.seq = 0;
            }
            Some(Box::new(SearchView {
                search, sel: std::mem::take(&mut self.sel), focus: self.focus,
                anchor: self.anchor, scroll: self.scroll, kb: self.kb, saved_at: Instant::now(),
            }))
        } else { None };
        HistoryEntry { loc: self.loc.clone(), search }
    }

    pub fn restore_search(&mut self, view: SearchView) {
        self.search = Some(view.search);
        self.sel = view.sel;
        self.focus = view.focus;
        self.anchor = view.anchor;
        self.scroll = view.scroll;
        self.sf.set(view.scroll);
        self.sf_written = view.scroll;
        self.kb = view.kb;
        self.want = None;
        self.loading = false;
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
            // by size, an unknown size goes after the known ones in both orders (it is not "0"); then by name
            if col == SortCol::Size && a.size.is_some() != b.size.is_some() {
                return b.size.is_some().cmp(&a.size.is_some());
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
    pub fn visit(&mut self, p: usize, loc: Loc, history: bool) {
        let pane = &mut self.panes[p];
        let searching = history && pane.searching();
        if history && (pane.loc != loc || searching) {
            let old = pane.history_entry();
            pane.back.push(old);
            pane.fwd.clear();
        }
        pane.back_anim = false;
        pane.loc = loc;
        if searching { self.trim_search_history(); }
    }

    pub fn step(&mut self, p: usize, back: bool) -> Option<HistoryEntry> {
        let pane = &mut self.panes[p];
        let entry = if back { pane.back.pop() } else { pane.fwd.pop() }?;
        let old = pane.history_entry();
        let added_search = old.search.is_some() && entry.search.is_none();
        if back {
            pane.want = old.loc.path().map(Path::to_path_buf);
            pane.fwd.push(old);
        } else {
            pane.back.push(old);
        }
        pane.loc = entry.loc.clone();
        pane.back_anim = back;
        if added_search { self.trim_search_history(); }
        Some(entry)
    }

    fn trim_search_history(&mut self) {
        const MAX_SEARCHES: usize = 5;
        let mut searches = Vec::new();
        for (p, pane) in self.panes.iter().enumerate() {
            for (forward, entries) in [(false, &pane.back), (true, &pane.fwd)] {
                for (i, entry) in entries.iter().enumerate() {
                    if let Some(view) = &entry.search {
                        searches.push((view.saved_at, p, forward, i));
                    }
                }
            }
        }
        if searches.len() > MAX_SEARCHES {
            let (_, p, forward, i) = *searches.iter().min().unwrap();
            let pane = &mut self.panes[p];
            if forward { pane.fwd.remove(i); } else { pane.back.remove(i); }
        }
    }

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

impl HistoryEntry {
    pub fn into_search(self) -> Option<Box<SearchView>> {
        self.search
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn tab() -> Tab {
        Tab { id: 1, panes: vec![Pane::new(Loc::ThisPc)], active: 0, view: View::Details, split_at: Instant::now() }
    }

    #[test]
    fn back_and_forward_restore_normal_and_ai_search_views() {
        for ai in [None, Some("AI kept these results".to_owned())] {
            let mut tab = tab();
            let base = Loc::Folder(r"C:\fixture".into());
            tab.visit(0, base.clone(), true);
            let row = Item {
                name: "found folder".into(), path: r"C:\fixture\found folder".into(), is_dir: true,
                size: None, modified: 0, attrs: 0, kind: Kind::Folder, folder: r"C:\fixture".into(),
                hl: vec![], recycled: None, empty: true,
            };
            let mut search = Search::new("fixture", false);
            search.ai = ai.clone();
            search.items.push(row.clone());
            search.all.push(row.clone());
            search.total = 1;
            search.ftype = 1;
            search.open.insert("group".into());
            let seq = search.next_request();
            let pane = tab.pane_mut();
            pane.search = Some(search);
            pane.sel.insert(row.path.clone());
            pane.focus = Some(0);
            pane.anchor = Some(0);
            pane.scroll = 75.0;
            pane.kb = true;

            // Opening a result's location can land in the same folder as the search scope.
            tab.visit(0, base.clone(), true);
            assert!(tab.pane().search.is_none());
            let back = tab.step(0, true).unwrap().into_search().unwrap();
            tab.pane_mut().restore_search(*back);
            let pane = tab.pane();
            let search = pane.search.as_ref().unwrap();
            assert_eq!(pane.loc, base);
            assert_eq!((pane.scroll, pane.focus, pane.anchor, pane.kb), (75.0, Some(0), Some(0), true));
            assert_eq!(pane.sel, HashSet::from([row.path.clone()]));
            assert_eq!(search.field.text, "fixture");
            assert_eq!(search.ai, ai);
            assert_eq!((search.total, search.ftype, search.seq), (1, 1, seq));
            assert_eq!(search.items[0].path, row.path);
            assert_eq!(search.all[0].path, row.path);
            assert!(search.open.contains("group"));
            assert!(tab.step(0, false).unwrap().into_search().is_none());
            assert!(tab.step(0, true).unwrap().into_search().is_some());
        }
    }

    #[test]
    fn search_history_caps_both_panes_and_can_restore_forward() {
        let mut tab = tab();
        tab.panes.push(Pane::new(Loc::RecycleBin));
        for i in 0..6 {
            let p = i % 2;
            tab.panes[p].search = Some(Search::new(&format!("query {i}"), false));
            tab.visit(p, Loc::Folder(format!(r"C:\fixture{i}").into()), true);
        }
        assert_eq!(tab.panes.iter().flat_map(|p| &p.back).filter(|e| e.search.is_some()).count(), 5);
        assert!(tab.panes[0].back.iter().all(|e| e.search.as_ref().is_none_or(|s| s.search.field.text != "query 0")));
        let first = tab.step(1, true).unwrap().into_search().unwrap();
        tab.panes[1].restore_search(*first);
        let older = tab.step(1, true).unwrap().into_search().unwrap();
        tab.panes[1].restore_search(*older);
        let newer = tab.step(1, false).unwrap().into_search().unwrap();
        assert_eq!(newer.search.field.text, "query 5");
        assert_eq!(tab.panes[0].loc, Loc::Folder(r"C:\fixture4".into()));
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
                empty: false,
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
/// Shell names Windows passes for its virtual folders (the desktop icons, "shell:" links) -> our pages.
/// This PC: `::{20D04FE0-3AEA-1069-A2D8-08002B30309D}` / `shell:MyComputerFolder`; Recycle Bin:
/// `::{645FF040-5081-101B-9F08-00AA002F954E}` / `shell:RecycleBinFolder`.
pub fn special_loc(s: &str) -> Option<Loc> {
    let s = s.trim().trim_matches('"').to_ascii_lowercase();
    let s = s.strip_prefix("shell:").unwrap_or(&s);
    let s = s.trim_start_matches(':');
    match s {
        "{20d04fe0-3aea-1069-a2d8-08002b30309d}" | "mycomputerfolder" | "this pc" => Some(Loc::ThisPc),
        "{645ff040-5081-101b-9f08-00aa002f954e}" | "recyclebinfolder" | "recycle bin" => Some(Loc::RecycleBin),
        _ => None,
    }
}

pub fn known(id: &str) -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads};
    let guid = match id {
        "Desktop" => FOLDERID_Desktop,
        "Downloads" => FOLDERID_Downloads,
        "Documents" => FOLDERID_Documents,
        _ => return None,
    };
    known_folder(&guid, id)
}

/// A known folder's real path (OneDrive-redirected folders included), cached; `name` = the fallback under the profile.
fn known_folder(id: &windows::core::GUID, name: &str) -> Option<PathBuf> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use windows::Win32::UI::Shell::{KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let c = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(p) = c.lock().unwrap().get(name) {
        return p.clone();
    }
    let p = unsafe { SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None) }
        .ok()
        .map(fmcore::util::take_pwstr)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(name)))
        .filter(|p| p.is_dir());
    c.lock().unwrap().insert(name.to_string(), p.clone());
    p
}

/// The Public Desktop (FOLDERID_PublicDesktop): shortcuts installed "for all users" live there.
pub fn public_desktop() -> Option<PathBuf> {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_PublicDesktop, "PublicDesktop")
}

#[cfg(test)]
mod search_tests {
    use super::*;
    #[test]
    fn merged_desktop_query_ors_both_folders() {
        let s = Search::new("gears", false);
        let q = s.query(&[PathBuf::from(r"C:\Users\S\Desktop"), PathBuf::from(r"C:\Users\Public\Desktop")]);
        assert_eq!(q, r#"gears <"C:\Users\S\Desktop\"|"C:\Users\Public\Desktop\">"#);
        let one = s.query(&[PathBuf::from(r"E:\x")]);
        assert_eq!(one, r#"gears "E:\x\""#);
        let pc = Search::new("gears", true);
        assert_eq!(pc.query(&[PathBuf::from(r"E:\x")]), "gears");
    }

    #[test]
    fn search_places_tree() {
        let mut sp = SearchPrefs::default();
        let c = PathBuf::from(r"C:\");
        let me = PathBuf::from(r"C:\Users\Sajjad");
        // untick C:, then tick only C:\Users\Sajjad
        sp.toggle(&c);
        assert!(!sp.searched(&c) && !sp.searched(&PathBuf::from(r"C:\Windows")));
        sp.toggle(&me);
        assert!(sp.searched(&me) && sp.searched(&PathBuf::from(r"C:\Users\Sajjad\Docs")) && !sp.searched(&PathBuf::from(r"C:\Users")));
        assert!(sp.mixed(&c) && sp.mixed(&PathBuf::from(r"C:\Users")) && !sp.mixed(&me));
        let pc = Search::new("gears", true);
        assert_eq!(pc.query_with(&[], &sp), r#"gears !<<"C:\" !<"C:\Users\Sajjad\">>>"#);
        // the folder's own row: kept or dropped by its own tick
        assert!(sp.keeps(&me) && !sp.keeps(&PathBuf::from(r"C:\Users")));
        // a folder search ignores them
        let here = Search::new("gears", false);
        assert_eq!(here.query_with(&[PathBuf::from(r"E:\x")], &sp), r#"gears "E:\x\""#);
        // ticking C: again clears everything below
        sp.toggle(&c);
        assert!(sp.places.is_empty());
        // only folders and images
        let sp = SearchPrefs { kinds_off: 0xFF & !(1 | 4), ..Default::default() };
        assert_eq!(pc.query_with(&[], &sp), "gears <folder:|ext:png;jpg;jpeg;gif;webp;bmp;heic;svg;ico>");
        let sp = SearchPrefs { kinds_off: 0xFF, ..Default::default() };
        assert!(pc.query_with(&[], &sp).ends_with("<folder: file:>"));
    }
}

#[cfg(test)]
mod search_live {
    /// Live check against the running Everything (cargo test -- --ignored): the rule terms are valid syntax.
    #[test]
    #[ignore]
    fn rule_terms_work_in_everything() {
        use fmcore::evipc::*;
        let n = |q: &str| query(q, 50, REQ_NAME | REQ_PATH, SORT_NAME, false, std::time::Duration::from_secs(3)).map(|r| r.total).unwrap_or(9999);
        let all = n("gears");
        let d = n(r#""D:\" gears"#);
        let d2 = n(r#"<"D:\"|"C:\"> gears"#);
        let skip = n(r#"!"C:\ProgramData\" gears"#);
        let skip_self = n(r#"!<"D:\Gears of War - E day\"|path:wfn:"D:\Gears of War - E day"> gears"#);
        println!("skip_self {skip_self}");
        let kinds = n(r#"<folder:|<file: !ext:lnk;exe>> gears"#);
        let none = n(r#"<folder: file:> gears"#);
        // the tree: C: unticked except C:\Users\Sajjad (ticked again), from SearchPrefs::terms
        let mut sp = super::SearchPrefs::default();
        sp.toggle(std::path::Path::new(r"C:\"));
        sp.toggle(std::path::Path::new(r"C:\Users\Sajjad"));
        let tree = n(&format!("{}gears", sp.terms()));
        let c = n(r#""C:\" gears"#);
        let mine = n(r#""C:\Users\Sajjad\" gears"#);
        println!("tree {tree} = all {all} - c {c} + mine {mine}");
        assert_eq!(tree, all - c + mine);
        println!("all {all} d {d} d|c {d2} skip {skip} kinds {kinds} none {none}");
        assert!(all > 0 && d > 0 && d < all && d2 >= d && skip < all && kinds > 0 && none == 0);
    }
}

#[cfg(test)]
mod hidden_timing {
    /// cargo test hidden_cost -- --ignored --nocapture: how long the hidden-folder check takes on real results
    #[test]
    #[ignore]
    fn hidden_cost() {
        use fmcore::evipc::*;
        for w in ["lumen", "gears", "settings", "a", "settings", "a"] {
            let t0 = std::time::Instant::now();
            let r = query(&format!("{w} !\\."), 1500, REQ_NAME | REQ_PATH | REQ_SIZE | REQ_MODIFIED | REQ_ATTRS, SORT_DATE_MODIFIED_DESC, false, std::time::Duration::from_secs(3));
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    println!("{w}: error {e} after {:?}", t0.elapsed());
                    continue;
                }
            };
            let q = t0.elapsed();
            let t1 = std::time::Instant::now();
            let mut cache = crate::app::HIDDEN_DIRS.lock().unwrap();
            let kept = r.items.iter().filter(|i| i.attrs & 0x2 == 0 && !crate::app::in_hidden_folder(std::path::Path::new(&i.path), &mut cache)).count();
            println!("{w}: {} items, query {:?}, hidden check {:?} ({} folders read), kept {kept}", r.items.len(), q, t1.elapsed(), cache.len());
        }
    }
}

#[cfg(test)]
mod real_rules_timing {
    /// cargo test real_rules -- --ignored --nocapture: the user's own Settings > Search rules, timed in Everything
    #[test]
    #[ignore]
    fn real_rules() {
        use fmcore::evipc::*;
        let sp = super::SearchPrefs::load(&fmcore::pins::config());
        let s = super::Search::new("", true);
        println!("terms: {}", sp.terms());
        for w in ["l", "lu", "lum", "lumen", "a", "settings"] {
            for (name, q) in [("plain", w.to_string()), ("rules", format!("{}{w}", sp.terms()))] {
                let _ = &s;
                let t0 = std::time::Instant::now();
                let r = query(&q, 1500, REQ_NAME | REQ_PATH | REQ_SIZE | REQ_MODIFIED | REQ_ATTRS, SORT_DATE_MODIFIED_DESC, false, std::time::Duration::from_secs(5));
                println!("{w:>9} {name}: {:?} ({})", t0.elapsed(), r.map(|r| r.total.to_string()).unwrap_or_else(|e| e));
            }
        }
    }
}

/// Search words with every run of spaces folded to one (what Everything sees: "a  b " = "a b").
pub fn fold_words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod quiet_tests {
    use super::*;
    #[test]
    fn empty_answer_stays_on_screen_only_for_more_letters() {
        let mut s = Search::new("zzz", false);
        s.busy = true;
        s.was_empty = true;
        s.done_words = "zzzx".into();
        // a Backspace: fewer letters can find what more letters did not
        s.pending_words = "zzz".into();
        assert!(!s.quiet());
        // one more letter on an empty answer: nothing new can appear, the list stays as it is
        s.done_words = "zz".into();
        assert!(s.quiet());
    }
}

#[cfg(test)]
mod view_memory_tests {
    use super::*;

    #[test]
    fn each_folder_and_drive_keeps_its_own_view() {
        let mut m = ViewMemory::default();
        let pics = Loc::Folder(PathBuf::from("C:/Users/Me/Pictures"));
        let drive = Loc::Folder(PathBuf::from("D:/"));
        assert_eq!(m.get(&pics), Some(View::Details));
        assert!(m.set(&pics, View::Grid));
        assert!(m.set(&drive, View::Grid));
        // case and a trailing slash do not matter; other folders (even below) stay Details
        assert_eq!(m.get(&Loc::Folder(PathBuf::from("c:/users/me/pictures/"))), Some(View::Grid));
        assert_eq!(m.get(&Loc::Folder(PathBuf::from("D:"))), Some(View::Grid));
        assert_eq!(m.get(&Loc::Folder(PathBuf::from("C:/Users/Me/Pictures/2024"))), Some(View::Details));
        // pages with their own layout keep none
        assert_eq!(m.get(&Loc::ThisPc), None);
        assert!(!m.set(&Loc::Settings, View::Grid));
        // the same choice again changes nothing; Details is the default and is not stored
        m.set(&pics, View::Grid);
        assert!(!m.set(&pics, View::Grid));
        assert!(m.set(&pics, View::Details));
        assert_eq!(m.to_json()["views"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn kept_small_and_survives_a_save() {
        let mut m = ViewMemory::default();
        for i in 0..VIEW_MAX + 25 {
            m.set(&Loc::Folder(PathBuf::from(format!("E:/f{i}"))), View::Grid);
        }
        let back = ViewMemory::from_json(&m.to_json());
        assert_eq!(back, m);
        assert_eq!(m.to_json()["views"].as_array().unwrap().len(), VIEW_MAX);
        // the oldest went first; a folder set again moves to the newest end
        assert_eq!(m.get(&Loc::Folder(PathBuf::from("E:/f0"))), Some(View::Details));
        assert_eq!(m.get(&Loc::Folder(PathBuf::from(format!("E:/f{}", VIEW_MAX + 24)))), Some(View::Grid));
        m.set(&Loc::Folder(PathBuf::from("E:/f30")), View::Grid);
        m.set(&Loc::Folder(PathBuf::from("E:/new")), View::Grid);
        assert_eq!(m.get(&Loc::Folder(PathBuf::from("E:/f30"))), Some(View::Grid));
        // a broken file reads as empty
        assert_eq!(ViewMemory::from_json(&serde_json::json!({"views": [[1, 2], ["x", "nope"]]})), ViewMemory::default());
    }
}

#[cfg(test)]
mod peek_tests {
    use super::*;

    #[test]
    fn folder_preview_takes_the_first_two_pictures() {
        let d = std::env::temp_dir().join(format!("wfm-peek-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub.png")).unwrap();
        for f in ["a.txt", "b.mp4", "c.jpg", "d.png", "e.jpg"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        // pictures first, in name order; a folder named like a picture is not one
        assert_eq!(names(peek_files(&d)), ["c.jpg", "d.png"]);
        for f in ["c.jpg", "d.png", "e.jpg"] {
            std::fs::remove_file(d.join(f)).unwrap();
        }
        // no pictures: the videos
        assert_eq!(names(peek_files(&d)), ["b.mp4"]);
        std::fs::remove_file(d.join("b.mp4")).unwrap();
        assert!(peek_files(&d).is_empty());
        assert!(peek_files(&d.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn old_compact_list_entries_load_as_details() {
        assert_eq!(View::from_saved("list"), Some(View::Details));
        assert_eq!(View::from_saved("compact"), Some(View::Details));
        assert_eq!(View::from_saved("grid"), Some(View::Grid));
        assert_eq!(View::from_name("list"), None, "gui view list is gone");
        let v = serde_json::json!({"views": [["d:\\old", "list"], ["d:\\pics", "grid"], ["d:\\x", "nonsense"]]});
        let m = ViewMemory::from_json(&v);
        assert_eq!(m.get(&Loc::Folder(PathBuf::from(r"D:\old"))), Some(View::Details));
        assert_eq!(m.get(&Loc::Folder(PathBuf::from(r"D:\pics"))), Some(View::Grid));
        assert_eq!(m.to_json()["views"].as_array().unwrap().len(), 1, "only the Grid entry is kept");
    }
}
