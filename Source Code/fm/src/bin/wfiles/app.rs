//! The window: state, custom frame (tabs in the title bar), Mica, DPI, background work, the `wfm gui` pipe and
//! offscreen screenshots. Painting is in paint.rs, input and actions in input.rs.
use crate::fmt::Field;
use crate::gfx::{Gfx, Rect};
use crate::jobs;
use crate::model::*;
use crate::preview;
use crate::theme::{self, Theme};
use fmcore::shell::Drive;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub const TITLE_H: f32 = 40.0;
/// caption button width (Windows 11: 46 px; they span the title bar height)
pub const CAP_W: f32 = 46.0;
pub const BAR_H: f32 = 48.0;
pub const TOP: f32 = TITLE_H + BAR_H;
pub const RAIL_W: f32 = 52.0;
pub const STATUS_H: f32 = 30.0;
pub const HEADER_H: f32 = 34.0;
pub const ROW2_H: f32 = 46.0;
pub const STRIP_H: f32 = 48.0;
pub const PANE_HEAD_H: f32 = 40.0;
pub const WHATIS_W: f32 = 320.0;

pub const WM_BG: u32 = WM_APP + 1;
pub const WM_IPC: u32 = WM_APP + 2;
pub const WM_JOB: u32 = WM_APP + 3;
pub const WM_MOUSELEAVE: u32 = 0x02A3;
pub const WM_NCMOUSELEAVE: u32 = 0x02A2;
pub const T_ANIM: usize = 1;
pub const T_RELIST: usize = 2;
pub const T_SEARCH: usize = 3;
pub const T_TOAST: usize = 4;
pub const T_MARQ: usize = 5;
pub const T_CARET: usize = 9;

/// Results of background work, applied on the UI thread.
pub enum Msg {
    Listed { tab: u64, pane: usize, seq: u64, res: Result<Vec<Item>, String>, ms: f64 },
    Sizes { dir: PathBuf, sizes: HashMap<String, u64> },
    Found { tab: u64, pane: usize, seq: u64, res: Result<(u32, Vec<Item>), String> },
    Drives(Vec<Drive>),
    Recent(Vec<fmcore::pins::Recent>),
    Thumb { path: PathBuf, img: Option<(u32, u32, Vec<u8>)> },
    Changed { dir: PathBuf },
    ShellMenu { seq: u64, items: Vec<fmcore::menu::MenuItem> },
    Preview { data: preview::Data },
    WhatIs { path: PathBuf, res: Result<crate::ai::FileAnswer, String> },
    Names { seq: u64, res: Result<Vec<String>, String> },
    Toast { text: String, undo: bool },
    Refresh,
    EvStatus { running: bool, total: u32 },
    KeyTest(Result<(), String>),
    OpenWith { path: PathBuf, apps: Vec<fmcore::shell::Handler> },
    AiFlags { ai: bool, meaning: bool },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Hit {
    None,
    Tab(usize),
    TabClose(usize),
    TabAdd,
    Min,
    Max,
    Close,
    Back,
    Fwd,
    Up,
    Omni,
    Crumb(usize),
    OmniSearch,
    OmniClear,
    New,
    ViewBtn,
    Side(usize),
    SideThisPc,
    SideSettings,
    SideEdge,
    /// header divider of a column (pane, 0 date / 1 type / 2 size)
    ColEdge(usize, u8),
    Header(usize, u8),
    Row(usize, usize),
    ListBg(usize),
    PaneHead(usize),
    Xfer,
    Scope(u8),
    Filter(u8),
    Meaning,
    MenuItem(usize),
    MenuQuick(u8),
    SubItem(usize),
    Drop(usize),
    Pv(u8),
    Job(u64, u8),
    Clash(u64, u8),
    SameChoice(u64),
    XferHead(u8),
    Toast(u8),
    Pal(usize),
    Dlg(u8),
    DlgRow(usize),
    Set(u8),
    WhatIs(u8),
    Scroll(usize),
    Block,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Open,
    OpenNewTab,
    OpenWith,
    Preview,
    ExtractTo,
    ExtractHere,
    WhatIs,
    CopyPath,
    Cut,
    Copy,
    Paste,
    Rename,
    Delete,
    DeleteForGood,
    Zip,
    NewFolder,
    NewText,
    NewMenu,
    Properties,
    SelectAll,
    Terminal,
    CopyToMenu,
    MoveToMenu,
    CopyOther,
    MoveOther,
    BatchRename,
    Split,
    Settings,
    Undo,
    Pin,
    Restore,
    ViewMenu,
    SortMenu,
    Transfers,
    Palette,
    Refresh,
    ToggleHidden,
    None,
}

#[derive(Clone, Debug)]
pub struct MItem {
    pub label: String,
    pub glyph: u32,
    pub keys: String,
    pub act: Act,
    /// shell verb / text (Act::None + shell) or a value for dropdowns
    pub shell: Option<String>,
    pub value: i64,
    pub bold: bool,
    pub enabled: bool,
    pub sep: bool,
    pub sub: bool,
    pub checked: bool,
    /// the shell entry's own icon
    pub icon: Option<std::sync::Arc<fmcore::menu::Icon>>,
    /// a colour swatch instead of a glyph (accent picker)
    pub swatch: Option<u32>,
}

impl MItem {
    pub fn new(label: &str, glyph: u32, act: Act) -> MItem {
        MItem { label: label.into(), glyph, keys: String::new(), act, shell: None, value: 0, bold: false, enabled: true, sep: false, sub: false, checked: false, icon: None, swatch: None }
    }
    pub fn keys(mut self, k: &str) -> MItem {
        self.keys = k.into();
        self
    }
    pub fn sep(mut self) -> MItem {
        self.sep = true;
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropKind {
    New,
    View,
    Sort,
    FType,
    FSize,
    FDate,
    Theme,
    Lang,
    OpenWith,
    Insert,
    Style,
    Digits,
    CopyTo,
    MoveTo,
    More,
    Shell,
    Look(u8),
}

pub struct CtxMenu {
    pub x: f32,
    pub y: f32,
    pub items: Vec<MItem>,
    pub paths: Vec<PathBuf>,
    pub bg: bool,
    pub seq: u64,
    pub loading: bool,
    pub shell: Vec<fmcore::menu::MenuItem>,
    pub at: Instant,
}

pub struct DropMenu {
    pub kind: DropKind,
    pub anchor: Rect,
    pub items: Vec<MItem>,
    pub width: f32,
    pub at: Instant,
    /// a submenu of the context menu (stays with it)
    pub child: bool,
}

pub struct Pv {
    pub idx: usize,
    pub path: PathBuf,
    pub data: Option<preview::Data>,
    pub scroll: f32,
    pub at: Instant,
    pub from: Rect,
}

pub struct PItem {
    pub label: String,
    pub sub: String,
    pub folder: bool,
    pub glyph: u32,
    pub act: Act,
    pub loc: Option<Loc>,
    pub keys: Vec<&'static str>,
    pub hl: Vec<usize>,
}

pub struct Palette {
    pub field: Field,
    pub sel: usize,
    pub items: Vec<PItem>,
    pub at: Instant,
}

pub struct Rename {
    pub mode: u8,
    pub pattern: Field,
    pub find: Field,
    pub repl: Field,
    pub start: Field,
    pub digits: usize,
    pub focus: u8,
    pub items: Vec<Item>,
    pub rows: Vec<(String, String, Option<(u32, u32)>)>,
    pub problem: Option<String>,
    pub ai: Option<Vec<String>>,
    pub ai_busy: bool,
    pub ai_err: Option<String>,
    pub style: usize,
    pub seq: u64,
    pub at: Instant,
}

pub struct WhatIs {
    pub item: Item,
    pub answer: Option<Result<crate::ai::FileAnswer, String>>,
    pub facts: Vec<(String, String)>,
}

pub struct Toast {
    pub text: String,
    pub bold: String,
    pub undo: bool,
    pub at: Instant,
    pub hover: bool,
}

pub struct Inline {
    pub pane: usize,
    pub path: PathBuf,
    pub field: Field,
}

pub struct DragIn {
    pub paths: Vec<PathBuf>,
    pub x: f32,
    pub y: f32,
    pub target: Option<PathBuf>,
    pub effect: u32,
}

#[derive(Clone)]
pub struct UiSettings {
    pub sizes: bool,
    pub ext: bool,
    pub hidden: bool,
    pub ask_delete: bool,
    pub scope_pc: bool,
}

pub struct SetState {
    pub ev_running: Option<bool>,
    pub ev_total: u32,
    pub msg: Option<String>,
    pub msg_ok: bool,
}

pub struct App {
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub t: Theme,
    pub ui: UiSettings,
    pub tabs: Vec<Tab>,
    pub cur: usize,
    pub next_id: u64,
    pub w: f32,
    pub h: f32,
    pub hits: Vec<(Rect, Hit)>,
    pub hover: Hit,
    pub nc_hover: Hit,
    pub nc_down: Hit,
    pub pressed: Hit,
    pub mx: f32,
    pub my: f32,
    pub drives: Vec<Drive>,
    pub recent: Vec<fmcore::pins::Recent>,
    pub side: Vec<(String, u32, u32, Loc)>,
    pub path_edit: Option<Field>,
    pub search_focus: bool,
    pub menu: Option<CtxMenu>,
    pub drop: Option<DropMenu>,
    pub sub: Option<DropMenu>,
    pub pv: Option<Pv>,
    pub xfer_open: bool,
    pub palette: Option<Palette>,
    pub rename: Option<Rename>,
    pub whatis: Option<WhatIs>,
    pub toast: Option<Toast>,
    pub inline: Option<Inline>,
    pub drag_in: Option<DragIn>,
    pub press: Option<(f32, f32, usize, usize)>,
    pub drag_scroll: Option<(usize, f32, f32)>,
    pub jobs: jobs::Shared,
    pub bg: Arc<Mutex<Vec<Msg>>>,
    pub ipc_in: Arc<Mutex<Vec<(String, std::sync::mpsc::Sender<String>)>>>,
    pub thumbs: HashMap<PathBuf, Option<(u32, u32, Vec<u8>)>>,
    pub bitmaps: HashMap<PathBuf, Option<ID2D1Bitmap>>,
    pub bmp_target: usize,
    pub thumb_q: Arc<(Mutex<Vec<PathBuf>>, Condvar)>,
    pub thumb_asked: HashSet<PathBuf>,
    pub sizes: HashMap<PathBuf, HashMap<String, u64>>,
    pub menu_gen: u64,
    pub recycle: Vec<fmcore::recycle::Recycled>,
    pub set: SetState,
    pub ai_on: bool,
    pub meaning_on: bool,
    pub open_with: Option<(PathBuf, Vec<fmcore::shell::Handler>)>,
    pub started: Instant,
    pub first_paint_ms: Option<f64>,
    pub first_list_ms: Option<f64>,
    pub shooting: bool,
    pub mica: bool,
    pub caret_on: bool,
    pub forced_dark: Option<bool>,
    pub last_error: Option<String>,
    pub key_field: Field,
    pub key_focus: bool,
    pub confirm: Option<(String, Act)>,
    pub pending_rename: Option<PathBuf>,
    pub toast_job: Option<u64>,
    pub done_seen: HashSet<u64>,
    pub dragging_out: bool,
    /// Settings > Personalize (look.json)
    pub look: crate::look::Look,
    /// hover and press amounts (eased)
    pub hov_f: crate::motion::Fades<Hit>,
    pub press_f: crate::motion::Fades<Hit>,
    /// menus that are closing (painted fading out, without hits)
    pub menu_out: Option<(CtxMenu, Instant)>,
    pub drop_out: Option<(DropMenu, bool, Instant)>,
    pub marquee: Option<Marquee>,
    /// press on the empty part of a row: a drag there starts the marquee, not a drag-out
    pub press_ws: bool,
    /// sidebar edge drag: (start x, start width)
    pub side_drag: Option<(f32, f32)>,
    /// column divider drag: (column, start x, start width)
    pub col_drag: Option<(u8, f32, f32)>,
    /// narrow window: the sidebar is open (as the rail) although the window is too narrow for it
    pub side_open: bool,
    /// Ctrl / Shift of a test drag (`wfm gui drag ... ctrl`)
    pub test_mods: Option<(bool, bool)>,
    /// height of the Settings page (scroll limit)
    pub settings_h: f32,
    pub caret_timer: bool,
    /// hits are not recorded (painting a closing menu)
    pub no_hits: bool,
    pub offscreen: bool,
    /// the OS can draw a backdrop (Windows 11 22H2+, not --no-mica)
    pub can_mica: bool,
    pub accent_field: Option<Field>,
}

/// Rubber-band selection: start in content coordinates (scroll included), end in window coordinates.
pub struct Marquee {
    pub pane: usize,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub base: HashSet<PathBuf>,
    pub ctrl: bool,
    pub shift: bool,
}

pub fn trace(msg: &str) {
    if std::env::var("WFILES_TRACE").is_ok() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("wfiles-trace.log")) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

impl App {
    pub fn scale(&self) -> f32 {
        self.gfx.scale()
    }

    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.cur]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.cur]
    }

    pub fn pane(&self) -> &Pane {
        self.tabs[self.cur].pane()
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        self.tabs[self.cur].pane_mut()
    }

    pub fn post(&self, m: Msg) {
        post_bg(&self.bg, self.hwnd.0 as isize, m);
    }

    /// Run `f` on a worker thread; its message comes back to the UI thread.
    pub fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let bg = self.bg.clone();
        let hw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let _c = fmcore::util::com();
            let m = f();
            post_bg(&bg, hw, m);
        });
    }

    /// Ask for the next frame (~60 fps). Only called while something moves; the timer stops itself after one tick.
    pub fn anim(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), T_ANIM, 15, None);
        }
    }

    /// Hover amount 0..1 of a hit (eased in and out).
    pub fn hk(&self, h: Hit) -> f32 {
        if self.shooting { 0.0 } else { self.hov_f.get(h) }
    }

    /// Press amount 0..1 of a hit.
    pub fn pk(&self, h: Hit) -> f32 {
        if self.shooting { 0.0 } else { self.press_f.get(h) }
    }

    pub fn save_look(&self) {
        self.look.save();
    }

    /// Backdrop type and the render target's transparency, live.
    pub fn apply_backdrop(&mut self) {
        let on = self.can_mica && self.look.backdrop != 3;
        unsafe {
            if self.can_mica {
                let v: i32 = self.look.dwm_backdrop();
                let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &v as *const i32 as *const _, 4);
            }
        }
        self.mica = on;
        self.gfx.set_alpha(on);
        self.invalidate();
    }

    pub fn timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), id, ms, None);
        }
    }

    pub fn dark(&self) -> bool {
        self.t.dark
    }

    pub fn apply_theme(&mut self) {
        let dark = match self.forced_dark {
            Some(d) => d,
            None => match self.look.theme {
                1 => false,
                2 => true,
                _ => theme::system_dark(),
            },
        };
        self.t = theme::make(dark, self.look.accent);
        self.gfx.corner = if self.look.square { 0.0 } else { 1.0 };
        set_dark_frame(self.hwnd, dark);
        self.invalidate();
    }

    /// The sidebar shows as the icon rail: narrow window, split view, or collapsed by its menu button.
    pub fn rail(&self) -> bool {
        self.w < 1000.0 || self.tab().split() || self.look.side_w < 1.0
    }

    /// Very narrow window: no sidebar unless opened with the menu button in the top bar.
    pub fn side_hidden(&self) -> bool {
        self.w < 640.0 && !self.side_open
    }

    pub fn side_w(&self) -> f32 {
        if self.side_hidden() {
            0.0
        } else if self.rail() {
            RAIL_W
        } else {
            self.look.side_w
        }
    }

    /// The content layer (right of the sidebar, below the top bar).
    pub fn main_rect(&self) -> Rect {
        let x = self.side_w();
        Rect::new(x, TOP, self.w - x, self.h - TOP)
    }

    // ------------------------------------------------------------------ sidebar
    pub fn build_side(&mut self) {
        let mut side = vec![];
        for (name, glyph, color) in [("Desktop", 0xE7F4u32, 0x2E7DD1u32), ("Downloads", 0xE896, 0x2E9E5B), ("Documents", 0xE8A5, 0x3A6FD8)] {
            if let Some(p) = known(name) {
                side.push((name.to_string(), glyph, color, Loc::Folder(p)));
            }
        }
        for p in fmcore::pins::pins() {
            if p.exists && Path::new(&p.path).is_dir() && !side.iter().any(|s| s.3 == Loc::Folder(PathBuf::from(&p.path))) {
                side.push((p.name.clone(), 0, 0, Loc::Folder(PathBuf::from(&p.path))));
            }
        }
        side.push(("Recycle Bin".into(), 0xE74D, 0, Loc::RecycleBin));
        self.side = side;
    }

    // ------------------------------------------------------------------ navigation
    pub fn new_tab(&mut self, loc: Loc) {
        self.next_id += 1;
        let view = self.tabs.get(self.cur).map(|t| t.view).unwrap_or(View::Details);
        let tab = Tab { id: self.next_id, panes: vec![Pane::new(loc.clone())], active: 0, view, split_at: Instant::now() };
        let at = if self.tabs.is_empty() { 0 } else { self.cur + 1 };
        self.tabs.insert(at, tab);
        self.cur = at;
        self.path_edit = None;
        self.load(self.cur, 0, false);
    }

    pub fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return;
        }
        self.tabs.remove(i);
        if self.cur >= self.tabs.len() || self.cur > i {
            self.cur = self.cur.saturating_sub(1).min(self.tabs.len() - 1);
        }
        self.whatis = None;
        self.invalidate();
    }

    pub fn go(&mut self, loc: Loc) {
        let (t, p) = (self.cur, self.tabs[self.cur].active);
        self.go_in(t, p, loc, true);
    }

    pub fn go_in(&mut self, t: usize, p: usize, loc: Loc, history: bool) {
        let pane = &mut self.tabs[t].panes[p];
        if let Loc::Folder(dir) = &loc
            && !dir.is_dir()
        {
            self.toast(&format!("Not found: {}", dir.display()), "", false);
            return;
        }
        if history && pane.loc != loc {
            let old = pane.loc.clone();
            pane.back.push(old);
            pane.fwd.clear();
        }
        pane.back_anim = false;
        pane.loc = loc;
        self.after_move(t, p);
    }

    pub fn back(&mut self) {
        let (t, p) = (self.cur, self.tabs[self.cur].active);
        let pane = &mut self.tabs[t].panes[p];
        if let Some(l) = pane.back.pop() {
            let old = std::mem::replace(&mut pane.loc, l);
            pane.want = old.path().map(|x| x.to_path_buf());
            pane.fwd.push(old);
            pane.back_anim = true;
            self.after_move(t, p);
        }
    }

    pub fn forward(&mut self) {
        let (t, p) = (self.cur, self.tabs[self.cur].active);
        let pane = &mut self.tabs[t].panes[p];
        if let Some(l) = pane.fwd.pop() {
            let old = std::mem::replace(&mut pane.loc, l);
            pane.back.push(old);
            pane.back_anim = false;
            self.after_move(t, p);
        }
    }

    pub fn up(&mut self) {
        let loc = self.pane().loc.clone();
        match loc {
            Loc::Folder(p) => {
                let parent = p.parent().map(|x| x.to_path_buf());
                let want = Some(p.clone());
                match parent {
                    Some(par) if par.as_os_str().len() > 0 => self.go(Loc::Folder(par)),
                    _ => self.go(Loc::ThisPc),
                }
                self.pane_mut().want = want;
                self.pane_mut().back_anim = true;
            }
            Loc::RecycleBin | Loc::Settings => self.go(Loc::ThisPc),
            Loc::ThisPc => {}
        }
    }

    fn after_move(&mut self, t: usize, p: usize) {
        {
            let pane = &mut self.tabs[t].panes[p];
            pane.search = None;
            pane.sel.clear();
            pane.focus = None;
            pane.anchor = None;
            pane.scroll = 0.0;
            pane.error = None;
            pane.watch = None;
        }
        if t == self.cur {
            self.path_edit = None;
            self.search_focus = false;
            self.inline = None;
            if let Some(w) = &self.whatis
                && !self.tabs[t].panes[p].loc.path().map(|d| w.item.path.starts_with(d)).unwrap_or(false)
            {
                self.whatis = None;
            }
        }
        self.load(t, p, false);
    }

    /// (Re)list a pane. `keep` = refresh in place (watch), keeps selection and scroll.
    pub fn load(&mut self, t: usize, p: usize, keep: bool) {
        let hidden = self.ui.hidden;
        let tab_id = self.tabs[t].id;
        let pane = &mut self.tabs[t].panes[p];
        pane.seq += 1;
        let seq = pane.seq;
        if !keep {
            pane.loading = true;
            pane.shown_at = Instant::now();
        }
        let loc = pane.loc.clone();
        match &loc {
            Loc::Folder(dir) => {
                let dir = dir.clone();
                if pane.watch.is_none() {
                    let bg = self.bg.clone();
                    let hw = self.hwnd.0 as isize;
                    let d2 = dir.clone();
                    pane.watch = fmcore::watch::Watcher::start(&dir, false, move |_c| post_bg(&bg, hw, Msg::Changed { dir: d2.clone() })).ok();
                }
                let t0 = self.started;
                self.spawn(move || {
                    let s = Instant::now();
                    let res = fmcore::list::list_dir(&dir, hidden).map(|v| v.iter().map(|e| Item::from_entry(&dir, e)).collect::<Vec<_>>());
                    let _ = t0;
                    Msg::Listed { tab: tab_id, pane: p, seq, res, ms: s.elapsed().as_secs_f64() * 1000.0 }
                });
            }
            Loc::ThisPc => {
                let items = self
                    .drives
                    .iter()
                    .map(|d| {
                        let letter = d.root.trim_end_matches('\\').to_string();
                        Item {
                            name: if d.label.is_empty() { drive_label(&letter) } else { format!("{} ({letter})", d.label) },
                            path: PathBuf::from(&d.root),
                            is_dir: true,
                            size: if d.total > 0 { Some(d.total) } else { None },
                            modified: 0,
                            attrs: 0,
                            kind: crate::ficons::Kind::Drive,
                            folder: if d.total > 0 { format!("{} free", crate::fmt::size(d.free)) } else { String::new() },
                            hl: vec![],
                            recycled: None,
                        }
                    })
                    .collect::<Vec<Item>>();
                let mut items = items;
                if self.look.show_pinned {
                    for (name, _, _, l) in &self.side {
                        if let Loc::Folder(dir) = l {
                            items.push(Item {
                                name: name.clone(),
                                path: dir.clone(),
                                is_dir: true,
                                size: None,
                                modified: 0,
                                attrs: 0,
                                kind: crate::ficons::Kind::Folder,
                                folder: dir.parent().map(|x| x.display().to_string()).unwrap_or_default(),
                                hl: vec![],
                                recycled: None,
                            });
                        }
                    }
                }
                self.post(Msg::Listed { tab: tab_id, pane: p, seq, res: Ok(items), ms: 0.0 });
            }
            Loc::RecycleBin => {
                let (items, recs) = recycle_items();
                self.recycle = recs;
                self.post(Msg::Listed { tab: tab_id, pane: p, seq, res: Ok(items), ms: 0.0 });
            }
            Loc::Settings => {
                pane.loading = false;
                self.refresh_settings();
            }
        }
        self.invalidate();
    }

    pub fn refresh_settings(&mut self) {
        self.spawn(|| {
            let running = fmcore::evipc::available();
            let total = if running { fmcore::evipc::query("", 0, app_req(), 1, false, Duration::from_secs(2)).map(|r| r.total).unwrap_or(0) } else { 0 };
            Msg::EvStatus { running, total }
        });
    }

    /// Folder sizes of `dir` from the Everything index: one subtree query, summed per child folder.
    /// Folder sizes of `dir`: one Everything subtree query summed per child folder. The index is trusted only when
    /// it knows every file directly in `dir` (else it is stale or excludes the place); folders it cannot answer for
    /// get a short walk each (whole job capped at 3 s). A folder stays empty rather than showing a wrong size.
    pub fn ask_sizes(&self, dir: &Path) {
        if !self.ui.sizes {
            return;
        }
        let dir = dir.to_path_buf();
        self.spawn(move || {
            let t0 = Instant::now();
            let mut sizes: HashMap<String, u64> = HashMap::new();
            let base = dir.to_string_lossy().trim_end_matches('\\').to_string();
            let (mut child_dirs, mut files_here) = (vec![], 0usize);
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        child_dirs.push(e.file_name().to_string_lossy().into_owned());
                    } else {
                        files_here += 1;
                    }
                }
            }
            if child_dirs.is_empty() {
                return Msg::Sizes { dir, sizes };
            }
            let q = format!("{} file:", fmcore::evipc::quote(&format!("{base}\\")));
            let res = fmcore::evipc::query(&q, 400_000, fmcore::evipc::REQ_PATH | fmcore::evipc::REQ_SIZE, 1, false, Duration::from_secs(4));
            trace(&format!("sizes {q}: {:?}", res.as_ref().map(|r| (r.total, r.items.len(), r.ms))));
            if let Ok(r) = res
                && r.total as usize <= r.items.len()
            {
                let pre = format!("{}\\", base.to_lowercase());
                let mut direct = 0usize;
                let mut sums: HashMap<String, u64> = HashMap::new();
                for it in &r.items {
                    let pl = it.path.to_lowercase();
                    if pl == base.to_lowercase() {
                        direct += 1;
                        continue;
                    }
                    if !pl.starts_with(&pre) {
                        continue;
                    }
                    let first = it.path[pre.len()..].split('\\').next().unwrap_or("").to_lowercase();
                    *sums.entry(first).or_insert(0) += it.size.unwrap_or(0);
                }
                if direct >= files_here {
                    sizes = sums;
                }
                trace(&format!("sizes: index has {direct} of {files_here} files here -> {}", if direct >= files_here { "trusted" } else { "not trusted" }));
            }
            for name in child_dirs {
                let key = name.to_lowercase();
                if sizes.contains_key(&key) {
                    continue;
                }
                let p = dir.join(&name);
                let empty = std::fs::read_dir(&p).map(|mut r| r.next().is_none()).unwrap_or(false);
                if empty {
                    sizes.insert(key, 0);
                    continue;
                }
                let left = Duration::from_secs(3).saturating_sub(t0.elapsed());
                if left.is_zero() {
                    break;
                }
                let f = fmcore::info::folder_size(&p, false, left.min(Duration::from_millis(800)));
                if f.complete {
                    sizes.insert(key, f.bytes);
                }
            }
            Msg::Sizes { dir, sizes }
        });
    }

    pub fn start_search(&mut self) {
        let here = self.pane().dir();
        let tab_id = self.tab().id;
        let p = self.tab().active;
        let Some(s) = self.pane_mut().search.as_mut() else { return };
        s.seq += 1;
        let seq = s.seq;
        let words = s.field.text.trim().to_string();
        if words.is_empty() {
            s.items.clear();
            s.total = 0;
            self.invalidate();
            return;
        }
        s.busy = true;
        let q = s.query(here.as_deref());
        let meaning = s.meaning;
        self.spawn(move || {
            if meaning {
                return Msg::Found { tab: tab_id, pane: p, seq, res: crate::input::meaning_search(&words, 200) };
            }
            let r = fmcore::evipc::query(&q, 300, fmcore::evipc::REQ_NAME | fmcore::evipc::REQ_PATH | fmcore::evipc::REQ_SIZE | fmcore::evipc::REQ_MODIFIED | fmcore::evipc::REQ_ATTRS, fmcore::evipc::SORT_DATE_MODIFIED_DESC, false, Duration::from_secs(3));
            let res = match r {
                Ok(r) => Ok((
                    r.total,
                    r.items
                        .into_iter()
                        .map(|i| Item {
                            kind: crate::ficons::kind_of(&i.name, i.is_dir),
                            path: PathBuf::from(i.full()),
                            hl: match_ranges(&i.name, &words),
                            size: i.size,
                            modified: i.modified,
                            attrs: i.attrs,
                            folder: i.path.clone(),
                            is_dir: i.is_dir,
                            name: i.name,
                            recycled: None,
                        })
                        .collect(),
                )),
                // Everything not running: the folder walk from fmcore (slower, this folder only)
                Err(_) => {
                    let q = fmcore::search::Query { words: words.split_whitespace().map(|s| s.to_string()).collect(), in_dir: here.clone(), top: 300, engine: fmcore::search::Engine::Walk, walk_limit_ms: 3000, ..Default::default() };
                    fmcore::search::search(&q).map(|f| {
                        (
                            f.hits.len() as u32,
                            f.hits
                                .into_iter()
                                .map(|h| Item {
                                    kind: crate::ficons::kind_of(&h.name, h.is_dir),
                                    path: PathBuf::from(&h.path),
                                    hl: match_ranges(&h.name, &words),
                                    size: h.size,
                                    modified: h.modified,
                                    attrs: h.attrs,
                                    folder: Path::new(&h.path).parent().map(|p| p.display().to_string()).unwrap_or_default(),
                                    is_dir: h.is_dir,
                                    name: h.name,
                                    recycled: None,
                                })
                                .collect(),
                        )
                    })
                }
            };
            Msg::Found { tab: tab_id, pane: p, seq, res }
        });
    }

    pub fn toast(&mut self, text: &str, bold: &str, undo: bool) {
        self.toast = Some(Toast { text: text.into(), bold: bold.into(), undo, at: Instant::now(), hover: false });
        self.timer(T_TOAST, 500);
        self.anim();
        self.invalidate();
    }

    // ------------------------------------------------------------------ background messages
    pub fn take_bg(&mut self) {
        let msgs: Vec<Msg> = std::mem::take(&mut *self.bg.lock().unwrap());
        for m in msgs {
            self.apply(m);
        }
        self.invalidate();
    }

    fn find_pane(&mut self, tab: u64, pane: usize) -> Option<(usize, usize)> {
        let t = self.tabs.iter().position(|t| t.id == tab)?;
        (pane < self.tabs[t].panes.len()).then_some((t, pane))
    }

    fn apply(&mut self, m: Msg) {
        match m {
            Msg::Listed { tab, pane, seq, res, ms } => {
                let Some((t, p)) = self.find_pane(tab, pane) else { return };
                if self.tabs[t].panes[p].seq != seq {
                    return;
                }
                let dir = self.tabs[t].panes[p].dir();
                let sizes = dir.as_ref().and_then(|d| self.sizes.get(d)).cloned();
                let pn = &mut self.tabs[t].panes[p];
                pn.loading = false;
                match res {
                    Ok(mut items) => {
                        if let Some(s) = &sizes {
                            for it in items.iter_mut().filter(|i| i.is_dir) {
                                it.size = s.get(&it.name.to_lowercase()).copied();
                            }
                        }
                        pn.items = items;
                        pn.sort_items();
                        pn.error = None;
                        let names: HashSet<PathBuf> = pn.items.iter().map(|i| i.path.clone()).collect();
                        pn.sel.retain(|s| names.contains(s));
                        if let Some(w) = pn.want.take()
                            && let Some(i) = pn.items.iter().position(|it| fmcore::util::path_eq(&it.path, &w))
                        {
                            pn.sel.clear();
                            pn.sel.insert(pn.items[i].path.clone());
                            pn.focus = Some(i);
                            pn.anchor = Some(i);
                            pn.kb = true;
                            self.scroll_to(t, p, i);
                        }
                        if self.first_list_ms.is_none() {
                            self.first_list_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
                            trace(&format!("first list {:.1} ms (list itself {ms:.1} ms)", self.first_list_ms.unwrap()));
                        }
                        if let Some(d) = dir {
                            self.ask_sizes(&d);
                        }
                        if let Some(pr) = self.pending_rename.take() {
                            if self.tabs[t].panes[p].items.iter().any(|i| i.path == pr) {
                                self.inline_rename_pub(pr);
                            } else {
                                self.pending_rename = Some(pr);
                            }
                        }
                    }
                    Err(e) => {
                        pn.items.clear();
                        pn.error = Some(e);
                    }
                }
                self.anim();
            }
            Msg::Sizes { dir, sizes } => {
                for tab in &mut self.tabs {
                    for pn in &mut tab.panes {
                        if pn.dir().map(|d| fmcore::util::path_eq(&d, &dir)).unwrap_or(false) {
                            for it in pn.items.iter_mut().filter(|i| i.is_dir) {
                                it.size = sizes.get(&it.name.to_lowercase()).copied();
                            }
                            if pn.sort.0 == SortCol::Size {
                                pn.sort_items();
                            }
                        }
                    }
                }
                self.sizes.insert(dir, sizes);
            }
            Msg::Found { tab, pane, seq, res } => {
                let Some((t, p)) = self.find_pane(tab, pane) else { return };
                let Some(s) = self.tabs[t].panes[p].search.as_mut() else { return };
                if s.seq != seq {
                    return;
                }
                s.busy = false;
                match res {
                    Ok((total, items)) => {
                        s.total = total;
                        s.items = items;
                        s.note.clear();
                    }
                    Err(e) => {
                        s.items.clear();
                        s.total = 0;
                        s.note = e;
                    }
                }
                let pn = &mut self.tabs[t].panes[p];
                pn.scroll = 0.0;
                let first = pn.rows().first().map(|i| i.path.clone());
                pn.sel.clear();
                pn.focus = None;
                if let Some(f) = first {
                    pn.sel.insert(f);
                    pn.focus = Some(0);
                    pn.anchor = Some(0);
                }
            }
            Msg::Drives(d) => {
                self.drives = d;
                for t in 0..self.tabs.len() {
                    for p in 0..self.tabs[t].panes.len() {
                        if self.tabs[t].panes[p].loc == Loc::ThisPc {
                            self.load(t, p, true);
                        }
                    }
                }
            }
            Msg::Recent(r) => self.recent = r,
            Msg::Thumb { path, img } => {
                self.thumbs.insert(path.clone(), img);
                self.bitmaps.remove(&path);
            }
            Msg::Changed { dir } => {
                let _ = dir;
                self.timer(T_RELIST, 180);
            }
            Msg::ShellMenu { seq, items } => {
                if let Some(m) = self.menu.as_mut()
                    && m.seq == seq
                {
                    m.loading = false;
                    m.shell = items;
                    crate::input::merge_shell_items(m);
                }
            }
            Msg::Preview { data } => {
                if let Some(pv) = self.pv.as_mut()
                    && pv.path == data.path
                {
                    self.bitmaps.remove(&PathBuf::from(format!("pv:{}", data.path.display())));
                    pv.data = Some(data);
                }
            }
            Msg::WhatIs { path, res } => {
                if let Some(w) = self.whatis.as_mut()
                    && w.item.path == path
                {
                    w.answer = Some(res);
                }
            }
            Msg::Names { seq, res } => {
                if let Some(r) = self.rename.as_mut()
                    && r.seq == seq
                {
                    r.ai_busy = false;
                    match res {
                        Ok(n) => {
                            r.ai = Some(n);
                            r.ai_err = None;
                        }
                        Err(e) => r.ai_err = Some(e),
                    }
                    crate::input::rename_preview(self);
                }
            }
            Msg::Toast { text, undo } => self.toast(&text, "", undo),
            Msg::Refresh => self.refresh_all(),
            Msg::EvStatus { running, total } => {
                self.set.ev_running = Some(running);
                self.set.ev_total = total;
            }
            Msg::KeyTest(r) => {
                self.set.msg_ok = r.is_ok();
                self.set.msg = Some(match r {
                    Ok(()) => "The key works".into(),
                    Err(e) => e,
                });
            }
            Msg::AiFlags { ai, meaning } => {
                self.ai_on = ai;
                self.meaning_on = meaning;
            }
            Msg::OpenWith { path, apps } => {
                self.open_with = Some((path, apps));
                crate::input::show_open_with(self);
            }
        }
    }

    pub fn refresh_ai_flags(&mut self) {
        self.ai_on = crate::ai::enabled();
        self.meaning_on = crate::models::semsearch_enabled();
    }

    pub fn refresh_all(&mut self) {
        for t in 0..self.tabs.len() {
            for p in 0..self.tabs[t].panes.len() {
                if matches!(self.tabs[t].panes[p].loc, Loc::Folder(_) | Loc::RecycleBin) {
                    self.load(t, p, true);
                }
            }
        }
    }

    pub fn scroll_to(&mut self, t: usize, p: usize, i: usize) {
        let (y, h, view_h) = crate::paint::row_geom(self, t, p, i);
        let pane = &mut self.tabs[t].panes[p];
        if y < pane.scroll {
            pane.scroll = y;
        } else if y + h > pane.scroll + view_h {
            pane.scroll = y + h - view_h;
        }
        pane.scroll = pane.scroll.max(0.0);
    }

    /// Bitmap for a thumbnail (made for the current render target).
    pub fn thumb_bitmap(&mut self, path: &Path) -> Option<ID2D1Bitmap> {
        if self.bmp_target != self.gfx.target_id() {
            self.bitmaps.clear();
            self.bmp_target = self.gfx.target_id();
        }
        if let Some(b) = self.bitmaps.get(path) {
            return b.clone();
        }
        match self.thumbs.get(path) {
            Some(Some((w, h, px))) => {
                let b = self.gfx.make_bitmap(*w, *h, px);
                self.bitmaps.insert(path.to_path_buf(), b.clone());
                b
            }
            Some(None) => None,
            None => {
                if !self.thumb_asked.contains(path) {
                    self.thumb_asked.insert(path.to_path_buf());
                    let (m, cv) = &*self.thumb_q;
                    m.lock().unwrap().push(path.to_path_buf());
                    cv.notify_one();
                }
                None
            }
        }
    }

    // ------------------------------------------------------------------ screenshot
    pub fn shot(&mut self, path: &str) -> Result<(), String> {
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            let dc = CreateCompatibleDC(None);
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).map_err(|e| e.message())?;
            let old = SelectObject(dc, bmp.into());
            if !self.gfx.begin_offscreen(dc, w as u32, h as u32) {
                return Err("offscreen target failed".into());
            }
            self.shooting = true;
            self.paint();
            self.shooting = false;
            self.gfx.end_offscreen();
            let _ = GdiFlush();
            let n = (w * h * 4) as usize;
            let mut px = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            self.invalidate();
            let img = fmcore::thumb::Image { width: w as u32, height: h as u32, bgra: px };
            fmcore::thumb::save_png(&img, Path::new(path))
        }
    }

    pub fn state_json(&self) -> serde_json::Value {
        use serde_json::json;
        let q = self.jobs.0.lock().unwrap();
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .map(|t| {
                json!({
                    "title": t.title(), "view": t.view.name(), "split": t.split(), "active_pane": t.active,
                    "panes": t.panes.iter().map(|p| json!({
                        "location": p.loc.display(),
                        "items": p.items.len(),
                        "rows": p.rows().len(),
                        "first_rows": p.rows().iter().take(12).map(|i| i.name.clone()).collect::<Vec<_>>(),
                        "selected": p.selected().iter().map(|i| i.name.clone()).collect::<Vec<_>>(),
                        "search": p.search.as_ref().map(|s| json!({"text": s.field.text, "this_pc": s.pc, "total": s.total, "busy": s.busy, "meaning": s.meaning, "note": s.note})),
                        "loading": p.loading, "error": p.error, "scroll": p.scroll,
                        "folder_sizes_known": p.items.iter().filter(|i| i.is_dir && i.size.is_some()).count(),
                        "back": p.back.len(), "forward": p.fwd.len(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let overlay = if self.pv.is_some() {
            "preview"
        } else if self.menu.is_some() {
            "menu"
        } else if self.rename.is_some() {
            "rename"
        } else if self.palette.is_some() {
            "palette"
        } else if self.xfer_open {
            "transfers"
        } else if self.drop.is_some() {
            "dropdown"
        } else {
            ""
        };
        json!({
            "ok": true, "tabs": tabs, "current_tab": self.cur, "overlay": overlay,
            "menu_items": self.menu.as_ref().map(|m| m.items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()),
            "dropdown": self.drop.as_ref().map(|d| d.items.iter().map(|i| i.label.clone()).collect::<Vec<_>>()),
            "path_edit": self.path_edit.as_ref().map(|f| f.text.clone()),
            "inline_rename": self.inline.as_ref().map(|f| f.field.text.clone()),
            "whatis": self.whatis.as_ref().map(|w| json!({"path": w.item.path, "answer": w.answer.as_ref().map(|a| match a { Ok(a) => json!({"what": a.what, "actions": a.actions}), Err(e) => json!({"error": e}) })})),
            "rename_rows": self.rename.as_ref().map(|r| r.rows.iter().map(|x| format!("{} -> {}", x.0, x.1)).collect::<Vec<_>>()),
            "rename_problem": self.rename.as_ref().and_then(|r| r.problem.clone()),
            "preview": self.pv.as_ref().map(|p| json!({"path": p.path, "loaded": p.data.is_some(), "kind": p.data.as_ref().map(|d| format!("{:?}", d.kind)), "meta": p.data.as_ref().map(|d| d.meta.clone())})),
            "toast": self.toast.as_ref().map(|t| format!("{}{}", t.text, t.bold)),
            "jobs": q.jobs.iter().map(|j| json!({"id": j.id, "title": j.title(), "state": format!("{:?}", j.state), "done": j.done, "total": j.total, "clashes": j.open_clashes(), "identical_skipped": j.identical, "errors": j.errors, "dest": j.dest})).collect::<Vec<_>>(),
            "theme": if self.t.dark { "dark" } else { "light" }, "mica": self.mica, "hwnd": self.hwnd.0 as isize,
            "look": self.look.to_json(), "marquee": self.marquee.is_some(),
            "size": [self.w, self.h], "rail": self.rail(),
            "drives": self.drives.len(), "sidebar": self.side.iter().map(|s| s.0.clone()).collect::<Vec<_>>(),
            "ai_buttons": self.ai_on, "meaning_button": self.meaning_on,
            "first_paint_ms": self.first_paint_ms, "first_list_ms": self.first_list_ms,
            "last_error": self.last_error,
        })
    }
}

pub fn app_req() -> u32 {
    fmcore::evipc::REQ_NAME
}

pub fn post_bg(bg: &Arc<Mutex<Vec<Msg>>>, hw: isize, m: Msg) {
    bg.lock().unwrap().push(m);
    unsafe {
        let _ = PostMessageW(Some(HWND(hw as *mut _)), WM_BG, WPARAM(0), LPARAM(0));
    }
}

pub fn lparam_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

pub fn set_dark_frame(hwnd: HWND, dark: bool) {
    let v: i32 = dark as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v as *const i32 as *const _, 4);
    }
}

pub fn mods() -> (bool, bool, bool) {
    unsafe { (GetKeyState(VK_CONTROL.0 as i32) < 0, GetKeyState(VK_SHIFT.0 as i32) < 0, GetKeyState(VK_MENU.0 as i32) < 0) }
}

fn app_of(hw: isize) -> Option<&'static mut App> {
    unsafe {
        let p = GetWindowLongPtrW(HWND(hw as *mut _), GWLP_USERDATA) as *mut App;
        if p.is_null() { None } else { Some(&mut *p) }
    }
}

fn sink_of(hw: isize) -> Option<&'static mut dyn crate::dnd::Sink> {
    app_of(hw).map(|a| a as &mut dyn crate::dnd::Sink)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if ptr.is_null() {
            if msg == WM_NCCALCSIZE && wp.0 != 0 {
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let app = &mut *ptr;
        let s = app.scale();
        match msg {
            WM_NCCALCSIZE if wp.0 != 0 => {
                if IsZoomed(hwnd).as_bool() {
                    let p = &mut *(lp.0 as *mut NCCALCSIZE_PARAMS);
                    let dpi = GetDpiForWindow(hwnd);
                    let f = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    let fy = GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                    p.rgrc[0].left += f;
                    p.rgrc[0].right -= f;
                    p.rgrc[0].top += fy;
                    p.rgrc[0].bottom -= fy;
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let (sx, sy) = lparam_xy(lp);
                let mut pt = POINT { x: sx, y: sy };
                let _ = ScreenToClient(hwnd, &mut pt);
                let (x, y) = (pt.x as f32 / s, pt.y as f32 / s);
                let maxed = IsZoomed(hwnd).as_bool();
                let b = 6.0;
                if !maxed {
                    let (l, r, t, bt) = (x < b, x > app.w - b, y < b, y > app.h - b);
                    let code = match (l, r, t, bt) {
                        (true, _, true, _) => HTTOPLEFT,
                        (_, true, true, _) => HTTOPRIGHT,
                        (true, _, _, true) => HTBOTTOMLEFT,
                        (_, true, _, true) => HTBOTTOMRIGHT,
                        (true, ..) => HTLEFT,
                        (_, true, ..) => HTRIGHT,
                        (_, _, true, _) => HTTOP,
                        (.., true) => HTBOTTOM,
                        _ => 0,
                    };
                    if code != 0 {
                        return LRESULT(code as isize);
                    }
                }
                if y < TITLE_H {
                    return LRESULT(match app.hit_at(x, y) {
                        Hit::Min => HTMINBUTTON,
                        Hit::Max => HTMAXBUTTON,
                        Hit::Close => HTCLOSE,
                        Hit::None => {
                            if app.menu.is_some() || app.drop.is_some() || app.palette.is_some() {
                                HTCLIENT
                            } else {
                                HTCAPTION
                            }
                        }
                        _ => HTCLIENT,
                    } as isize);
                }
                LRESULT(HTCLIENT as isize)
            }
            WM_NCMOUSEMOVE => {
                let h = match wp.0 as u32 {
                    HTMINBUTTON => Hit::Min,
                    HTMAXBUTTON => Hit::Max,
                    HTCLOSE => Hit::Close,
                    _ => Hit::None,
                };
                if h != app.nc_hover {
                    app.nc_hover = h;
                    app.hover = Hit::None;
                    app.nc_hover_changed();
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                    app.invalidate();
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCMOUSELEAVE => {
                app.nc_hover = Hit::None;
                app.nc_hover_changed();
                app.invalidate();
                LRESULT(0)
            }
            WM_NCLBUTTONDOWN => match wp.0 as u32 {
                HTMINBUTTON | HTMAXBUTTON | HTCLOSE => {
                    app.nc_down = app.nc_hover;
                    app.invalidate();
                    LRESULT(0)
                }
                _ => {
                    app.close_popups();
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            },
            WM_NCLBUTTONUP => {
                let d = std::mem::replace(&mut app.nc_down, Hit::None);
                match (wp.0 as u32, d) {
                    (HTMINBUTTON, Hit::Min) => {
                        let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    }
                    (HTMAXBUTTON, Hit::Max) => {
                        let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
                    }
                    (HTCLOSE, Hit::Close) => {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    _ => return DefWindowProcW(hwnd, msg, wp, lp),
                }
                LRESULT(0)
            }
            WM_NCLBUTTONDBLCLK => DefWindowProcW(hwnd, msg, wp, lp),
            // no WS_SYSMENU (it makes DWM draw its own caption buttons under ours): the window menu by hand
            WM_NCRBUTTONUP if wp.0 as u32 == HTCAPTION => {
                let (x, y) = lparam_xy(lp);
                show_window_menu(hwnd, x, y);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_xy(lp);
                app.mouse_move(x as f32 / s, y as f32 / s, wp.0 as u32 & 1 != 0);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                app.hover = Hit::None;
                app.hov_f.only(None);
                app.anim();
                app.invalidate();
                LRESULT(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_xy(lp);
                let (c, sh, _) = mods();
                SetCapture(hwnd);
                app.mouse_down(x as f32 / s, y as f32 / s, c, sh, msg == WM_LBUTTONDBLCLK);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = lparam_xy(lp);
                app.mouse_up(x as f32 / s, y as f32 / s);
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                let (x, y) = lparam_xy(lp);
                app.middle_click(x as f32 / s, y as f32 / s);
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let (x, y) = lparam_xy(lp);
                app.right_click(x as f32 / s, y as f32 / s);
                LRESULT(0)
            }
            WM_XBUTTONUP => {
                if (wp.0 >> 16) & 0xFFFF == 1 {
                    app.back();
                } else {
                    app.forward();
                }
                app.invalidate();
                LRESULT(1)
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
                let mut pt = POINT { x: lparam_xy(lp).0, y: lparam_xy(lp).1 };
                let _ = ScreenToClient(hwnd, &mut pt);
                app.wheel(pt.x as f32 / s, pt.y as f32 / s, d);
                LRESULT(0)
            }
            WM_SYSKEYDOWN if wp.0 == 0x73 => {
                // Alt+F4 (no system menu to do it for us)
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            WM_SYSCHAR if wp.0 == 0x20 => {
                // Alt+Space: the window menu under the top-left corner
                let mut pt = POINT { x: 8, y: (TITLE_H * s) as i32 };
                let _ = ClientToScreen(hwnd, &mut pt);
                show_window_menu(hwnd, pt.x, pt.y);
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                if app.marquee.is_some() || app.side_drag.is_some() {
                    app.mouse_up(app.mx, app.my);
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let (c, sh, alt) = mods();
                if app.key(wp.0 as u16, c, sh, alt || msg == WM_SYSKEYDOWN) {
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CHAR => {
                if let Some(c) = char::from_u32(wp.0 as u32) {
                    app.char_in(c);
                }
                LRESULT(0)
            }
            WM_SYSCHAR => LRESULT(0),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                app.paint();
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                app.invalidate();
                LRESULT(0)
            }
            WM_TIMER => {
                app.on_timer(wp.0);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let mi = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                // small enough for a narrow snap column: the sidebar hides, columns drop, the list can shrink to nothing
                mi.ptMinTrackSize = POINT { x: (360.0 * dpi) as i32, y: (200.0 * dpi) as i32 };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
                app.apply_theme();
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xFFFF) == 0 {
                    app.close_popups();
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xFFFF) as u32 == HTCLIENT {
                    let cur = match app.hover {
                        _ if app.side_drag.is_some() => IDC_SIZEWE,
                        _ if app.col_drag.is_some() => IDC_SIZEWE,
                        Hit::SideEdge | Hit::ColEdge(..) => IDC_SIZEWE,
                        Hit::Omni => IDC_IBEAM,
                        _ if app.path_edit.is_some() && matches!(app.hover, Hit::OmniSearch) => IDC_IBEAM,
                        _ => IDC_ARROW,
                    };
                    SetCursor(LoadCursorW(None, cur).ok());
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            m if m == WM_BG || m == WM_JOB => {
                app.take_bg();
                if m == WM_JOB {
                    app.jobs_changed();
                }
                LRESULT(0)
            }
            m if m == WM_IPC => {
                let jobs: Vec<(String, std::sync::mpsc::Sender<String>)> = app.ipc_in.lock().unwrap().drain(..).collect();
                for (line, tx) in jobs {
                    let r = crate::input::ipc(app, &line);
                    let _ = tx.send(r);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                // remember where we were (start page "Last folder")
                if let Some(p) = app.pane().loc.path() {
                    app.look.last = p.display().to_string();
                    app.save_look();
                }
                crate::dnd::revoke(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// The window menu (Restore, Move, Size, Minimize, Maximize, Close) at a screen point.
fn show_window_menu(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let m = GetSystemMenu(hwnd, false);
        if m.is_invalid() {
            return;
        }
        let maxed = IsZoomed(hwnd).as_bool();
        let st = |on: bool| if on { MF_ENABLED } else { MF_GRAYED };
        let _ = EnableMenuItem(m, SC_RESTORE, MF_BYCOMMAND | st(maxed));
        let _ = EnableMenuItem(m, SC_MOVE, MF_BYCOMMAND | st(!maxed));
        let _ = EnableMenuItem(m, SC_SIZE, MF_BYCOMMAND | st(!maxed));
        let _ = EnableMenuItem(m, SC_MAXIMIZE, MF_BYCOMMAND | st(!maxed));
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, x, y, None, hwnd, None);
        if cmd.0 != 0 {
            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd.0 as usize), LPARAM(0));
        }
    }
}

pub struct Opts {
    pub offscreen: bool,
    pub software: bool,
    pub theme: Option<bool>,
    pub size: Option<(f32, f32)>,
    pub path: Option<String>,
    pub no_mica: bool,
}

/// Create the window and run the message loop.
pub fn run(o: Opts) -> i32 {
    let started = Instant::now();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let cls = w!("WinToolsFiles");
        let cfg = fmcore::pins::config();
        let ui = &cfg["ui"];
        let uis = UiSettings {
            sizes: ui["folder_sizes"].as_bool().unwrap_or(true),
            ext: ui["extensions"].as_bool().unwrap_or(true),
            hidden: ui["hidden"].as_bool().unwrap_or(false),
            ask_delete: ui["ask_delete"].as_bool().unwrap_or(false),
            scope_pc: ui["scope_pc"].as_bool().unwrap_or(true),
        };
        let look = crate::look::Look::load();
        let dark = o.theme.unwrap_or(match look.theme {
            1 => false,
            2 => true,
            _ => theme::system_dark(),
        });
        let t = theme::make(dark, look.accent);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(inst.into()), PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: cls,
            hbrBackground: CreateSolidBrush(COLORREF(((t.layer.2 * 255.0) as u32) << 16 | ((t.layer.1 * 255.0) as u32) << 8 | (t.layer.0 * 255.0) as u32)),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let dpi = GetDpiForSystem() as f32 / 96.0;
        let (dw, dh) = o.size.unwrap_or((1180.0, 760.0));
        let (w, h) = ((dw * dpi) as i32, (dh * dpi) as i32);
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        // No WS_SYSMENU: with it DWM paints its own caption buttons into the extended frame, under our buttons
        // (the doubled maximize box and heavy x the owner saw). The window menu is shown by hand instead.
        let mut style = WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_SYSMENU.0) | WS_CLIPCHILDREN;
        if std::env::var("WFILES_TEST").is_ok() && std::env::var("WFILES_TEST_SYSMENU").is_ok() {
            // tests only: the old style, to compare what DWM draws
            style |= WS_SYSMENU;
        }
        // --offscreen (tests): far off every monitor, no taskbar button, never activated
        let (ex, x, y) = if o.offscreen { (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, -20000, -20000) } else { (WINDOW_EX_STYLE(0), (sw - w) / 2, (sh - h) / 2) };
        let Ok(hwnd) = CreateWindowExW(ex, cls, w!("Rust File Explorer"), style, x, y, w, h, None, None, Some(inst.into()), None) else { return 1 };
        set_dark_frame(hwnd, t.dark);
        // Backdrop (Windows 11 22H2+): DWM draws Mica / Mica Alt / Acrylic where the window is transparent
        let build = os_build();
        let can_mica = !o.no_mica && build >= 22621 && std::env::var("WFILES_NO_MICA").is_err();
        let mica = can_mica && look.backdrop != 3;
        if can_mica {
            let v: i32 = look.dwm_backdrop();
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &v as *const i32 as *const _, 4);
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
        } else {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });
        }
        trace(&format!("{:.1} ms: hwnd", started.elapsed().as_secs_f64() * 1000.0));
        let Ok(mut gfx) = Gfx::new(hwnd, o.software, mica) else { return 1 };
        gfx.corner = if look.square { 0.0 } else { 1.0 };
        let queue = jobs::new_queue();
        let app = Box::new(App {
            hwnd,
            gfx,
            t,
            ui: uis,
            tabs: vec![],
            cur: 0,
            next_id: 0,
            w: dw,
            h: dh,
            hits: vec![],
            hover: Hit::None,
            nc_hover: Hit::None,
            nc_down: Hit::None,
            pressed: Hit::None,
            mx: -1.0,
            my: -1.0,
            drives: vec![],
            recent: vec![],
            side: vec![],
            path_edit: None,
            search_focus: false,
            menu: None,
            drop: None,
            sub: None,
            pv: None,
            xfer_open: false,
            palette: None,
            rename: None,
            whatis: None,
            toast: None,
            inline: None,
            drag_in: None,
            press: None,
            drag_scroll: None,
            jobs: queue.clone(),
            bg: Arc::new(Mutex::new(vec![])),
            ipc_in: Arc::new(Mutex::new(vec![])),
            thumbs: HashMap::new(),
            bitmaps: HashMap::new(),
            bmp_target: 0,
            thumb_q: Arc::new((Mutex::new(vec![]), Condvar::new())),
            thumb_asked: HashSet::new(),
            sizes: HashMap::new(),
            menu_gen: 0,
            recycle: vec![],
            set: SetState { ev_running: None, ev_total: 0, msg: None, msg_ok: false },
            ai_on: false,
            meaning_on: false,
            open_with: None,
            started,
            first_paint_ms: None,
            first_list_ms: None,
            shooting: false,
            mica,
            caret_on: true,
            forced_dark: o.theme,
            last_error: None,
            key_field: Field::default(),
            key_focus: false,
            confirm: None,
            pending_rename: None,
            toast_job: None,
            done_seen: HashSet::new(),
            dragging_out: false,
            look,
            hov_f: crate::motion::Fades::new(crate::motion::FAST, crate::motion::NORMAL),
            press_f: crate::motion::Fades::new(60.0, crate::motion::NORMAL),
            menu_out: None,
            drop_out: None,
            marquee: None,
            press_ws: false,
            side_drag: None,
            col_drag: None,
            side_open: false,
            test_mods: None,
            settings_h: 0.0,
            caret_timer: false,
            no_hits: false,
            offscreen: o.offscreen,
            can_mica,
            accent_field: None,
        });
        let ptr = Box::into_raw(app);
        let app = &mut *ptr;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        // first location: the argument (a folder, or a file to select), else the start page from Personalize
        let start = match app.look.start.as_str() {
            "thispc" => Loc::ThisPc,
            "last" => Some(PathBuf::from(&app.look.last)).filter(|p| p.is_dir()).map(Loc::Folder).unwrap_or(Loc::ThisPc),
            p => Some(PathBuf::from(p)).filter(|p| p.is_dir()).map(Loc::Folder).unwrap_or(Loc::ThisPc),
        };
        let (loc, select) = match o.path.as_deref().map(|p| p.trim_matches('"').trim_start_matches("/select,")) {
            Some(p) if Path::new(p).is_dir() => (Loc::Folder(fmcore::util::abs(p)), None),
            Some(p) if Path::new(p).is_file() => (Loc::Folder(fmcore::util::abs(p).parent().map(|x| x.to_path_buf()).unwrap_or_default()), Some(fmcore::util::abs(p))),
            _ => (start, None),
        };
        trace(&format!("{:.1} ms: app struct", started.elapsed().as_secs_f64() * 1000.0));
        app.build_side();
        app.new_tab(loc);
        trace(&format!("{:.1} ms: side + tab", started.elapsed().as_secs_f64() * 1000.0));
        app.tabs[0].panes[0].want = select;
        // drives (with a time limit for dead network shares) and recent files, off the UI thread
        app.spawn(|| Msg::Drives(fmcore::shell::drives_within(Duration::from_millis(1500))));
        app.spawn(|| Msg::Recent(fmcore::pins::recent(4).unwrap_or_default()));
        // AI and meaning-search switches read files: keep them off the first frame
        app.spawn(|| Msg::AiFlags { ai: crate::ai::enabled(), meaning: crate::models::semsearch_enabled() });
        // thumbnails worker (newest request first)
        {
            let q = app.thumb_q.clone();
            let bg = app.bg.clone();
            let hw = hwnd.0 as isize;
            let px = (128.0 * GetDpiForWindow(hwnd) as f32 / 96.0).round() as u32;
            std::thread::spawn(move || {
                let _c = fmcore::util::com();
                loop {
                    let path = {
                        let (m, cv) = &*q;
                        let mut g = m.lock().unwrap();
                        while g.is_empty() {
                            g = cv.wait(g).unwrap();
                        }
                        g.pop().unwrap()
                    };
                    let img = fmcore::thumb::thumbnail(&path, px.max(128), fmcore::thumb::ThumbKind::ThumbnailOnly).ok().map(|mut i| {
                        crate::preview::premultiply(&mut i.bgra);
                        (i.width, i.height, i.bgra)
                    });
                    post_bg(&bg, hw, Msg::Thumb { path, img });
                }
            });
        }
        // copy queue worker
        {
            let hw = hwnd.0 as isize;
            jobs::start(queue, move || {
                let _ = PostMessageW(Some(HWND(hw as *mut _)), WM_JOB, WPARAM(0), LPARAM(0));
            });
        }
        // control pipe for `wfm gui ...`
        {
            let ipc_in = app.ipc_in.clone();
            let hw = hwnd.0 as isize;
            std::thread::spawn(move || {
                fmcore::pipe::serve(|line| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    ipc_in.lock().unwrap().push((line, tx));
                    let _ = PostMessageW(Some(HWND(hw as *mut _)), WM_IPC, WPARAM(0), LPARAM(0));
                    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| r#"{"ok":false,"error":"window busy"}"#.into())
                })
            });
        }
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        trace(&format!("{:.1} ms: before show", started.elapsed().as_secs_f64() * 1000.0));
        let _ = ShowWindow(hwnd, if o.offscreen { SW_SHOWNOACTIVATE } else { SW_SHOW });
        trace(&format!("{:.1} ms: shown", started.elapsed().as_secs_f64() * 1000.0));
        let _ = UpdateWindow(hwnd);
        trace(&format!("{:.1} ms: shown + first paint", started.elapsed().as_secs_f64() * 1000.0));
        crate::dnd::register(hwnd, sink_of);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        drop(Box::from_raw(ptr));
        0
    }
}

fn os_build() -> u32 {
    // RtlGetVersion is not lied to by compatibility shims
    #[repr(C)]
    struct Osv {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd: [u16; 128],
    }
    unsafe extern "system" {
        fn RtlGetVersion(v: *mut Osv) -> i32;
    }
    let mut v = Osv { size: size_of::<Osv>() as u32, major: 0, minor: 0, build: 0, platform: 0, csd: [0; 128] };
    unsafe {
        RtlGetVersion(&mut v);
    }
    v.build
}
