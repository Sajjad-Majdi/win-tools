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

/// lParam of the key messages `wfm gui keys` posts (never produced by a real keyboard)
pub const TEST_KEY: isize = 0x7E57_0001;
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
/// size-map value of a folder with no entries at all
pub const EMPTY_DIR: u64 = u64::MAX;
/// tooltip delay (one shot)
pub const T_TIP: usize = 10;
/// one-shot: give start-up pages back once the window has settled
pub const T_TRIM: usize = 11;

/// Results of background work, applied on the UI thread.
pub enum Msg {
    Cleanup(u64, crate::cleanup_page::Update),
    Listed { tab: u64, pane: usize, seq: u64, res: Result<Vec<Item>, String>, ms: f64 },
    Sizes { dir: PathBuf, sizes: HashMap<String, u64>, done: bool },
    /// search results: Everything's matches and (ranked off the UI thread) the rows to show
    /// search results; `ai` = the AI's note when "AI Search" made them
    Found { tab: u64, pane: usize, seq: u64, res: Result<(u32, Vec<Item>), String>, rows: Vec<Item>, ai: Option<String> },
    Drives(Vec<Drive>),
    Recent(Vec<fmcore::pins::Recent>),
    Thumb { path: PathBuf, img: Option<(u32, u32, Vec<u8>)> },
    /// a real shell icon (cache key, pixel size, premultiplied pixels)
    Icon { key: String, px: u32, img: Option<crate::sicons::Img> },
    /// sub-folders that complete the typed path (address bar)
    PathSug { seq: u64, items: Vec<String> },
    /// the wallpaper grid (Accent = Follow desktop)
    Wall(Option<theme::Wall>),
    Changed { dir: PathBuf },
    /// the shell's own entries and the apps that open the file (with their icons), for the "Open with" submenu
    ShellMenu { seq: u64, items: Vec<fmcore::menu::MenuItem>, apps: Vec<(fmcore::shell::Handler, Option<fmcore::menu::Icon>)> },
    Preview { data: preview::Data },
    WhatIs { path: PathBuf, res: Result<crate::ai::FileAnswer, String> },
    Names { seq: u64, res: Result<Vec<String>, String> },
    Toast { text: String, undo: bool },
    /// an error for the user (red mark, short text)
    ToastErr(String),
    Refresh,
    EvStatus { running: bool, total: u32 },
    KeyTest(Result<(), String>),
    OpenWith { path: PathBuf, apps: Vec<fmcore::shell::Handler> },
    AiFlags { ai: bool },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Hit {
    Cleanup(crate::cleanup_page::Target),
    /// the search strip's "AI Search" button
    Ask,
    /// "Turn on AI Search" under empty results (AI features off)
    AiOffer,
    /// Settings > Search folder tree: tick box / expand arrow of row i
    PlaceChk(usize),
    PlaceExp(usize),
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
    MenuItem(usize),
    MenuQuick(u8),
    SubItem(usize),
    /// the pin action of an "Open with" app row
    SubPin(usize),
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
    /// an address bar suggestion
    Sug(usize),
    Block,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Cleanup(crate::cleanup_page::Command),
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
    /// paste into the selected folder (its own menu only)
    PasteInto,
    Eject,
    EmptyBin,
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
    /// tab menu (the tab is App::menu_tab)
    TabNew,
    TabClose,
    TabCloseOthers,
    TabCloseRight,
    TabDuplicate,
    /// sidebar menu (the item is App::menu_side)
    SideOpen,
    SideNewTab,
    SideUnpin,
    SideEject,
    SideCopyPath,
    SideProps,
    SideEmptyBin,
    /// "More options ›" (the shell's other verbs, Send to)
    MoreMenu,
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
    /// an "Open with" app row: it has the pin action ("Always open .ext files with ...") with this tooltip
    pub pin: Option<String>,
}

impl MItem {
    pub fn new(label: &str, glyph: u32, act: Act) -> MItem {
        MItem { label: label.into(), glyph, keys: String::new(), act, shell: None, value: 0, bold: false, enabled: true, sep: false, sub: false, checked: false, icon: None, swatch: None, pin: None }
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
    CleanupScope,
    CleanupAi,
    CleanupHidden,
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
    /// the shell entries are still loading (only the "Open with" submenu shows it; the menu never changes size)
    pub loading: bool,
    pub shell: Vec<fmcore::menu::MenuItem>,
    pub apps: Vec<(fmcore::shell::Handler, Option<std::sync::Arc<fmcore::menu::Icon>>)>,
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
    /// the button that opened it: a click on that button while it is open only closes it
    pub by: Hit,
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
    pub err: bool,
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
    pub cleanup: crate::cleanup_page::Page,
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub t: Theme,
    pub ui: UiSettings,
    /// Settings > Search: the rules for This PC searches
    pub sp: SearchPrefs,
    /// Settings > Search folder tree: open folders, their sub-folders (read once), the rows as painted
    pub place_open: HashSet<PathBuf>,
    pub place_kids: HashMap<PathBuf, Vec<PathBuf>>,
    pub place_rows: Vec<PathBuf>,
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
    /// the tab / sidebar item a tab or sidebar menu is for
    pub menu_tab: Option<usize>,
    /// attributes and change time of pinned folders (their own icons)
    pub dir_attr: HashMap<PathBuf, (u32, u64)>,
    /// the tooltip shown now: (target, text, where the pointer was)
    pub tip: Option<(Hit, String, f32, f32)>,
    /// address bar: folders that complete the typed path, the picked one, and which request they answer
    pub sug: Vec<String>,
    pub sug_sel: Option<usize>,
    pub sug_seq: u64,
    /// rows and tabs drawn this frame with the width their name had (for tooltips of cut names)
    pub name_w: Vec<(Hit, f32)>,
    pub menu_side: Option<Loc>,
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
    /// real file icons (shell), made off the UI thread
    pub icons: crate::sicons::Icons,
    /// Windows' app picker was opened: when the window is active again, file-type icons are made anew
    pub assoc_stale: bool,
    pub thumb_asked: HashSet<PathBuf>,
    pub sizes: HashMap<PathBuf, HashMap<String, u64>>,
    /// folders whose size pass is still running (their unknown cells show "…")
    pub sizes_busy: HashSet<PathBuf>,
    /// Accent = Follow desktop: the wallpaper as a small colour grid and its colour behind the window now
    pub wall: Option<theme::Wall>,
    pub desk: Option<crate::gfx::C>,
    /// icon rail scroll (short windows) and how far it can go
    pub rail_scroll: f32,
    pub rail_max: f32,
    /// sidebar (expanded or rail) scroll with momentum, its visible area, and "show the selected item" after a move
    pub side_sf: crate::motion::Follow,
    pub side_area: Rect,
    pub side_reveal: bool,
    /// scroll of a menu / dropdown / submenu taller than the window, keyed by when it opened
    pub pop_scroll: [(Option<Instant>, f32, f32); 3],
    pub menu_gen: u64,
    pub recycle: Vec<fmcore::recycle::Recycled>,
    pub set: SetState,
    pub ai_on: bool,
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
    /// Settings > AI relay: address, header name, secret (masked); which one has the caret (0 = none)
    pub relay_fields: [Field; 3],
    pub relay_focus: u8,
    pub confirm: Option<(String, Act)>,
    pub pending_rename: Option<PathBuf>,
    pub toast_job: Option<u64>,
    pub shot_hover: bool,
    /// handling a `wfm gui` command (synthetic mouse)
    pub synthetic: bool,
    pub done_seen: HashSet<u64>,
    /// jobs whose clash question was already shown
    pub asked_jobs: HashSet<u64>,
    pub dragging_out: bool,
    /// type-ahead text and when the last letter came
    pub ta_text: String,
    pub ta_at: Option<Instant>,
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
            let warp = unsafe { GetModuleHandleW(w!("d3d10warp.dll")).is_ok() };
            let _ = writeln!(f, "{msg}{}", if warp { "  [warp loaded]" } else { "" });
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

    /// Run a search on THE search thread. Everything answers one IPC query at a time and cannot cancel one, so a
    /// thread per key made old queries queue in front of the newest when typing fast. One thread; per pane only the
    /// newest request waits (a newer one replaces it), and it is sent when the running one is back.
    pub fn spawn_search(&self, tab: u64, pane: usize, f: impl FnOnce() -> Msg + Send + 'static) {
        static WORKER: std::sync::Once = std::sync::Once::new();
        let (m, cv) = &*SEARCH_SLOT;
        {
            let mut q = m.lock().unwrap();
            q.retain(|j| j.0 != (tab, pane));
            q.push(((tab, pane), Box::new(f), self.bg.clone(), self.hwnd.0 as isize));
        }
        cv.notify_one();
        WORKER.call_once(|| {
            std::thread::spawn(|| {
                let _c = fmcore::util::com();
                let (m, cv) = &*SEARCH_SLOT;
                loop {
                    let job = {
                        let mut q = m.lock().unwrap();
                        while q.is_empty() {
                            q = cv.wait(q).unwrap();
                        }
                        q.remove(0)
                    };
                    let (_, f, bg, hw) = job;
                    let msg = f();
                    post_bg(&bg, hw, msg);
                }
            });
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
        if self.shooting {
            // `wfm gui shot FILE --hover`: show the hovered item as fully hovered (tests of hover-only UI)
            if self.shot_hover && (self.hover == h || self.nc_hover == h) { 1.0 } else { 0.0 }
        } else {
            self.hov_f.get(h)
        }
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
        self.gfx.set_see_through(self.look.see_through());
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

    /// What is really used to draw the window: the backdrop DWM reports, the render target's alpha mode and the
    /// opacity of each panel (for `wfm gui state` and the transparency proof).
    pub fn surfaces(&self) -> serde_json::Value {
        let mut bd = -1i32;
        unsafe {
            let _ = DwmGetWindowAttribute(self.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &mut bd as *mut i32 as *mut _, 4);
        }
        let a = self.panel_alpha();
        let tint = self.tint();
        serde_json::json!({
            "dwm_backdrop": bd,
            "frame_extended": self.mica,
            "alpha_mode": if self.gfx.alpha_target { "premultiplied" } else { "ignore (opaque)" },
            "renderer": self.gfx.renderer,
            "titlebar": a, "tabs_active": a, "toolbar": a, "sidebar": a, "content": a, "cards": a, "menus": 1.0, "text": 1.0,
            "tint": {"hue": tint.hue.round(), "accent_chroma": (tint.chroma * 1000.0).round() / 1000.0, "source": if self.look.desktop { "desktop" } else if self.look.accent.is_some() { "accent" } else { "windows accent" }, "strength": crate::look::STRENGTHS[self.look.strength as usize % 3], "surface_chroma": (theme::surface_chroma(tint, self.t.dark, self.look.strength) * 1000.0).round() / 1000.0},
            "window": format!("#{:02X}{:02X}{:02X}", (self.t.mica_a.0 * 255.0).round() as u8, (self.t.mica_a.1 * 255.0).round() as u8, (self.t.mica_a.2 * 255.0).round() as u8),
            "layer": format!("#{:02X}{:02X}{:02X}", (self.t.layer.0 * 255.0).round() as u8, (self.t.layer.1 * 255.0).round() as u8, (self.t.layer.2 * 255.0).round() as u8),
        })
    }

    /// Opacity of every panel over the backdrop (1 when there is no backdrop to show).
    pub fn panel_alpha(&self) -> f32 {
        if self.mica { self.look.layer_alpha() } else { 1.0 }
    }

    /// The palette's base: always the accent (Windows' when Accent = Windows, else the chosen one). Nothing comes
    /// from the wallpaper.
    pub fn tint(&self) -> theme::Tint {
        theme::Tint::of(self.accent_colour())
    }

    /// The one colour the whole window follows: the chosen accent, the wallpaper's colour behind the window
    /// (Follow desktop), or the Windows accent.
    pub fn accent_colour(&self) -> crate::gfx::C {
        if self.look.desktop
            && let Some(c) = self.desk
        {
            return c;
        }
        accent_of(&self.look)
    }

    /// Follow desktop: read the wallpaper (off the UI thread; the cached grid while the file is the same).
    pub fn refresh_desk(&self) {
        if self.look.desktop {
            self.spawn(|| Msg::Wall(theme::cached_wall().or_else(theme::wallpaper_grid)));
        }
    }

    /// Follow desktop: the wallpaper's colour behind the window now, made vivid enough to be an accent.
    pub fn retint(&mut self) {
        if !self.look.desktop {
            return;
        }
        let Some(w) = &self.wall else { return };
        let (win, mon) = unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            let hm = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(hm, &mut mi);
            let m = mi.rcMonitor;
            ((r.left as f32, r.top as f32, r.right as f32, r.bottom as f32), (m.left as f32, m.top as f32, m.right as f32, m.bottom as f32))
        };
        let t = w.tint_at(win, mon);
        let c = theme::oklch(0.60, t.chroma.clamp(0.10, 0.16), t.hue);
        if self.desk != Some(c) {
            self.desk = Some(c);
            self.apply_theme();
        }
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
        let custom = if self.look.desktop { self.desk.map(theme::to_hex) } else { self.look.accent };
        self.t = theme::make(dark, custom, self.tint(), self.look.strength);
        self.gfx.corner = if self.look.square { 0.0 } else { 1.0 };
        set_dark_frame(self.hwnd, dark);
        self.invalidate();
    }

    /// The sidebar shows as the icon rail: narrow window, split view, or collapsed by its menu button.
    pub fn rail(&self) -> bool {
        self.tab().split() || self.look.side_collapsed || self.auto_rail()
    }

    /// The window is too narrow for the full sidebar plus a usable list (about 600 px): icons only, by itself.
    pub fn auto_rail(&self) -> bool {
        self.w - self.look.side_w < 600.0
    }

    /// Very narrow window: no sidebar in the layout (the menu button in the top bar opens it as an overlay).
    pub fn side_hidden(&self) -> bool {
        self.w < 480.0
    }

    /// The full sidebar is open over the content (narrow window, opened with the menu button).
    pub fn side_overlay(&self) -> bool {
        self.side_open && (self.side_hidden() || (self.auto_rail() && !self.look.side_collapsed && !self.tab().split()))
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
    /// Attributes and change time of a folder (for its own icon), read once and kept.
    pub fn dir_attrs(&mut self, p: &Path) -> (u32, u64) {
        if let Some(v) = self.dir_attr.get(p) {
            return *v;
        }
        use std::os::windows::fs::MetadataExt;
        let v = std::fs::metadata(p).map(|m| (m.file_attributes(), m.last_write_time())).unwrap_or((0, 0));
        self.dir_attr.insert(p.to_path_buf(), v);
        v
    }

    pub fn build_side(&mut self) {
        self.dir_attr.clear();
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
        if loc == Loc::Cleanup && let Some(i) = self.tabs.iter().position(|t| t.pane().loc == Loc::Cleanup) {
            self.cur = i;
            self.invalidate();
            return;
        }
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
        if self.tabs.get(i).is_some_and(|t| t.pane().loc == Loc::Cleanup) {
            if self.cleanup.moving() {
                return;
            }
            self.close_cleanup();
        }
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

    /// Keep the caches bounded over a long session: thumbnails, icons and their GPU bitmaps are dropped (and made
    /// again on demand) once they pass a limit. Called when the folder changes.
    pub fn trim_caches(&mut self) {
        if self.thumbs.len() > 300 {
            self.thumbs.clear();
            self.thumb_asked.clear();
        }
        if self.icons.done.len() > 1500 {
            self.icons.done.clear();
            self.icons.asked.clear();
        }
        if self.bitmaps.len() > 400 {
            self.bitmaps.clear();
        }
        if self.sizes.len() > 200 {
            self.sizes.clear();
        }
    }

    pub fn go(&mut self, loc: Loc) {
        if self.pane().loc == Loc::Cleanup || loc == Loc::Cleanup {
            if loc == Loc::ThisPc && let Some(i) = self.tabs.iter().position(|t| t.pane().loc == Loc::ThisPc) {
                self.cur = i;
                self.invalidate();
            } else if self.pane().loc != loc {
                self.new_tab(loc);
            }
            return;
        }
        self.tip = None;
        self.side_reveal = true;
        self.trim_caches();
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
        if let Some(i) = self.cleanup_back_tab() {
            self.cur = i;
            self.invalidate();
            return;
        }
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
            Loc::Cleanup => {}
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
            pane.watch.clear();
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
                // the Desktop shows what the real desktop shows: the user's and the Public Desktop's items together
                let public = if known("Desktop").map(|d| fmcore::util::path_eq(&d, &dir)).unwrap_or(false) { public_desktop() } else { None };
                if pane.watch.is_empty() {
                    for w in std::iter::once(dir.clone()).chain(public.clone()) {
                        let bg = self.bg.clone();
                        let hw = self.hwnd.0 as isize;
                        let d2 = dir.clone();
                        if let Ok(x) = fmcore::watch::Watcher::start(&w, false, move |_c| post_bg(&bg, hw, Msg::Changed { dir: d2.clone() })) {
                            pane.watch.push(x);
                        }
                    }
                }
                let t0 = self.started;
                self.spawn(move || {
                    let s = Instant::now();
                    let res = fmcore::list::list_dir(&dir, hidden).map(|v| {
                        let mut items: Vec<Item> = v.iter().map(|e| Item::from_entry(&dir, e)).collect();
                        if let Some(pd) = &public
                            && let Ok(v2) = fmcore::list::list_dir(pd, hidden)
                        {
                            for e in &v2 {
                                if !items.iter().any(|i| i.name.eq_ignore_ascii_case(&e.name)) {
                                    items.push(Item::from_entry(pd, e));
                                }
                            }
                        }
                        // protected system files (hidden + system, e.g. desktop.ini) stay hidden, as in Explorer
                        items.retain(|i| i.attrs & 0x6 != 0x6);
                        items
                    });
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
                            empty: false,
                        }
                    })
                    .collect::<Vec<Item>>();
                let mut items = items;
                if self.look.show_pinned {
                    let pins: Vec<(String, PathBuf)> = self.side.iter().filter_map(|(n, _, _, l)| if let Loc::Folder(d) = l { Some((n.clone(), d.clone())) } else { None }).collect();
                    for (name, dir) in pins {
                        let (attrs, modified) = self.dir_attrs(&dir);
                        {
                            items.push(Item {
                                name: name.clone(),
                                path: dir.clone(),
                                is_dir: true,
                                size: None,
                                modified,
                                attrs,
                                kind: crate::ficons::Kind::Folder,
                                folder: dir.parent().map(|x| x.display().to_string()).unwrap_or_default(),
                                hl: vec![],
                                recycled: None,
                                empty: false,
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
            Loc::Cleanup => pane.loading = false,
        }
        self.invalidate();
    }

    pub fn refresh_settings(&mut self) {
        self.refresh_cleanup_hidden();
        self.spawn(|| {
            let running = fmcore::evipc::available();
            let total = if running { fmcore::evipc::query("", 0, app_req(), 1, false, Duration::from_secs(2)).map(|r| r.total).unwrap_or(0) } else { 0 };
            Msg::EvStatus { running, total }
        });
    }

    /// Folder sizes of `dir` from the Everything index: one subtree query, summed per child folder.
    /// Folder sizes of `dir`, off the UI thread, streamed as they come: one Everything query per child folder
    /// (only the sizes of its files, through the IPC: about as fast as `wfm info`); a folder outside the index gets
    /// a short walk (the whole pass stops walking after 4 s). Truly empty folders (no entries) are marked
    /// EMPTY_DIR. Unknown stays unknown (the cell shows "…" while the pass runs, then nothing).
    pub fn ask_sizes(&mut self, dir: &Path) {
        if !self.ui.sizes {
            return;
        }
        let dir = dir.to_path_buf();
        self.sizes_busy.insert(dir.clone());
        let bg = self.bg.clone();
        let hw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let mut sizes: HashMap<String, u64> = HashMap::new();
            let child_dirs: Vec<String> = std::fs::read_dir(&dir)
                .map(|rd| rd.flatten().filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                .unwrap_or_default();
            let mut last_post = Instant::now();
            let mut outside = vec![];
            for name in &child_dirs {
                let p = dir.join(name);
                let key = name.to_lowercase();
                if std::fs::read_dir(&p).map(|mut r| r.next().is_none()).unwrap_or(false) {
                    sizes.insert(key, EMPTY_DIR);
                    continue;
                }
                let q = format!("{} file:", fmcore::evipc::quote(&format!("{}\\", p.display())));
                match fmcore::evipc::size_sum(&q, 4_000_000, Duration::from_secs(4)) {
                    Ok(Some((n, sum))) if n > 0 => {
                        sizes.insert(key, sum);
                    }
                    // nothing indexed under a non-empty folder: outside the index (walked below)
                    _ => outside.push(name.clone()),
                }
                if last_post.elapsed() > Duration::from_millis(250) {
                    last_post = Instant::now();
                    post_bg(&bg, hw, Msg::Sizes { dir: dir.clone(), sizes: sizes.clone(), done: false });
                }
            }
            for name in outside {
                let left = Duration::from_secs(4).saturating_sub(t0.elapsed());
                if left.is_zero() {
                    break;
                }
                let f = fmcore::info::folder_size(&dir.join(&name), false, left.min(Duration::from_millis(1500)));
                if f.complete {
                    sizes.insert(name.to_lowercase(), f.bytes);
                }
            }
            trace(&format!("sizes {}: {} of {} folders in {:.0} ms", dir.display(), sizes.len(), child_dirs.len(), t0.elapsed().as_secs_f64() * 1000.0));
            post_bg(&bg, hw, Msg::Sizes { dir, sizes, done: true });
        });
    }

    /// The real folders behind a place: the Desktop is the user's Desktop plus the Public Desktop.
    pub fn sources(&self, dir: Option<PathBuf>) -> Vec<PathBuf> {
        let Some(d) = dir else { return vec![] };
        let mut v = vec![d.clone()];
        if known("Desktop").map(|k| fmcore::util::path_eq(&k, &d)).unwrap_or(false)
            && let Some(p) = public_desktop()
        {
            v.push(p);
        }
        v
    }

    /// The user's own places for ranking (lower case, trailing backslash): known folders, pins, recent places.
    fn own_places(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![];
        let mut add = |p: &Path| {
            let mut s = p.to_string_lossy().to_lowercase();
            if !s.ends_with('\\') {
                s.push('\\');
            }
            v.push(s);
        };
        for (_, _, _, l) in &self.side {
            if let Loc::Folder(p) = l {
                add(p);
            }
        }
        if let Some(p) = public_desktop() {
            add(&p);
        }
        for r in &self.recent {
            if let Some(p) = Path::new(&r.target).parent() {
                add(p);
            }
        }
        v
    }

    pub fn start_search(&mut self) {
        let here = self.sources(self.pane().dir());
        let own = self.own_places();
        let sp = self.sp.clone();
        let show_hidden = self.ui.hidden;
        let tab_id = self.tab().id;
        let p = self.tab().active;
        let Some(s) = self.pane_mut().search.as_mut() else { return };
        s.seq += 1;
        let seq = s.seq;
        let words = s.field.text.trim().to_string();
        if words.is_empty() {
            s.items.clear();
            s.all.clear();
            s.total = 0;
            self.invalidate();
            return;
        }
        // more letters typed onto a search whose answer was complete (every match came back): filter those rows
        // right here, no query, no wait
        if !s.busy && s.ai.is_none() && plain_words(&words) && !s.done_words.is_empty() {
            let (lw, dw) = (words.to_lowercase(), s.done_words.to_lowercase());
            if lw.len() > dw.len() && lw.starts_with(&dw) && (s.total as usize) <= s.all.len() {
                let ws: Vec<&str> = lw.split_whitespace().collect();
                s.all.retain(|i| {
                    let n = i.name.to_lowercase();
                    ws.iter().all(|w| n.contains(w))
                });
                for i in s.all.iter_mut() {
                    i.hl = match_ranges(&i.name, &words);
                }
                s.total = s.all.len() as u32;
                s.done_words = words;
                s.open.clear();
                s.rearrange();
                let pn = self.pane_mut();
                pn.scroll = 0.0;
                pn.sel.clear();
                pn.focus = None;
                pn.anchor = None;
                self.invalidate();
                return;
            }
        }
        s.busy = true;
        s.own = own.clone();
        s.open.clear();
        let (junk, open) = (s.junk, s.open.clone());
        let mut q = s.query_with(&here, &sp);
        // a whole-PC search follows Settings > Search (a ticked-out folder's own row is dropped after the query)
        let whole = s.pc || here.is_empty();
        // hidden items stay out of search unless "Show hidden files" is on (also everything inside hidden folders,
        // filtered below: Everything cannot ask about parents)
        if !show_hidden {
            // anything named with a dot (.vscode) or inside one (a path holding "\."); NOT attrib:H, which makes
            // Everything read every match from disk when attributes are not indexed (0.7 s, or a time-out for "a").
            // Hidden items go below, from the attributes the results already carry.
            q += " !\\.";
        }
        // not for 1-2 letters: they match millions and are rarely the final word
        let prefix = (plain_words(&words) && words.chars().count() >= 3).then(|| format!("startwith:{}{}", fmcore::evipc::quote(&words), &q[words.len().min(q.len())..]));
        s.pending_words = words.clone();
        self.spawn_search(tab_id, p, move || {
            // the 200 newest matches (attributes are not indexed: every extra row is read from disk), then a fast
            // second look (~20 ms) for names that START with the words: they rank best and may be older
            let req = fmcore::evipc::REQ_NAME | fmcore::evipc::REQ_PATH | fmcore::evipc::REQ_SIZE | fmcore::evipc::REQ_MODIFIED | fmcore::evipc::REQ_ATTRS;
            let mut r = fmcore::evipc::query(&q, 200, req, fmcore::evipc::SORT_DATE_MODIFIED_DESC, false, Duration::from_secs(3));
            if let (Ok(r1), Some(pq)) = (r.as_mut(), prefix.as_ref())
                && r1.total as usize > r1.items.len()
                && let Ok(r2) = fmcore::evipc::query(pq, 100, req, fmcore::evipc::SORT_DATE_MODIFIED_DESC, false, Duration::from_secs(2))
            {
                let seen: HashSet<String> = r1.items.iter().map(|i| i.full().to_lowercase()).collect();
                r1.items.extend(r2.items.into_iter().filter(|i| !seen.contains(&i.full().to_lowercase())));
            }
            let item = |name: String, path: PathBuf, folder: String, is_dir: bool, size: Option<u64>, modified: u64, attrs: u32| Item {
                kind: crate::ficons::kind_of(&name, is_dir),
                hl: match_ranges(&name, &words),
                size: if is_dir { size.filter(|s| *s > 0) } else { size },
                modified,
                attrs,
                folder,
                is_dir,
                name,
                path,
                recycled: None,
                empty: false,
            };
            let res: Result<(u32, Vec<Item>), String> = match r {
                Ok(r) => {
                    let n = r.items.len();
                    // folders read once per run of the app (each keystroke searches again)
                    let mut cache = HIDDEN_DIRS.lock().unwrap();
                    let items: Vec<Item> = r
                        .items
                        .into_iter()
                        .filter(|i| !whole || sp.keeps(Path::new(&i.full())))
                        .filter(|i| show_hidden || i.attrs & 0x2 == 0 && !i.name.starts_with('.') && !in_hidden_folder(Path::new(&i.path), &mut cache))
                        .map(|i| item(i.name.clone(), PathBuf::from(i.full()), i.path.clone(), i.is_dir, i.size, i.modified, i.attrs))
                        .collect();
                    let gone = (n - items.len()) as u32;
                    Ok((r.total.saturating_sub(gone), items))
                }
                // Everything not running: the folder walk from fmcore (slower, this place only: every real folder)
                Err(_) => {
                    let mut all = vec![];
                    let mut err = None;
                    for dir in if here.is_empty() { vec![None] } else { here.iter().cloned().map(Some).collect() } {
                        let q = fmcore::search::Query { words: words.split_whitespace().map(|s| s.to_string()).collect(), in_dir: dir, top: 300, engine: fmcore::search::Engine::Walk, walk_limit_ms: 3000, ..Default::default() };
                        match fmcore::search::search(&q) {
                            Ok(f) => all.extend(f.hits.into_iter().map(|h| {
                                let folder = Path::new(&h.path).parent().map(|p| p.display().to_string()).unwrap_or_default();
                                item(h.name.clone(), PathBuf::from(&h.path), folder, h.is_dir, h.size, h.modified, h.attrs)
                            })),
                            Err(e) => err = Some(e),
                        }
                    }
                    match err {
                        Some(e) if all.is_empty() => Err(e),
                        _ => Ok((all.len() as u32, all)),
                    }
                }
            };
            let rows = match &res {
                Ok((_, all)) => crate::rank::arrange(all, &words, &own, junk, &open, fmcore::util::now_ft()),
                Err(_) => vec![],
            };
            Msg::Found { tab: tab_id, pane: p, seq, res, rows, ai: None }
        });
    }

    /// An error toast: red mark, the message made short (full paths become file names).
    pub fn toast_err(&mut self, e: &str) {
        let text = humanize(e);
        self.toast(&text, "", false);
        if let Some(t) = self.toast.as_mut() {
            t.err = true;
        }
    }

    pub fn toast(&mut self, text: &str, bold: &str, undo: bool) {
        self.toast = Some(Toast { text: text.into(), bold: bold.into(), undo, err: false, at: Instant::now(), hover: false });
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
            Msg::Cleanup(id, update) => self.cleanup_updated(id, update),
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
                                let v = s.get(&it.name.to_lowercase()).copied();
                                it.empty = v == Some(EMPTY_DIR);
                                it.size = v.map(|x| if x == EMPTY_DIR { 0 } else { x });
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
            Msg::Sizes { dir, sizes, done } => {
                if done {
                    self.sizes_busy.remove(&dir);
                }
                for tab in &mut self.tabs {
                    for pn in &mut tab.panes {
                        if pn.dir().map(|d| fmcore::util::path_eq(&d, &dir)).unwrap_or(false) {
                            for it in pn.items.iter_mut().filter(|i| i.is_dir) {
                                let v = sizes.get(&it.name.to_lowercase()).copied();
                                it.empty = v == Some(EMPTY_DIR);
                                it.size = v.map(|x| if x == EMPTY_DIR { 0 } else { x });
                            }
                            if pn.sort.0 == SortCol::Size {
                                pn.sort_items();
                            }
                        }
                    }
                }
                self.sizes.insert(dir, sizes);
            }
            Msg::Found { tab, pane, seq, res, rows, ai } => {
                let Some((t, p)) = self.find_pane(tab, pane) else { return };
                let Some(s) = self.tabs[t].panes[p].search.as_mut() else { return };
                if s.seq != seq {
                    return;
                }
                s.busy = false;
                // the words this answer is for (a later keystroke may filter it locally)
                s.done_words = if ai.is_none() && res.is_ok() { std::mem::take(&mut s.pending_words) } else { String::new() };
                s.ai = ai;
                match res {
                    Ok((total, items)) => {
                        s.total = total;
                        s.all = items;
                        s.items = rows;
                        s.note.clear();
                    }
                    Err(e) => {
                        s.items.clear();
                        s.all.clear();
                        s.total = 0;
                        s.note = e;
                    }
                }
                let pn = &mut self.tabs[t].panes[p];
                pn.scroll = 0.0;
                // new results select nothing (the keys stay in the search box; Enter opens the first result)
                pn.sel.clear();
                pn.focus = None;
                pn.anchor = None;
            }
            Msg::Drives(d) => {
                self.drives = d;
                self.cleanup_drives_ready();
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
            Msg::Icon { key, px, img } => {
                self.icons.done.insert((key, px), img);
            }
            Msg::Wall(w) => {
                if w.is_some() {
                    self.wall = w;
                    self.retint();
                }
            }
            Msg::PathSug { seq, items } => {
                if seq == self.sug_seq && self.path_edit.is_some() {
                    self.sug = items;
                    self.sug_sel = None;
                }
            }
            Msg::Changed { dir } => {
                let _ = dir;
                self.timer(T_RELIST, 180);
            }
            Msg::ShellMenu { seq, items, apps } => {
                if let Some(m) = self.menu.as_mut()
                    && m.seq == seq
                {
                    m.loading = false;
                    m.shell = items;
                    m.apps = apps.into_iter().map(|(h, i)| (h, i.map(std::sync::Arc::new))).collect();
                    if let Some(p) = m.paths.first() {
                        self.open_with = Some((p.clone(), m.apps.iter().map(|a| a.0.clone()).collect()));
                    }
                    // an open "Open with" submenu fills in place (only it may grow)
                    let fresh = crate::input::open_with_items(m);
                    if let Some(s) = self.sub.as_mut()
                        && s.child
                    {
                        match s.kind {
                            DropKind::OpenWith => s.items = fresh,
                            DropKind::More => s.items = crate::input::more_items(m),
                            _ => {}
                        }
                    }
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
            Msg::ToastErr(e) => self.toast_err(&e),
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
            Msg::AiFlags { ai } => {
                self.ai_on = ai;
            }
            Msg::OpenWith { path, apps } => {
                self.open_with = Some((path, apps));
                crate::input::show_open_with(self);
            }
        }
    }

    pub fn refresh_ai_flags(&mut self) {
        self.ai_on = crate::ai::enabled();
        if !self.ai_on {
            self.stop_cleanup_review();
        }
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

    /// The real (shell) icon of a file as a bitmap at `size` DIP, or None while it loads (or for folders).
    pub fn icon_bitmap(&mut self, path: &Path, name: &str, is_dir: bool, attrs: u32, modified: u64, size: f32) -> Option<ID2D1Bitmap> {
        let key = crate::sicons::key_of(path, name, is_dir, attrs, modified)?;
        let px = crate::sicons::bucket(size * self.gfx.scale());
        if self.bmp_target != self.gfx.target_id() {
            self.bitmaps.clear();
            self.bmp_target = self.gfx.target_id();
        }
        let bkey = PathBuf::from(format!("icon|{px}|{key}"));
        if let Some(b) = self.bitmaps.get(&bkey) {
            return b.clone();
        }
        let k = (key, px);
        match self.icons.done.get(&k) {
            Some(Some((w, h, img))) => {
                let b = self.gfx.make_bitmap(*w, *h, img);
                self.bitmaps.insert(bkey, b.clone());
                b
            }
            Some(None) => None,
            None => {
                if self.icons.asked.insert(k.clone()) {
                    let (m, cv) = &*self.icons.q;
                    m.lock().unwrap().push((k.0, k.1, path.to_path_buf()));
                    cv.notify_one();
                }
                None
            }
        }
    }

    /// Draw an item's icon: the shell's when ready, else the vector icon of its type.
    pub fn item_icon(&mut self, path: &Path, name: &str, is_dir: bool, kind: crate::ficons::Kind, r: crate::gfx::Rect, t: &crate::theme::Theme) {
        self.item_icon_a(path, name, is_dir, 0, 0, kind, r, t)
    }

    /// Same, knowing the item's attributes and time (folders with their own icon: read-only / system + desktop.ini).
    #[allow(clippy::too_many_arguments)]
    pub fn item_icon_a(&mut self, path: &Path, name: &str, is_dir: bool, attrs: u32, modified: u64, kind: crate::ficons::Kind, r: crate::gfx::Rect, t: &crate::theme::Theme) {
        match self.icon_bitmap(path, name, is_dir, attrs, modified, r.w) {
            Some(b) => self.gfx.bitmap(&b, r, 1.0),
            None => crate::ficons::draw(&self.gfx, t, kind, r),
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
                        "search": p.search.as_ref().map(|s| json!({"text": s.field.text, "this_pc": s.pc, "total": s.total, "busy": s.busy, "note": s.note})),
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
            "look": self.look.to_json(), "marquee": self.marquee.is_some(), "hover": format!("{:?}", self.hover),
            "surfaces": self.surfaces(),
            "size": [self.w, self.h], "rail": self.rail(),
            "drives": self.drives.len(), "sidebar": self.side.iter().map(|s| s.0.clone()).collect::<Vec<_>>(),
            "ai_buttons": self.ai_on,
            "cleanup": self.cleanup_status(),
            "first_paint_ms": self.first_paint_ms, "first_list_ms": self.first_list_ms,
            "last_error": self.last_error,
        })
    }
}

/// "already exists: C:\\Users\\x\\a.md" -> "Already exists: a.md" (paths shortened to their names).
pub fn humanize(e: &str) -> String {
    // a path starts at "X:\" or "\\" and runs to a quote, ": ", ", ", " to <another path>", or the end, so
    // names with spaces ("a b.txt", "Program Files") stay whole
    let cs: Vec<char> = e.chars().collect();
    let starts = |i: usize| -> bool {
        (i + 2 < cs.len() && cs[i].is_ascii_alphabetic() && cs[i + 1] == ':' && cs[i + 2] == '\\' && (i == 0 || !cs[i - 1].is_alphanumeric()))
            || (i + 1 < cs.len() && cs[i] == '\\' && cs[i + 1] == '\\' && (i == 0 || cs[i - 1] == ' ' || cs[i - 1] == '"'))
    };
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if !starts(i) {
            out.push(cs[i]);
            i += 1;
            continue;
        }
        let mut j = i + 2;
        while j < cs.len() {
            let rest: String = cs[j..cs.len().min(j + 4)].iter().collect();
            if cs[j] == '"' || cs[j] == '\'' || rest.starts_with(": ") || rest.starts_with(", ") || (rest.starts_with(" to ") && starts(j + 4)) {
                break;
            }
            j += 1;
        }
        let path: String = cs[i..j].iter().collect();
        let path = path.trim_end_matches(['.', ')', ' ']);
        let name = Path::new(path.trim_end_matches('\\')).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        // drop quotes that already wrapped the path
        if out.ends_with('"') || out.ends_with('\'') {
            out.pop();
        }
        out += &format!("\u{201C}{name}\u{201D}");
        i = i + path.chars().count();
        if i < cs.len() && (cs[i] == '"' || cs[i] == '\'') {
            i += 1;
        }
    }
    let mut c = out.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => out,
    }
}

#[cfg(test)]
mod humanize_tests {
    #[test]
    fn paths_with_spaces() {
        assert_eq!(super::humanize(r"already exists: C:\Users\x\a b.txt"), "Already exists: \u{201C}a b.txt\u{201D}");
        assert_eq!(super::humanize(r#"cannot open "C:\Program Files\App\x y.dll": access denied"#), "Cannot open \u{201C}x y.dll\u{201D}: access denied");
        assert_eq!(super::humanize(r"cannot move C:\a b\c d.txt to D:\e f\g: in use"), "Cannot move \u{201C}c d.txt\u{201D} to \u{201C}g\u{201D}: in use");
        assert_eq!(super::humanize("no path here"), "No path here");
    }
}

/// The chosen accent colour, else the Windows accent, else Windows' default blue (Follow desktop starts here
/// until the wallpaper is read).
pub fn accent_of(look: &crate::look::Look) -> crate::gfx::C {
    look.accent.map(crate::gfx::C::hex).or_else(theme::windows_accent_base).unwrap_or(crate::gfx::C::hex(0x0067C0))
}

/// The palette's base for a look (start-up: Follow desktop uses its cached wallpaper colour when there is one).
pub fn tint_for(look: &crate::look::Look) -> theme::Tint {
    theme::Tint::of(accent_of(look))
}

/// Give the pages this window is not using now back to Windows (EmptyWorkingSet): they come back on demand.
/// Private memory is unchanged; only what sits in RAM shrinks.
pub fn trim_memory() {
    unsafe {
        let _ = windows::Win32::System::Threading::SetProcessWorkingSetSize(windows::Win32::System::Threading::GetCurrentProcess(), usize::MAX, usize::MAX);
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
            WM_CONTEXTMENU if lp.0 == -1 || (lp.0 & 0xFFFF_FFFF) == 0xFFFF_FFFF => {
                app.keyboard_menu();
                app.invalidate();
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
                // minimised: nothing is drawn, so the pages can go back to Windows until it is restored
                if wp.0 == SIZE_MINIMIZED as usize {
                    trim_memory();
                }
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
            WM_EXITSIZEMOVE => {
                // Follow desktop: the part of the wallpaper behind the window changed
                app.retint();
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
                // the Windows theme or accent may have changed: the whole palette follows it live (and a new
                // wallpaper when the accent follows the desktop)
                app.apply_theme();
                app.cleanup_motion_changed();
                app.refresh_desk();
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xFFFF) == 0 {
                    app.close_popups();
                } else if app.assoc_stale {
                    // back from Windows' app picker: a new default app brings its own file icons
                    app.assoc_stale = false;
                    app.refresh_type_icons();
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
                if !app.cleanup.moving() {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // remember where we were (start page "Last session"), the tabs and the window's place
                if let Some(p) = app.pane().loc.path() {
                    app.look.last = p.display().to_string();
                }
                if !app.offscreen && std::env::var("WFILES_TEST").is_err() {
                    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                    if GetWindowPlacement(hwnd, &mut wp).is_ok() {
                        let r = wp.rcNormalPosition;
                        app.look.win = Some([r.left, r.top, r.right, r.bottom, (wp.showCmd == SW_SHOWMAXIMIZED.0 as u32 || IsZoomed(hwnd).as_bool()) as i32]);
                    }
                }
                app.look.tabs = app
                    .tabs
                    .iter()
                    .filter_map(|t| match &t.pane().loc {
                        Loc::Folder(p) => Some(p.display().to_string()),
                        Loc::ThisPc => Some("::thispc".to_string()),
                        Loc::RecycleBin => Some("::bin".to_string()),
                        Loc::Cleanup => Some("::cleanup".to_string()),
                        _ => None,
                    })
                    .collect();
                app.look.tab_cur = app.cur.min(app.look.tabs.len().saturating_sub(1));
                app.save_look();
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
    // the GPU device loads its driver for ~50 ms: start it now, in parallel with the window
    let mut dev_early = if std::env::var("WFILES_CPU").is_ok() { None } else { Some(crate::gfx::device_early()) };
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
            // search starts in the current folder unless the user picked This PC
            scope_pc: ui["scope_pc"].as_bool().unwrap_or(false),
        };
        let look = crate::look::Look::load();
        let dark = o.theme.unwrap_or(match look.theme {
            1 => false,
            2 => true,
            _ => theme::system_dark(),
        });
        let t = theme::make(dark, look.accent, tint_for(&look), look.strength);
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
        // one renderer: DirectComposition on the GPU (its device is being made since the start of run); the CPU
        // HwndRenderTarget only when that fails or WFILES_CPU is set
        let dev_rx = if std::env::var("WFILES_CPU").is_ok() { None } else { dev_early.take() };
        let Ok(mut gfx) = Gfx::new(hwnd, o.software, mica, dev_rx) else { return 1 };
        gfx.corner = if look.square { 0.0 } else { 1.0 };
        gfx.set_see_through(look.see_through());
        let queue = jobs::new_queue();
        let app_look_win = look.win;
        let app = Box::new(App {
            hwnd,
            gfx,
            t,
            ui: uis,
            sp: SearchPrefs::load(&cfg),
            place_open: HashSet::new(),
            place_kids: HashMap::new(),
            place_rows: vec![],
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
            cleanup: crate::cleanup_page::Page::default(),
            recent: vec![],
            side: vec![],
            menu_tab: None,
            dir_attr: HashMap::new(),
            tip: None,
            sug: vec![],
            sug_sel: None,
            sug_seq: 0,
            name_w: vec![],
            menu_side: None,
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
            icons: crate::sicons::Icons::default(),
            assoc_stale: false,
            thumb_asked: HashSet::new(),
            sizes: HashMap::new(),
            sizes_busy: HashSet::new(),
            wall: None,
            desk: None,
            rail_scroll: 0.0,
            rail_max: 0.0,
            side_sf: crate::motion::Follow::default(),
            side_area: Rect::default(),
            side_reveal: true,
            pop_scroll: [(None, 0.0, 0.0); 3],
            menu_gen: 0,
            recycle: vec![],
            set: SetState { ev_running: None, ev_total: 0, msg: None, msg_ok: false },
            ai_on: false,
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
            relay_fields: Default::default(),
            relay_focus: 0,
            confirm: None,
            pending_rename: None,
            toast_job: None,
            shot_hover: false,
            synthetic: false,
            done_seen: HashSet::new(),
            asked_jobs: HashSet::new(),
            dragging_out: false,
            ta_text: String::new(),
            ta_at: None,
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
            Some(p) if special_loc(p).is_some() => (special_loc(p).unwrap(), None),
            Some(p) if Path::new(p).is_dir() => (Loc::Folder(fmcore::util::abs(p)), None),
            Some(p) if Path::new(p).is_file() => (Loc::Folder(fmcore::util::abs(p).parent().map(|x| x.to_path_buf()).unwrap_or_default()), Some(fmcore::util::abs(p))),
            _ => (start, None),
        };
        trace(&format!("{:.1} ms: app struct", started.elapsed().as_secs_f64() * 1000.0));
        app.build_side();
        app.new_tab(loc);
        // start page "Last session": the other tabs of the last session come back too
        if o.path.is_none() && app.look.start == "last" && (app.look.tabs.len() > 1 || app.look.tabs.first().is_some_and(|s| s == "::cleanup")) {
            let saved = app.look.tabs.clone();
            let cur = app.look.tab_cur.min(saved.len() - 1);
            let locs: Vec<Loc> = saved
                .iter()
                .map(|s| match s.as_str() {
                    "::thispc" => Some(Loc::ThisPc),
                    "::bin" => Some(Loc::RecycleBin),
                    "::cleanup" => Some(Loc::Cleanup),
                    p => Some(PathBuf::from(p)).filter(|p| p.is_dir()).map(Loc::Folder),
                })
                .collect::<Option<Vec<_>>>()
                .unwrap_or_default();
            if locs.len() == saved.len() {
                app.tabs.clear();
                for l in locs {
                    app.new_tab(l);
                }
                app.cur = cur;
            }
        }
        trace(&format!("{:.1} ms: side + tab", started.elapsed().as_secs_f64() * 1000.0));
        app.tabs[0].panes[0].want = select;
        // drives (with a time limit for dead network shares) and recent files, off the UI thread
        app.spawn(|| Msg::Drives(fmcore::shell::drives_within(Duration::from_millis(1500))));
        app.spawn(|| Msg::Recent(fmcore::pins::recent(4).unwrap_or_default()));
        // Follow desktop: the cached wallpaper grid at once (one small file), else read it off the UI thread
        if app.look.desktop {
            app.wall = theme::cached_wall();
            if app.wall.is_some() {
                app.retint();
            } else {
                app.refresh_desk();
            }
        }
        // the AI switch reads files: keep it off the first frame
        app.spawn(|| Msg::AiFlags { ai: crate::ai::enabled() });
        // thumbnails worker (newest request first)
        {
            {
                let bg = app.bg.clone();
                let hw = hwnd.0 as isize;
                crate::sicons::start(app.icons.q.clone(), move |key, px, img| post_bg(&bg, hw, Msg::Icon { key, px, img }));
            }
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
        // the window's last place and size (only when it is still on a monitor), unless a size was asked for
        let mut show = if o.offscreen { SW_SHOWNOACTIVATE } else { SW_SHOW };
        if !o.offscreen && o.size.is_none() && std::env::var("WFILES_TEST").is_err()
            && let Some([l, t, r, b, max]) = app_look_win
            && r - l >= 360
            && b - t >= 200
            && !MonitorFromRect(&RECT { left: l, top: t, right: r, bottom: b }, MONITOR_DEFAULTTONULL).is_invalid()
        {
            let wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, showCmd: SW_HIDE.0 as u32, rcNormalPosition: RECT { left: l, top: t, right: r, bottom: b }, ..Default::default() };
            let _ = SetWindowPlacement(hwnd, &wp);
            if max == 1 {
                show = SW_SHOWMAXIMIZED;
            }
        }
        let _ = ShowWindow(hwnd, show);
        trace(&format!("{:.1} ms: shown", started.elapsed().as_secs_f64() * 1000.0));
        let _ = UpdateWindow(hwnd);
        trace(&format!("{:.1} ms: shown + first paint", started.elapsed().as_secs_f64() * 1000.0));
        crate::dnd::register(hwnd, sink_of);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // test key messages (`wfm gui keys`) already carry their WM_CHAR
            if msg.lParam.0 != TEST_KEY {
                let _ = TranslateMessage(&msg);
            }
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

/// Is this folder, or any folder above it, hidden (hidden attribute or a dot name like .vscode)? Answers are cached
/// per folder, so a search reads each folder's attributes once.
pub fn in_hidden_folder(dir: &Path, cache: &mut HashMap<PathBuf, bool>) -> bool {
    let mut chain = vec![];
    let mut found = None;
    for a in dir.ancestors() {
        // the drive root itself carries hidden + system on some disks: it never counts
        if a.parent().is_none() || a.as_os_str().is_empty() {
            break;
        }
        if let Some(h) = cache.get(a) {
            found = Some(*h);
            break;
        }
        chain.push(a.to_path_buf());
        let dot = a.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false);
        // GetFileAttributesW reads the attributes without opening the folder (std::fs::metadata opens a handle)
        let attrs = unsafe { windows::Win32::Storage::FileSystem::GetFileAttributesW(windows::core::PCWSTR(fmcore::util::wide(a.as_os_str()).as_ptr())) };
        if dot || (attrs != u32::MAX && attrs & 0x2 != 0) {
            found = Some(true);
            break;
        }
    }
    let h = found.unwrap_or(false);
    // a hidden folder hides everything below it; a visible chain is visible all the way up
    for c in chain {
        cache.insert(c, h);
    }
    h
}

/// in_hidden_folder answers, kept for the whole run (a folder's hidden flag rarely changes)
pub static HIDDEN_DIRS: std::sync::LazyLock<Mutex<HashMap<PathBuf, bool>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

impl App {
    /// File-type icons are made again (after a default app changed): the shell's own image list is reset and every
    /// per-type icon in the cache is dropped, so the rows ask for them anew.
    pub fn refresh_type_icons(&mut self) {
        unsafe {
            use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
            // FileIconInit(TRUE) (shell32 ordinal 660): the system image list reloads changed type icons
            if let Ok(h) = GetModuleHandleW(windows::core::w!("shell32.dll"))
                && let Some(f) = GetProcAddress(h, windows::core::PCSTR(660usize as *const u8))
            {
                let f: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
                f(1);
            }
        }
        self.icons.done.retain(|k, _| !k.0.starts_with("e|"));
        self.icons.asked.retain(|k| !k.0.starts_with("e|"));
        self.invalidate();
    }
}

/// One search job: (tab, pane), the work, where its message goes.
type SearchJob = ((u64, usize), Box<dyn FnOnce() -> Msg + Send>, Arc<Mutex<Vec<Msg>>>, isize);
/// The waiting searches (at most one per pane) for the single search thread.
static SEARCH_SLOT: std::sync::LazyLock<(Mutex<Vec<SearchJob>>, Condvar)> = std::sync::LazyLock::new(|| (Mutex::new(vec![]), Condvar::new()));

/// Words with no Everything syntax (no operators, wildcards, quotes, paths or field: terms): safe to filter locally
/// and to use in a startwith: query.
pub fn plain_words(w: &str) -> bool {
    !w.is_empty() && !w.chars().any(|c| matches!(c, ':' | '|' | '!' | '"' | '*' | '?' | '<' | '>' | '/' | '\\'))
}
