//! The Task Manager window: state, custom frame, input, actions and the `wtm gui` control channel.
//! Painting lives in paint.rs (Processes page), side.rs (right panes), pages.rs (Startup apps, Settings) and
//! over.rs (performance flyout, menus, confirm dialog, toasts).
use super::data::{self, Ctl, HistAnomaly, Shared, WM_DATA};
use super::gfx::{Gfx, Rect};
use super::model::{self, Group, Hist, SortKey};
use super::prefs::{self, Prefs};
use super::motion;
use super::theme::{self, Theme};
use crate::explain::Answer;
use crate::procs::{self, Proc};
use crate::sys::wide;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub const TITLE_H: f32 = 48.0;
pub const TILES_H: f32 = 76.0;
pub const CHIPS_H: f32 = 30.0;
pub const CMD_H: f32 = 48.0;
pub const HEAD_H: f32 = 32.0;
pub const ROW_H: f32 = 36.0;
pub const PANE_W: f32 = 376.0;
/// Sidebar: collapsed (icons) and expanded (icon + label) widths.
pub const NAV_W: f32 = 48.0;
pub const NAV_W_OPEN: f32 = 200.0;
pub const FREE_W: f32 = 432.0;
const WM_PANEL: u32 = WM_DATA + 1;
const WM_IPC: u32 = WM_DATA + 2;
const WM_KEYTEST: u32 = WM_DATA + 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const TIMER_ANIM: usize = 1;
const TIMER_TICK: usize = 2;
const TIMER_TRIM: usize = 3;
/// tooltip delay after the pointer rests
const TIMER_TIP: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Hit {
    None,
    Block,
    Tab(u8),
    Settings,
    Min,
    Max,
    Close,
    Search,
    SearchClear,
    Tile(u8),
    Chip(u8),
    ChipEnd(u8),
    ChipMore,
    EndTask,
    Restart,
    Details,
    More,
    Free,
    Sort,
    Header(u8),
    Row(usize),
    Chevron(usize),
    SrStartupToggle(usize),
    SrServiceBtn(usize),
    PaneClose,
    PaneEnd,
    PaneRestart,
    PaneMore,
    PaneSec(u8),
    PaneDisable(usize),
    PaneScroll,
    FreeEnd(usize),
    FreeEndDisable(usize),
    FreeKeep,
    FreeAll,
    MenuItem(u8),
    DlgEnd,
    DlgWait,
    ToastLink,
    ToastClose,
    FlyFree,
    StToggle(usize),
    StSort,
    StRow(usize),
    StPaneClose,
    StSecHead(u8),
    StMore(u8),
    StFoldAll,
    SetDefault,
    RestoreDefault,
    Seg(u8, u8),
    Toggle(u8),
    KeyField,
    KeySave,
    KeyTest,
    KeyRemove,
    KeyLink,
    Scroll(u8),
    /// Settings > Accent: 0 system, 1..=8 presets, 9 custom
    Accent(u8),
    Hue,
    HexField,
    /// Settings > Columns: 0 Type, 1 PID, 2 Disk, 3 Network, 4 GPU
    ColPill(u8),
    DefSort,
    /// the sidebar's expand / collapse button
    NavToggle,
    /// "Restart as administrator" (shown when a process cannot be read without admin)
    RunAdmin,
    /// the process list as one keyboard stop (Up / Down / Enter / Menu key work in it)
    List,
    /// Performance page: a resource in the list (index), a top process (index)
    PerfItem(u8),
    PerfTop(u8),
}

/// Can the keyboard stop on this control (Tab / Shift+Tab, Enter or Space presses it)?
pub fn focusable(h: Hit) -> bool {
    matches!(
        h,
        Hit::Search
            | Hit::List
            | Hit::Tab(_)
            | Hit::NavToggle
            | Hit::Tile(_)
            | Hit::EndTask
            | Hit::Restart
            | Hit::Details
            | Hit::More
            | Hit::Free
            | Hit::Sort
            | Hit::Header(_)
            | Hit::PaneClose
            | Hit::PaneEnd
            | Hit::PaneRestart
            | Hit::PaneMore
            | Hit::PaneSec(_)
            | Hit::RunAdmin
            | Hit::FreeAll
            | Hit::StToggle(_)
            | Hit::StSort
            | Hit::StPaneClose
            | Hit::StSecHead(_)
            | Hit::StMore(_)
            | Hit::StFoldAll
            | Hit::SetDefault
            | Hit::RestoreDefault
            | Hit::Seg(..)
            | Hit::Toggle(_)
            | Hit::KeyField
            | Hit::KeySave
            | Hit::KeyTest
            | Hit::KeyRemove
            | Hit::Accent(_)
            | Hit::HexField
            | Hit::ColPill(_)
            | Hit::DefSort
            | Hit::PerfItem(_)
            | Hit::PerfTop(_)
    )
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RowK {
    Group(usize),
    Child(usize, usize),
    PinSep,
    /// section header of the grouped list: 0 Apps, 1 Background processes, 2 Windows processes; count
    Kind(u8, usize),
    /// "Show all (N)" under a section (N = its size); 0 = "Show fewer"
    More(u8, usize),
    Sec(u8, usize),
    Startup(usize),
    Service(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub k: RowK,
    pub y: f32,
    pub h: f32,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Sel {
    Group(String),
    Child(u32),
    Startup(usize),
    Service(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaneKind {
    Details,
    Free,
}

#[derive(Clone, Debug, Default)]
pub struct StartupHit {
    pub name: String,
    pub source: String,
    pub command: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PaneInfo {
    pub answer: Option<Answer>,
    /// the built-in answer (kept next to an AI answer; its maker is a local fact)
    pub local: Option<Answer>,
    pub signed: Option<bool>,
    pub parent: Option<(String, String, i64)>,
    pub startup: Vec<StartupHit>,
    pub services: Vec<String>,
    pub tasks: Vec<String>,
    pub flags: Vec<String>,
    pub cmdline: String,
}

#[derive(Clone)]
pub struct Pane {
    pub kind: PaneKind,
    pub key: String,
    pub pid: u32,
    pub open: [bool; 3],
    pub info: PaneInfo,
    pub loading: bool,
    pub ai_loading: bool,
    pub keep_open: bool,
    pub at: Instant,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MenuKind {
    Row,
    Sort,
    StartupSort,
    Chips,
    DefSort,
}

#[derive(Clone)]
pub struct Menu {
    pub kind: MenuKind,
    pub x: f32,
    pub y: f32,
    pub at: Instant,
    /// the rectangle of the button that opened it (a dropdown opens under it); None = at the pointer
    pub anchor: Option<Rect>,
    /// the button that opened it: a click on that button while it is open only closes it
    pub by: Hit,
}

#[derive(Clone)]
pub struct EndJob {
    pub pids: Vec<u32>,
    pub label: String,
    pub mem: u64,
    pub icon: String,
    pub count: usize,
    pub relaunch: Option<(String, String)>,
    pub disable: Option<String>,
}

#[derive(Clone)]
pub struct Dialog {
    pub title: String,
    pub body: String,
    pub job: EndJob,
    pub at: Instant,
    /// a process Windows needs: red warning, the button says "End anyway", Enter cancels
    pub danger: bool,
    /// Restart: the command line and folder started again after the end
    pub restart: Option<(String, String)>,
    /// the cancel button: "Wait" for a hung app, else "Cancel"
    pub cancel: &'static str,
}

#[derive(Clone)]
pub enum ToastAct {
    Relaunch(String, String),
}

#[derive(Clone)]
pub struct Toast {
    pub text: String,
    pub code: String,
    pub link: Option<(String, ToastAct)>,
    pub ok: bool,
    pub at: Instant,
    pub secs: u64,
}


pub struct App {
    pub hwnd: HWND,
    pub gfx: Gfx,
    pub t: Theme,
    pub prefs: Prefs,
    pub theme_override: Option<bool>,
    pub mica: bool,
    shared: Arc<Mutex<Shared>>,
    pub ctl: Arc<Ctl>,
    data_gen: u64,
    startup_gen: u64,
    pub procs: Arc<Vec<Proc>>,
    pub perf: crate::perf::Snapshot,
    pub services: HashMap<u32, Vec<String>>,
    pub services_all: Arc<Vec<crate::services::Service>>,
    pub startup: Arc<Vec<crate::startup::Entry>>,
    pub cmdlines: HashMap<u32, String>,
    pub hung: Vec<u32>,
    pub hist_anoms: Vec<HistAnomaly>,
    pub gpu_names: Vec<String>,
    icon_px: HashMap<(String, u32), Vec<u8>>,
    icons: HashMap<(String, u32), Option<ID2D1Bitmap>>,
    icon_target: usize,
    icon_asked: HashSet<(String, u32)>,
    pub groups: Vec<Group>,
    order_prev: HashMap<String, usize>,
    last_order: Option<Instant>,
    pub hist: HashMap<String, Hist>,
    pub child_hist: HashMap<u32, Hist>,
    /// cpu %, memory used bytes, disk bytes/s, network bytes/s, gpu %
    pub tot: [VecDeque<f32>; 5],
    pub rows: Vec<Row>,
    pub sel: Option<Sel>,
    pub expanded: HashSet<String>,
    pub sort: SortKey,
    pub startup_sort: u8,
    /// a just-switched startup row: (index, its place before, when) -> it moves after STARTUP_HOLD
    pub startup_hold: Option<(usize, bool, Instant)>,
    /// the details pane of a startup entry (index, what is known about it)
    pub st_pane: Option<(usize, super::pages::StInfo)>,
    /// where the open menu was drawn last (a click on its padding stays in it)
    pub menu_r: Option<Rect>,
    pub query: String,
    pub search_focus: bool,
    /// caret in the search text (chars; usize::MAX = at the end) and "all selected" (Ctrl+A)
    pub caret: usize,
    pub q_all: bool,
    /// list, startup page, settings page, pane
    pub scroll: [f32; 4],
    drag_scroll: Option<(u8, f32, f32)>,
    pub hits: Vec<(Rect, Hit)>,
    /// tooltips of the last frame: area and text (cut text, icon-only buttons, disabled reasons)
    pub tips: Vec<(Rect, String)>,
    /// pointer position and when it came to rest there (tooltips show after 500 ms)
    pub mouse: (f32, f32),
    pub rest_at: Instant,
    pub hover: Hit,
    pub pressed: Hit,
    pub nc_hover: Hit,
    pub nc_down: Hit,
    pub page: u8,
    pub pane: Option<Pane>,
    pane_result: Arc<Mutex<Option<(u32, PaneInfo, bool)>>>,
    pub flyout: Option<(u8, Instant)>,
    pub menu: Option<Menu>,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    /// every running animation (fades, springs, transitions, closing overlays)
    pub mo: std::cell::RefCell<Mo>,
    anim_until: std::cell::Cell<Instant>,
    /// --offscreen: test window far off the screen (never activated, never moved onto a monitor)
    pub offscreen: bool,
    /// window is active (caption glyphs dim when not); `active_force` overrides it for tests
    pub active: bool,
    pub active_force: Option<bool>,
    /// Settings > Accent > Custom: hex field being typed and the hue strip's place
    pub hex_focus: bool,
    pub hex_buf: String,
    pub hue_rect: Rect,
    /// Settings > Accent > Custom is open
    pub custom_open: bool,
    pub w: f32,
    pub h: f32,
    ipc_in: Arc<Mutex<Vec<(String, std::sync::mpsc::Sender<String>)>>>,
    pub first_paint_ms: Option<f64>,
    pub full_data_ms: Option<f64>,
    started: Instant,
    pub test: bool,
    pub last_action: String,
    pub ai_on: bool,
    pub lang_fa: Option<bool>,
    /// API key being typed (never shown, never logged)
    pub key_buf: String,
    pub key_focus: bool,
    pub key_saved: bool,
    pub key_testing: bool,
    /// result of the last "Test key"
    pub key_result: Option<Result<(), String>>,
    key_out: Arc<Mutex<Option<Result<(), String>>>>,
    pub shift: bool,
    pub hi_surrogate: u16,
    /// the focused dialog button: 0 the action, 1 cancel
    pub dlg_focus: u8,
    /// the sort is reversed (second click on a column header)
    pub sort_rev: bool,
    /// Performance page: graph history per resource, the selected resource, the keys of the top processes shown
    pub perf_hist: HashMap<String, VecDeque<f32>>,
    pub perf_sel: String,
    pub perf_top_keys: Vec<String>,
    /// narrow window: the sidebar is opened over the page (not saved)
    pub nav_peek: bool,
    /// the control with the keyboard after Tab (a focus ring shows it); None after a mouse click
    pub focus: Option<Hit>,
    /// drop the next WM_CHAR (Space pressed a focused control)
    pub eat_char: bool,
    /// sort value per row key, averaged over the last samples, and the sort it belongs to
    pub smooth: HashMap<String, f64>,
    pub smooth_sort: SortKey,
    /// the window runs as administrator (it can read every process)
    pub elevated: bool,
    pub is_default: bool,
    /// set while painting a screenshot: animations show their end state
    pub shooting: bool,
    pub list_rect: Rect,
    pub pane_rect: Rect,
    pub content_h: [f32; 4],
    pub view_h: [f32; 4],
    /// bring the selected row into view on the next paint (after the layout is known)
    pub want_scroll_sel: bool,
    /// the pointer is over the list (its order is frozen meanwhile)
    pub pointer_in_list: bool,
    /// last click or keyboard move (the order stays frozen for 3 s after it)
    pub last_input: Instant,
    /// row ids as last painted: a click maps its row index through these, so a re-sort never hits another row
    pub painted: Vec<u64>,
    /// current sidebar width (springs between NAV_W and NAV_W_OPEN)
    pub nav_w: f32,
    /// collapsed sidebar: the hovered item's name and its centre y
    pub nav_tip: Option<(&'static str, f32)>,
    /// list row height (Settings > Density)
    pub row_h: f32,
}

/// Everything that is moving right now. Lives in a RefCell so painting code (`&self`) can advance it.
#[derive(Default)]
pub struct Mo {
    pub fades: HashMap<u64, motion::Tween>,
    pub springs: HashMap<u64, motion::Spring>,
    pub vals: HashMap<(u64, u8), motion::Tween>,
    pub row_y: HashMap<u64, motion::Tween>,
    pub row_in: HashMap<u64, motion::Tween>,
    pub prev_rows: HashMap<u64, f32>,
    pub prev_query: String,
    pub scroll: [Option<motion::Spring>; 4],
    /// the selection a highlight was last drawn for, and when it appeared
    pub sel_prev: Option<(u64, Instant)>,
    /// overlays as last drawn, and the ones that are closing (drawn fading out)
    pub last_flyout: Option<(u8, Instant)>,
    pub last_menu: Option<Menu>,
    pub last_toast: Option<Toast>,
    pub last_dialog: Option<Dialog>,
    pub last_pane: Option<(Pane, f32)>,
    pub ghost_flyout: Option<(u8, Instant, Instant)>,
    pub ghost_menu: Option<(Menu, Instant)>,
    pub ghost_toast: Option<(Toast, Instant)>,
    pub ghost_dialog: Option<(Dialog, Instant)>,
    pub ghost_pane: Option<(Pane, f32, Instant)>,
}

/// Key of a per-clickable animation: `tag` tells hover, press, knob, ... apart.
pub fn hid(tag: u8, h: Hit) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut s = std::collections::hash_map::DefaultHasher::new();
    (tag, h).hash(&mut s);
    s.finish()
}

pub fn ease(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

impl App {
    // ------------------------------------------------------------------ small helpers
    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Progress 0..1 of an animation that started at `at` and lasts `ms` (eased).
    pub fn anim(&self, at: Instant, ms: u64) -> f32 {
        if self.shooting {
            return 1.0;
        }
        let p = at.elapsed().as_secs_f32() * 1000.0 / ms as f32;
        ease(p)
    }

    /// Spring progress (0 -> 1, may overshoot) of something that started at `at`; asks for frames until it settles.
    pub fn spring_in(&self, at: Instant, c: motion::Cfg) -> f32 {
        if self.shooting {
            return 1.0;
        }
        let e = at.elapsed().as_secs_f32();
        if e < motion::settle(c) {
            self.animate(30);
            motion::spring(e, c)
        } else {
            1.0
        }
    }

    /// Eased progress 0..1 over `ms` from `at` along `curve`; asks for frames until done.
    pub fn tween_in(&self, at: Instant, ms: f32, curve: fn(f32) -> f32) -> f32 {
        if self.shooting {
            return 1.0;
        }
        let p = at.elapsed().as_secs_f32() * 1000.0 / ms;
        if p < 1.0 {
            self.animate(30);
        }
        curve(p.clamp(0.0, 1.0))
    }

    /// Ask for animation frames for the next `ms`. The timer stops by itself when nothing moves.
    pub fn animate(&self, ms: u64) {
        let until = Instant::now() + Duration::from_millis(ms + 20);
        if until > self.anim_until.get() {
            self.anim_until.set(until);
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_ANIM, 15, None);
        }
    }

    /// Animated x for a sliding pill (tabs, segmented controls): a spring toward the selected item.
    pub fn slide(&mut self, key: u8, target: f32) -> f32 {
        self.glide(0x5000 + key as u64, target, motion::GLIDE).0
    }

    /// A named spring toward `target`: (position, velocity). First use starts at the target.
    pub fn glide(&self, id: u64, target: f32, c: motion::Cfg) -> (f32, f32) {
        let mut mo = self.mo.borrow_mut();
        let sp = mo.springs.entry(id).or_insert_with(|| motion::Spring::new(target, 0.2));
        if self.shooting {
            sp.snap(target);
            return (target, 0.0);
        }
        let x = sp.step(target, c, Instant::now());
        if sp.moving() {
            self.animate(30);
        }
        (x, sp.v)
    }

    /// Fade level 0..1 for something that is `on` (hover fills): in over FAST, out over NORMAL, decelerating.
    pub fn fade(&self, id: u64, on: bool) -> f32 {
        if self.shooting {
            return on as u8 as f32;
        }
        let now = Instant::now();
        let tgt = on as u8 as f32;
        let mut mo = self.mo.borrow_mut();
        let Some(tw) = mo.fades.get_mut(&id) else {
            if on {
                let mut tw = motion::Tween::fixed(0.0);
                tw.retarget(1.0, motion::FAST, now, motion::decel);
                mo.fades.insert(id, tw);
                self.animate(motion::FAST as u64);
            }
            return 0.0;
        };
        if tw.to != tgt {
            tw.retarget(tgt, if on { motion::FAST } else { motion::NORMAL }, now, motion::decel);
        }
        let v = tw.value(now, motion::decel);
        if tw.done(now) {
            if !on {
                mo.fades.remove(&id);
            }
        } else {
            self.animate(30);
        }
        v
    }

    /// Hover level of a clickable (fades in and out).
    pub fn hov(&self, h: Hit) -> f32 {
        self.fade(hid(0, h), self.hover == h)
    }

    /// Press depth of a clickable: a spring to 1 while held, back to 0 (with a small bounce) when let go.
    pub fn press_depth(&self, h: Hit) -> f32 {
        let down = self.pressed == h && self.hover == h;
        let id = hid(2, h);
        if self.shooting {
            return down as u8 as f32;
        }
        let mut mo = self.mo.borrow_mut();
        if !down && !mo.springs.contains_key(&id) {
            return 0.0;
        }
        let sp = mo.springs.entry(id).or_insert_with(|| motion::Spring::new(0.0, 0.004));
        let x = sp.step(down as u8 as f32, motion::PRESS, Instant::now());
        if sp.moving() || down {
            self.animate(30);
        } else {
            mo.springs.remove(&id);
        }
        x
    }

    /// Start the press-scale (0.97 at full press) around `r` for clickable `h`; pair with `press_end`.
    pub fn press_begin(&self, r: Rect, h: Hit) -> bool {
        let d = self.press_depth(h);
        if d.abs() < 0.002 {
            return false;
        }
        self.gfx.push_scale(r.cx(), r.cy(), 1.0 - motion::PRESS_SCALE * d);
        true
    }

    pub fn press_end(&self, on: bool) {
        if on {
            self.gfx.pop_xf();
        }
    }

    /// A value that glides to `target` when it changes (numbers and bars after a new sample).
    pub fn tv(&self, id: u64, slot: u8, target: f32) -> f32 {
        if self.shooting {
            return target;
        }
        let now = Instant::now();
        let mut mo = self.mo.borrow_mut();
        let tw = mo.vals.entry((id, slot)).or_insert_with(|| motion::Tween::fixed(target));
        tw.used = now;
        if (tw.to - target).abs() > (target.abs() * 1e-4).max(1e-6) {
            tw.retarget(target, motion::VALUE, now, motion::decel);
        }
        if !tw.done(now) {
            self.animate(30);
        }
        tw.value(now, motion::decel)
    }

    /// Where row `id` is drawn now (it slides when the list re-orders) and its opacity (new rows fade in).
    pub fn row_motion(&self, id: u64, y: f32) -> (f32, f32) {
        if self.shooting {
            return (y, 1.0);
        }
        let now = Instant::now();
        let mo = self.mo.borrow();
        // a row that changes place moves there quickly and calmly
        let (dy, dim) = match mo.row_y.get(&id) {
            Some(tw) if !tw.done(now) => {
                self.animate(30);
                (tw.value(now, motion::decel), 1.0)
            }
            _ => (y, 1.0),
        };
        let a = match mo.row_in.get(&id) {
            Some(tw) if !tw.done(now) => {
                self.animate(30);
                tw.value(now, motion::decel)
            }
            _ => 1.0,
        };
        (dy, a * dim)
    }

    /// Remember where each row is, and start slides for rows that moved (not while the search text changes).
    fn note_rows(&mut self) {
        let now = Instant::now();
        let ids: Vec<(u64, f32)> = self.rows.iter().map(|r| (self.row_id(r.k), r.y)).collect();
        let mut mo = self.mo.borrow_mut();
        let same = mo.prev_query == self.query && !mo.prev_rows.is_empty();
        let mut moved = false;
        if same {
            for (id, y) in &ids {
                match mo.prev_rows.get(id).copied() {
                    Some(py) if (py - y).abs() > 0.5 => {
                        let cur = mo.row_y.get(id).filter(|t| !t.done(now)).map(|t| t.value(now, motion::standard)).unwrap_or(py);
                        mo.row_y.insert(*id, motion::Tween { from: cur, to: *y, at: now, ms: motion::MOVE, used: now });
                        moved = true;
                    }
                    Some(_) => {}
                    None => {
                        mo.row_in.insert(*id, motion::Tween { from: 0.0, to: 1.0, at: now, ms: motion::NORMAL, used: now });
                        moved = true;
                    }
                }
            }
        } else {
            mo.row_y.clear();
            mo.row_in.clear();
        }
        mo.row_y.retain(|_, t| !t.done(now));
        mo.row_in.retain(|_, t| !t.done(now));
        mo.prev_rows = ids.into_iter().collect();
        mo.prev_query = self.query.clone();
        drop(mo);
        if moved {
            self.animate(motion::MOVE as u64);
        }
    }

    /// Stable identity of a list row (survives re-ordering).
    pub fn row_id(&self, k: RowK) -> u64 {
        use std::hash::{Hash, Hasher};
        match k {
            RowK::Group(gi) => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                self.groups.get(gi).map(|g| g.key.as_str()).unwrap_or("").hash(&mut h);
                h.finish() & !(0xFF << 56)
            }
            RowK::Child(gi, mi) => (1 << 56) | self.groups.get(gi).and_then(|g| g.members.get(mi)).map(|m| m.pid as u64).unwrap_or(0),
            RowK::Startup(i) => (2 << 56) | i as u64,
            RowK::Service(i) => (3 << 56) | i as u64,
            RowK::Sec(s, _) => (4 << 56) | s as u64,
            RowK::PinSep => 5 << 56,
            RowK::Kind(k, _) => (6 << 56) | k as u64,
            RowK::More(k, _) => (7 << 56) | k as u64,
        }
    }

    // ------------------------------------------------------------------ scrolling (momentum with a soft stop)
    /// Scroll area `k` by `delta` with momentum: the target moves, a critically damped spring follows it.
    pub fn scroll_by(&mut self, k: usize, delta: f32) {
        let max = (self.content_h[k] - self.view_h[k]).max(0.0);
        let cur = self.scroll[k];
        let mut mo = self.mo.borrow_mut();
        let sp = mo.scroll[k].get_or_insert_with(|| motion::Spring::new(cur, 0.25));
        sp.target = (sp.target + delta).clamp(0.0, max);
        drop(mo);
        if self.shooting {
            self.set_scroll(k, (cur + delta).clamp(0.0, max));
        } else {
            self.animate(60);
        }
    }

    /// Glide area `k` to `v`.
    pub fn scroll_smooth(&mut self, k: usize, v: f32) {
        let d = v - self.mo.borrow().scroll[k].map(|s| s.target).unwrap_or(self.scroll[k]);
        self.scroll_by(k, d);
    }

    /// Jump area `k` to `v` (stops any glide).
    pub fn set_scroll(&mut self, k: usize, v: f32) {
        self.mo.borrow_mut().scroll[k] = None;
        self.scroll[k] = v;
        self.clamp_scroll();
    }

    /// Advance the scroll springs (once per frame, before painting).
    fn tick_scroll(&mut self) {
        let now = Instant::now();
        for k in 0..4 {
            let max = (self.content_h[k] - self.view_h[k]).max(0.0);
            let mut mo = self.mo.borrow_mut();
            let Some(sp) = mo.scroll[k].as_mut() else { continue };
            let tgt = sp.target.clamp(0.0, max);
            let x = sp.step(tgt, motion::SCROLL, now);
            let moving = sp.moving();
            if !moving {
                mo.scroll[k] = None;
            }
            drop(mo);
            self.scroll[k] = x.clamp(0.0, max);
            if moving {
                self.animate(30);
            }
        }
    }

    pub fn theme_now(&self) -> Theme {
        let dark = match (self.theme_override, self.prefs.theme) {
            (Some(d), _) => d,
            (None, 1) => false,
            (None, 2) => true,
            _ => theme::system_dark(),
        };
        // Transparency needs a backdrop behind the window (Mica, Mica Alt, Acrylic); Solid is always opaque
        let transp = if self.backdrop_on() { self.prefs.transp } else { 0 };
        theme::make_toned(dark, self.accent_rgb(), transp, Some(self.palette_base(dark)))
    }

    /// The desktop (DWM colorization) colour, for "Follow desktop".
    pub fn desktop_rgb(&self) -> Option<u32> {
        let mut c = 0u32;
        let mut opaque = windows::core::BOOL(0);
        unsafe { DwmGetColorizationColor(&mut c, &mut opaque) }.ok().map(|_| c & 0xFFFFFF)
    }

    /// The accent in use: the chosen colour, the desktop colour ("Follow desktop"), or None = the Windows accent.
    pub fn accent_rgb(&self) -> Option<u32> {
        if self.prefs.desktop {
            let mut c = 0u32;
            let mut opaque = windows::core::BOOL(0);
            if unsafe { DwmGetColorizationColor(&mut c, &mut opaque) }.is_ok() {
                return Some(c & 0xFFFFFF);
            }
        }
        self.prefs.accent
    }

    /// Base hue and chroma of the window palette: the accent's hue, as strong as Settings > Colour strength says,
    /// so the whole window (title bar, sidebar, content, cards, hover, selection, borders) is one tonal family.
    pub fn palette_base(&self, dark: bool) -> (f32, f32) {
        // absolute chroma of the window colour: Subtle / Medium / Strong
        let chroma = match self.prefs.strength {
            0 => 0.018,
            2 => 0.10,
            _ => if dark { 0.05 } else { 0.06 },
        };
        let a = theme::make(dark, self.accent_rgb(), 0).acc;
        (theme::hue_of(((a.0 * 255.0) as u32) << 16 | ((a.1 * 255.0) as u32) << 8 | (a.2 * 255.0) as u32), chroma)
    }

    pub fn apply_theme(&mut self) {
        self.t = self.theme_now();
        set_dark_frame(self.hwnd, self.t.dark);
        self.invalidate();
    }

    /// Is a DWM backdrop (Mica, Mica Alt, Acrylic) showing behind the window?
    pub fn backdrop_on(&self) -> bool {
        self.mica && self.prefs.backdrop != 3
    }

    /// Apply the look settings that live outside the colour theme: backdrop, corners, text size, row height.
    pub fn apply_look(&mut self) {
        if self.mica {
            set_backdrop(self.hwnd, [2, 4, 3, 1][self.prefs.backdrop.min(3) as usize]);
        }
        set_corners(self.hwnd, self.prefs.corners == 0);
        self.gfx.font_scale.set([0.93, 1.0, 1.1][self.prefs.font.min(2) as usize]);
        self.gfx.corner.set(if self.prefs.corners == 0 { 1.0 } else { 0.0 });
        self.row_h = if self.prefs.density == 0 { 30.0 } else { ROW_H };
        self.build_rows();
        self.apply_theme();
    }

    /// Width of the right pane (Settings > Side pane width); Free up memory is a little wider.
    pub fn pane_w(&self, kind: PaneKind) -> f32 {
        let w = [336.0, PANE_W, 432.0][self.prefs.pane.min(2) as usize];
        if kind == PaneKind::Free { w + FREE_W - PANE_W } else { w }
    }

    pub fn lang_is_fa(&self) -> bool {
        self.lang_fa.unwrap_or_else(|| crate::ai::lang() == "fa")
    }

    pub fn now_ft() -> i64 {
        crate::insight::now_filetime()
    }

    pub fn proc(&self, pid: u32) -> Option<&Proc> {
        self.procs.iter().find(|p| p.pid == pid)
    }

    pub fn group(&self, key: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.key == key)
    }

    pub fn group_of(&self, pid: u32) -> Option<&Group> {
        self.groups.iter().find(|g| g.members.iter().any(|m| m.pid == pid))
    }

    pub fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    // ------------------------------------------------------------------ data
    fn take_data(&mut self) {
        let mut sh = self.shared.lock().unwrap();
        if sh.generation == self.data_gen && sh.icons.is_empty() {
            return;
        }
        let new_sample = sh.generation != self.data_gen;
        self.data_gen = sh.generation;
        self.procs = sh.procs.clone();
        self.perf = sh.perf.clone();
        self.services = sh.services.clone();
        self.services_all = sh.services_all.clone();
        self.cmdlines = sh.cmdlines.clone();
        self.hung = sh.hung.clone();
        self.hist_anoms = sh.hist.clone();
        if !sh.gpu_names.is_empty() {
            self.gpu_names = sh.gpu_names.clone();
        }
        if sh.startup_gen != self.startup_gen {
            self.startup_gen = sh.startup_gen;
            self.startup = sh.startup.clone();
        }
        for (path, px, data) in sh.icons.drain(..) {
            let k = (path.to_lowercase(), px);
            self.icons.remove(&k);
            self.icon_px.insert(k, data);
        }
        drop(sh);
        if new_sample {
            self.rebuild_groups(true);
            if self.full_data_ms.is_none() && !self.cmdlines.is_empty() {
                self.full_data_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
            }
        } else {
            self.build_rows();
        }
        if let Some(p) = &self.pane
            && p.kind == PaneKind::Details
            && self.group(&p.key).is_none()
            && self.group_of(p.pid).is_none()
        {
            self.pane = None;
        }
    }

    fn rebuild_groups(&mut self, sample: bool) {
        let empty = HashMap::new();
        let gpu = self.perf.gpu.as_ref().map(|g| &g.per_pid).unwrap_or(&empty);
        let mut v = model::groups(&self.procs, &self.services, &self.cmdlines, gpu, &self.hung, self.prefs.merge, self.perf.mem_total.max(1));
        // the sort value of each row, averaged over the last samples (about 4 s), for a calm order
        if sample || self.smooth_sort != self.sort {
            let reset = self.smooth_sort != self.sort;
            self.smooth_sort = self.sort;
            let mut next = HashMap::with_capacity(v.len());
            for g in &v {
                let x = model::sort_val(g, self.sort);
                let e = if reset { x } else { self.smooth.get(&g.key).map(|o| o * 0.75 + x * 0.25).unwrap_or(x) };
                next.insert(g.key.clone(), e);
            }
            self.smooth = next;
        }
        // a calm list: the order is frozen while the pointer is over the list, a menu or flyout is open, or for
        // 3 s after a click or key; otherwise rows move at most every 5 s, inside their own section, and only for a
        // clear gap in value (model::calm_order). Name order is exact and never waits.
        let fresh = self.sort == SortKey::Name || self.order_prev.is_empty() || (!self.order_frozen() && self.last_order.map(|t| t.elapsed() >= Duration::from_secs(5)).unwrap_or(true));
        if fresh {
            let (sort, rev) = (self.sort, self.sort_rev);
            if self.prefs.group && self.query.is_empty() {
                let mut out = Vec::with_capacity(v.len());
                for kind in [procs::Kind::App, procs::Kind::Background, procs::Kind::Windows] {
                    let (sec, rest): (Vec<Group>, Vec<Group>) = v.into_iter().partition(|g| g.kind == kind);
                    v = rest;
                    out.extend(model::calm_order(sec, &self.order_prev, sort, rev, &self.smooth));
                }
                out.extend(v);
                v = out;
            } else {
                v = model::calm_order(v, &self.order_prev, sort, rev, &self.smooth);
            }
            self.last_order = Some(Instant::now());
        } else {
            // frozen: the same order as before; new rows wait at the end of their section
            let prev = &self.order_prev;
            v.sort_by_key(|g| prev.get(&g.key).copied().unwrap_or(usize::MAX));
        }
        self.order_prev = v.iter().enumerate().map(|(i, g)| (g.key.clone(), i)).collect();
        if sample {
            let now = Instant::now();
            let gn = self.data_gen;
            for g in &v {
                self.hist.entry(g.key.clone()).or_default().add(g.cpu, g.mem, g.io, g.gpu, now, gn);
                if g.members.len() > 1 {
                    for m in &g.members {
                        self.child_hist.entry(m.pid).or_default().add(m.cpu, m.mem, m.io, m.gpu, now, gn);
                    }
                }
            }
            self.hist.retain(|_, h| h.seen == gn);
            self.child_hist.retain(|_, h| h.seen == gn);
            let s = &self.perf;
            let disk: f64 = s.disks.iter().map(|d| d.read_rate + d.write_rate).sum();
            let net: f64 = s.nets.iter().map(|n| n.recv_rate + n.send_rate).sum();
            let vals = [s.cpu, s.mem_total.saturating_sub(s.mem_avail) as f32, disk as f32, net as f32, s.gpu.as_ref().map(|g| g.util).unwrap_or(0.0)];
            self.push_perf_hist();
            for (q, v) in self.tot.iter_mut().zip(vals) {
                q.push_back(v);
                while q.len() > model::HIST_N {
                    q.pop_front();
                }
            }
        }
        self.groups = v;
        self.build_rows();
    }

    /// Is the list order frozen right now?
    pub fn order_frozen(&self) -> bool {
        self.pointer_in_list || self.menu.is_some() || self.flyout.is_some() || self.dialog.is_some() || self.last_input.elapsed() < Duration::from_secs(3)
    }

    /// The row a click on painted row `i` means now (rows may have been rebuilt since the last paint).
    pub fn row_now(&self, i: usize) -> Option<usize> {
        let id = *self.painted.get(i)?;
        if self.rows.get(i).map(|r| self.row_id(r.k)) == Some(id) {
            return Some(i);
        }
        self.rows.iter().position(|r| self.row_id(r.k) == id)
    }

    /// Show all / show fewer rows of section `k`.
    pub fn toggle_section(&mut self, k: u8) {
        self.set_pref(|p| p.sec_open[k as usize % 3] = !p.sec_open[k as usize % 3]);
        self.build_rows();
    }

    /// The display list: pinned "Not responding" rows, then the rest; children of open groups; or search results.
    pub fn build_rows(&mut self) {
        let mut rows = vec![];
        let mut y = 4.0f32;
        fn push(rows: &mut Vec<Row>, y: &mut f32, k: RowK, h: f32) {
            rows.push(Row { k, y: *y, h });
            *y += h;
        }
        if self.searching() {
            let toks = model::tokens(&self.query);
            let by_pid: HashMap<u32, &Proc> = self.procs.iter().map(|p| (p.pid, p)).collect();
            let mut hits = vec![];
            for (gi, g) in self.groups.iter().enumerate() {
                let label_hit = model::startup_matches(&crate::startup::Entry { name: g.label.clone(), command: String::new(), source: "", enabled: true }, &toks);
                let kids: Vec<usize> = g.members.iter().enumerate().filter(|(_, m)| by_pid.get(&m.pid).map(|p| model::member_matches(p, self.cmdlines.get(&m.pid).map(|s| s.as_str()), &toks)).unwrap_or(false)).map(|(i, _)| i).collect();
                if label_hit || !kids.is_empty() {
                    // name matches first, command-line-only matches last
                    let rank = g.members.iter().filter_map(|m| by_pid.get(&m.pid).and_then(|p| crate::procs::match_rank(p, &toks, self.cmdlines.get(&m.pid).map(|s| s.as_str())))).min().unwrap_or(if label_hit { 0 } else { 999 });
                    hits.push((gi, kids, if label_hit { 0 } else { rank }));
                }
            }
            hits.sort_by_key(|h| h.2);
            let hits: Vec<(usize, Vec<usize>)> = hits.into_iter().map(|h| (h.0, h.1)).collect();
            let st: Vec<usize> = self.startup.iter().enumerate().filter(|(_, e)| model::startup_matches(e, &toks)).map(|(i, _)| i).collect();
            let sv: Vec<usize> = self.services_all.iter().enumerate().filter(|(_, s)| model::service_matches(s, &toks)).map(|(i, _)| i).collect();
            // an empty section is not shown (no "Processes 0" over an empty list)
            if !hits.is_empty() {
                push(&mut rows, &mut y, RowK::Sec(0, hits.len()), 32.0);
            }
            for (gi, kids) in &hits {
                push(&mut rows, &mut y, RowK::Group(*gi), self.row_h);
                let g = &self.groups[*gi];
                if g.members.len() > 1 {
                    let all: Vec<usize> = if kids.is_empty() || self.expanded.contains(&g.key) { (0..g.members.len()).collect() } else { kids.clone() };
                    for mi in all {
                        push(&mut rows, &mut y, RowK::Child(*gi, mi), self.row_h);
                    }
                }
            }
            if !st.is_empty() {
                y += 4.0;
                push(&mut rows, &mut y, RowK::Sec(1, st.len()), 32.0);
                for i in st {
                    push(&mut rows, &mut y, RowK::Startup(i), 40.0);
                }
            }
            if !sv.is_empty() {
                y += 4.0;
                push(&mut rows, &mut y, RowK::Sec(2, sv.len()), 32.0);
                for i in sv.into_iter().take(60) {
                    push(&mut rows, &mut y, RowK::Service(i), 40.0);
                }
            }
        } else if self.prefs.group {
            // sections like Windows Task Manager; each keeps the current sort and shows its top 5 until opened
            let sel_key = self.sel_group().map(|g| g.key.clone());
            for (k, kind) in [procs::Kind::App, procs::Kind::Background, procs::Kind::Windows].iter().enumerate() {
                let mut list: Vec<usize> = (0..self.groups.len()).filter(|&i| self.groups[i].kind == *kind).collect();
                if list.is_empty() {
                    continue;
                }
                list.sort_by_key(|&i| !self.groups[i].hung);
                if k > 0 {
                    y += 6.0;
                }
                push(&mut rows, &mut y, RowK::Kind(k as u8, list.len()), 36.0);
                let open = self.prefs.sec_open[k];
                let short = list.len() > 6 && !open;
                for (n, &gi) in list.iter().enumerate() {
                    let g = &self.groups[gi];
                    if short && n >= 5 && sel_key.as_deref() != Some(g.key.as_str()) {
                        continue;
                    }
                    push(&mut rows, &mut y, RowK::Group(gi), self.row_h);
                    if g.members.len() > 1 && self.expanded.contains(&g.key) {
                        for mi in 0..g.members.len() {
                            push(&mut rows, &mut y, RowK::Child(gi, mi), self.row_h);
                        }
                    }
                }
                if list.len() > 6 {
                    push(&mut rows, &mut y, RowK::More(k as u8, if open { 0 } else { list.len() }), 32.0);
                }
            }
        } else {
            let pinned: Vec<usize> = (0..self.groups.len()).filter(|&i| self.groups[i].hung).collect();
            let rest: Vec<usize> = (0..self.groups.len()).filter(|&i| !self.groups[i].hung).collect();
            for (n, list) in [&pinned, &rest].iter().enumerate() {
                for &gi in list.iter() {
                    push(&mut rows, &mut y, RowK::Group(gi), self.row_h);
                    let g = &self.groups[gi];
                    if g.members.len() > 1 && self.expanded.contains(&g.key) {
                        for mi in 0..g.members.len() {
                            push(&mut rows, &mut y, RowK::Child(gi, mi), self.row_h);
                        }
                    }
                }
                if n == 0 && !pinned.is_empty() {
                    push(&mut rows, &mut y, RowK::PinSep, 9.0);
                }
            }
        }
        self.content_h[0] = y + 4.0;
        self.rows = rows;
        // the selection never points at a row that is not shown (a search that hides it, a collapsed group):
        // End task, Delete and Details act only on what the user can see
        let shown = |k: &RowK| -> Option<(&str, Option<u32>)> {
            match *k {
                RowK::Group(gi) => self.groups.get(gi).map(|g| (g.key.as_str(), None)),
                RowK::Child(gi, mi) => self.groups.get(gi).and_then(|g| g.members.get(mi).map(|m| (g.key.as_str(), Some(m.pid)))),
                _ => None,
            }
        };
        let fix = match &self.sel {
            Some(Sel::Group(key)) if !self.rows.iter().any(|r| shown(&r.k).map(|(k, c)| k == key && c.is_none()).unwrap_or(false)) => Some(None),
            Some(Sel::Child(pid)) if !self.rows.iter().any(|r| shown(&r.k).map(|(_, c)| c == Some(*pid)).unwrap_or(false)) => {
                // its group row may still be there (the group was collapsed): select that instead
                let key = self.group_of(*pid).map(|g| g.key.clone());
                Some(key.filter(|k| self.rows.iter().any(|r| shown(&r.k).map(|(rk, c)| rk == k && c.is_none()).unwrap_or(false))).map(Sel::Group))
            }
            _ => None,
        };
        if let Some(new) = fix {
            self.sel = new;
        }
        self.clamp_scroll();
        self.note_rows();
    }

    pub fn clamp_scroll(&mut self) {
        for i in 0..4 {
            let max = (self.content_h[i] - self.view_h[i]).max(0.0);
            self.scroll[i] = self.scroll[i].clamp(0.0, max);
        }
    }

    /// The bitmap of an exe icon at `dip` size (16 or 20 use the small icon, bigger ones ask the worker).
    pub fn icon(&mut self, path: &str, dip: f32) -> Option<ID2D1Bitmap> {
        if path.is_empty() {
            return None;
        }
        if self.icon_target != self.gfx.target_id() {
            self.icons.clear();
            self.icon_target = self.gfx.target_id();
        }
        let small = self.ctl.icon_px.load(Relaxed);
        let want = if dip <= 20.0 { small } else { (32.0 * self.gfx.scale()).round() as u32 };
        let k = (path.to_lowercase(), want);
        if let Some(b) = self.icons.get(&k) {
            return b.clone();
        }
        let Some(px) = self.icon_px.get(&k) else {
            if self.icon_asked.insert(k.clone()) {
                self.ctl.icon_req.lock().unwrap().push((path.to_string(), want));
            }
            // meanwhile a big icon falls back to the small one
            if want != small {
                return self.icon(path, 16.0);
            }
            return None;
        };
        let side = ((px.len() / 4) as f64).sqrt() as u32;
        let b = if px.is_empty() || side == 0 { None } else { self.gfx.make_bitmap(side, side, px) };
        self.icons.insert(k, b.clone());
        b
    }

    // ------------------------------------------------------------------ paint
    fn paint(&mut self) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        let (pw, ph) = ((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32);
        self.gfx.dpi = unsafe { GetDpiForWindow(self.hwnd) } as f32;
        let first = self.first_paint_ms.is_none();
        let tp = |m: &str| {
            if first {
                trace(&format!("  paint {m}: {:.1} ms", self.started.elapsed().as_secs_f64() * 1000.0));
            }
        };
        tp("start");
        if !self.gfx.ensure(pw, ph) {
            trace("render target failed");
            return;
        }
        tp("target");
        self.w = pw as f32 / self.gfx.scale();
        self.h = ph as f32 / self.gfx.scale();
        self.hits.clear();
        self.tips.clear();
        self.tick_scroll();
        self.track_ghosts();
        let t = self.t;
        self.gfx.begin();
        // Mica shows through transparent pixels of the window target; screenshots use the solid fallback
        // the window colour at the panel alpha (Settings > Transparency): Mica / Acrylic shows through it evenly;
        // screenshots and Solid use the opaque colour
        if self.backdrop_on() && !self.gfx.preview() {
            self.gfx.clear(t.mica);
        } else {
            self.gfx.clear(t.mica.a(1.0));
        }
        self.paint_title();
        // the sidebar opened over a narrow window is drawn after the page
        let nav_over = self.nav_auto() && (self.nav_peek || self.nav_w > NAV_W + 1.0);
        if !nav_over {
            self.paint_nav();
        }
        if first {
            trace(&format!("  paint title: {:.1} ms", self.started.elapsed().as_secs_f64() * 1000.0));
        }
        match self.page {
            0 => self.paint_processes(),
            1 => self.paint_startup(),
            3 => self.paint_performance(),
            _ => self.paint_settings(),
        }
        if nav_over {
            self.paint_nav();
        }
        self.paint_overlays();
        self.paint_nav_tip();
        self.paint_focus();
        self.paint_tip();
        if first {
            trace(&format!("  paint page: {:.1} ms", self.started.elapsed().as_secs_f64() * 1000.0));
        }
        self.gfx.end();
        if self.first_paint_ms.is_none() {
            self.first_paint_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
        }
    }

    pub fn hit(&mut self, r: Rect, h: Hit) {
        self.hits.push((r, h));
    }

    /// A tooltip for an area of this frame.
    pub fn tip(&mut self, r: Rect, text: impl Into<String>) {
        self.tips.push((r, text.into()));
    }

    /// Hover / press fill for a clickable rect (call before drawing its content).
    pub fn hp(&self, r: Rect, rad: f32, h: Hit) {
        let hv = self.hov(h);
        let pd = self.press_depth(h).clamp(0.0, 1.0);
        if hv > 0.0 {
            self.gfx.fill_round(r, rad, self.t.hov.fade(hv));
        }
        if pd > 0.0 {
            self.gfx.fill_round(r, rad, self.t.prs.fade(pd));
        }
    }

    /// Render one frame offscreen and save it as a 32-bit BMP (lets agents see the GUI without a screen).
    /// `alpha`: keep the window's per-pixel alpha (what DWM gets) and also write `<path>.alpha.bmp` (alpha as grey).
    fn shot(&mut self, path: &str, alpha: bool) -> Result<(), String> {
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
            if !self.gfx.begin_offscreen(dc, w as u32, h as u32, alpha) {
                return Err("offscreen target failed".into());
            }
            self.shooting = true;
            // two passes: the first one loads icons for the new target
            self.paint();
            self.paint();
            self.shooting = false;
            self.gfx.end_offscreen();
            let _ = GdiFlush();
            let n = (w * h * 4) as usize;
            let mut px = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
            if alpha {
                let a: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[3], p[3], p[3], 255]).collect();
                write_bmp(&format!("{path}.alpha.bmp"), w, h, &a)?;
            }
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            self.icon_target = 0;
            self.invalidate();
            write_bmp(path, w, h, &px)
        }
    }

    /// Capture the real window as DWM composes it (caption buttons, backdrop) with PrintWindow(PW_RENDERFULLCONTENT).
    /// Works for a window that is off every monitor (`--offscreen`), so nothing is ever shown on the screen.
    fn shot_real(&mut self, path: &str) -> Result<(), String> {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> windows::core::BOOL;
        }
        unsafe {
            let mut wr = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut wr);
            let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
            let dc = CreateCompatibleDC(None);
            let mut bi = BITMAPINFO::default();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).map_err(|e| e.message())?;
            let old = SelectObject(dc, bmp.into());
            // PW_RENDERFULLCONTENT = 2: ask DWM for the composed window, including Direct2D content
            let ok = PrintWindow(self.hwnd, dc, 2).as_bool();
            let _ = GdiFlush();
            let n = (w * h * 4) as usize;
            let mut px = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
            let transparent = px.chunks_exact(4).filter(|p| p[3] == 0).count();
            for p in px.chunks_exact_mut(4) {
                p[3] = 255;
            }
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            if !ok {
                return Err("PrintWindow failed".into());
            }
            self.last_action = format!("shot-real {w}x{h}, {:.0}% transparent pixels", transparent as f64 * 100.0 / (n / 4).max(1) as f64);
            write_bmp(path, w, h, &px)
        }
    }

    // ------------------------------------------------------------------ selection
    pub fn sel_group(&self) -> Option<&Group> {
        match &self.sel {
            Some(Sel::Group(k)) => self.group(k),
            Some(Sel::Child(pid)) => self.group_of(*pid),
            _ => None,
        }
    }

    /// The pid that "this process" actions use: the child, or the group's root.
    pub fn sel_pid(&self) -> Option<u32> {
        match &self.sel {
            Some(Sel::Child(pid)) => Some(*pid),
            Some(Sel::Group(k)) => self.group(k).map(|g| g.root()),
            _ => None,
        }
    }

    pub fn row_sel(&self, k: RowK) -> bool {
        match (k, &self.sel) {
            (RowK::Group(gi), Some(Sel::Group(key))) => self.groups.get(gi).map(|g| &g.key == key).unwrap_or(false),
            (RowK::Child(gi, mi), Some(Sel::Child(pid))) => self.groups.get(gi).and_then(|g| g.members.get(mi)).map(|m| m.pid == *pid).unwrap_or(false),
            (RowK::Startup(i), Some(Sel::Startup(j))) => i == *j,
            (RowK::Service(i), Some(Sel::Service(j))) => i == *j,
            _ => false,
        }
    }

    fn select_row(&mut self, i: usize) {
        let Some(r) = self.rows.get(i).copied() else { return };
        self.sel = match r.k {
            RowK::Group(gi) => self.groups.get(gi).map(|g| Sel::Group(g.key.clone())),
            RowK::Child(gi, mi) => self.groups.get(gi).and_then(|g| g.members.get(mi)).map(|m| Sel::Child(m.pid)),
            RowK::Startup(i) => Some(Sel::Startup(i)),
            RowK::Service(i) => Some(Sel::Service(i)),
            _ => self.sel.clone(),
        };
        // an open details pane follows the selection
        if let Some(p) = &self.pane
            && p.kind == PaneKind::Details
            && let Some(g) = self.sel_group()
            && g.key != p.key
        {
            let open = p.open;
            self.open_details(open);
        }
    }

    fn sel_index(&self) -> Option<usize> {
        self.rows.iter().position(|r| self.row_sel(r.k))
    }

    fn move_sel(&mut self, d: i32) {
        let n = self.rows.len() as i32;
        let mut i = self.sel_index().map(|i| i as i32).unwrap_or(-1);
        loop {
            i += d;
            if i < 0 || i >= n {
                return;
            }
            if !matches!(self.rows[i as usize].k, RowK::PinSep | RowK::Sec(..) | RowK::Kind(..) | RowK::More(..)) {
                break;
            }
        }
        self.select_row(i as usize);
        self.scroll_to(i as usize);
    }

    /// Open the row menu for the selected row (keyboard), at its left part.
    fn menu_for_sel(&mut self) {
        let Some(i) = self.sel_index() else { return };
        self.scroll_to(i);
        let r = self.rows[i];
        let y = (self.list_rect.y + r.y - self.scroll[0] + r.h / 2.0).clamp(self.list_rect.y, self.list_rect.b());
        self.open_menu(Menu { kind: MenuKind::Row, x: self.list_rect.x + 80.0, y, at: Instant::now(), anchor: None, by: Hit::None }, None);
        self.animate(180);
    }

    /// Move the keyboard to the next (or previous) control of this frame.
    fn tab_focus(&mut self, back: bool) {
        let mut stops: Vec<Hit> = vec![];
        for (_, h) in &self.hits {
            if focusable(*h) && !stops.contains(h) {
                stops.push(*h);
            }
        }
        if stops.is_empty() {
            return;
        }
        let cur = self.focus.or(if self.search_focus { Some(Hit::Search) } else { None }).and_then(|f| stops.iter().position(|h| *h == f));
        let n = stops.len();
        let i = match (cur, back) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(c), false) => (c + 1) % n,
            (Some(c), true) => (c + n - 1) % n,
        };
        let f = stops[i];
        self.focus = Some(f);
        self.search_focus = f == Hit::Search;
        self.key_focus = f == Hit::KeyField;
        self.hex_focus = f == Hit::HexField;
        if f == Hit::List && self.sel_index().is_none() {
            self.sel_edge(true);
        }
    }

    /// Up / Down on the Startup apps page (in its shown order).
    fn move_startup(&mut self, d: i32) {
        // only the rows on screen (folded sections and rows past "Show all" are skipped)
        let rows: Vec<(f32, usize)> = self.startup_rows().into_iter().filter_map(|r| if let super::pages::StRow::Item(i) = r.1 { Some((r.0, i)) } else { None }).collect();
        if rows.is_empty() {
            return;
        }
        let cur = rows.iter().position(|&(_, i)| self.sel == Some(Sel::Startup(i)));
        let n = match cur {
            Some(c) => (c as i32 + d).clamp(0, rows.len() as i32 - 1) as usize,
            None => 0,
        };
        self.sel = Some(Sel::Startup(rows[n].1));
        let y = rows[n].0;
        let view = self.view_h[1];
        if y < self.scroll[1] {
            self.scroll_smooth(1, y);
        } else if y + 56.0 > self.scroll[1] + view {
            self.scroll_smooth(1, y + 56.0 - view);
        }
    }

    /// Home / End: select the first / last row.
    fn sel_edge(&mut self, first: bool) {
        let ok = |k: RowK| !matches!(k, RowK::PinSep | RowK::Sec(..) | RowK::Kind(..) | RowK::More(..));
        let i = if first { self.rows.iter().position(|r| ok(r.k)) } else { self.rows.iter().rposition(|r| ok(r.k)) };
        if let Some(i) = i {
            self.select_row(i);
            self.scroll_to(i);
        }
    }

    pub fn scroll_to(&mut self, i: usize) {
        let Some(r) = self.rows.get(i) else { return };
        let lh = self.view_h[0];
        let (ry, rh) = (r.y, r.h);
        let cur = self.mo.borrow().scroll[0].map(|s| s.target).unwrap_or(self.scroll[0]);
        if ry < cur {
            self.scroll_smooth(0, ry - 4.0);
        } else if ry + rh > cur + lh {
            self.scroll_smooth(0, ry + rh - lh + 4.0);
        }
    }

    /// pid or (part of) a name -> select that row (expands its group for a child pid)
    fn select_target(&mut self, arg: &str) -> Result<(), String> {
        if let Ok(pid) = arg.parse::<u32>() {
            let g = self.group_of(pid).ok_or(format!("no process with PID {pid}"))?.clone();
            if g.members.len() > 1 && g.root() != pid {
                self.expanded.insert(g.key.clone());
                self.sel = Some(Sel::Child(pid));
            } else {
                self.sel = Some(Sel::Group(g.key.clone()));
            }
        } else {
            let a = arg.to_lowercase();
            let g = self.groups.iter().find(|g| g.label.to_lowercase() == a || g.exe.to_lowercase() == a || g.exe.to_lowercase() == format!("{a}.exe")).or_else(|| self.groups.iter().find(|g| g.label.to_lowercase().contains(&a) || g.exe.to_lowercase().contains(&a))).ok_or(format!("no process named '{arg}'"))?;
            self.sel = Some(Sel::Group(g.key.clone()));
        }
        self.page = 0;
        self.build_rows();
        self.want_scroll_sel = true;
        Ok(())
    }

    // ------------------------------------------------------------------ actions
    /// What End task will do for the selection (`process_only` = Shift: only the selected process).
    pub fn end_job(&self, process_only: bool) -> Option<EndJob> {
        let g = self.sel_group()?;
        let child = matches!(self.sel, Some(Sel::Child(_)));
        let (pids, count, mem, label) = if child || process_only {
            let pid = self.sel_pid()?;
            let m = g.members.iter().find(|m| m.pid == pid)?;
            (vec![pid], 1, m.mem, if child { m.name.clone() } else { g.label.clone() })
        } else {
            let mut all = g.pids();
            for p in g.pids() {
                all.extend(procs::descendants(&self.procs, p));
            }
            all.sort();
            all.dedup();
            let n = all.len();
            (all, n, g.mem, g.label.clone())
        };
        let root = if child || process_only { pids[0] } else { g.root() };
        let relaunch = if g.kind == procs::Kind::Windows { None } else { relaunch_info(root) };
        Some(EndJob { pids, label, mem, icon: g.path.clone(), count, relaunch, disable: None })
    }

    /// End task button / Del: confirm first when an app may lose work.
    pub fn end_selected(&mut self, process_only: bool) {
        let Some(job) = self.end_job(process_only) else { return };
        let g = self.sel_group().cloned();
        let risky = g.as_ref().map(|g| g.hung || g.kind == procs::Kind::App).unwrap_or(false);
        let hung = g.as_ref().map(|g| g.hung).unwrap_or(false);
        let body = if hung { format!("{} is not responding. Text you have not sent or saved may be lost.", job.label) } else { format!("{} may have work you have not saved. It will be lost.", job.label) };
        self.ask_end(job, if risky { Some(body) } else { None });
    }

    /// End a job, after a confirm when it needs one: always for a process Windows needs (red warning) and for more
    /// than one process; for an app or a hung app (`risky`) when "Ask before ending" is on.
    pub fn ask_end(&mut self, mut job: EndJob, risky: Option<String>) {
        // the window itself and Windows' kernel (System, PID 4 and below) are never ended: take them out first
        let me = std::process::id();
        job.pids.retain(|p| *p != me && *p > 4);
        if job.pids.is_empty() {
            let mine = self.sel_group().map(|g| g.pids().contains(&me)).unwrap_or(false);
            self.toast_msg(if mine { "This is Rust Task Manager itself: close it with the × button" } else { "Windows' kernel (System) cannot be ended" }, false);
            return;
        }
        job.count = job.pids.len();
        let risky_hung = self.sel_group().map(|g| g.hung).unwrap_or(false);
        let mut crit: Vec<String> = job.pids.iter().filter_map(|p| self.proc(*p)).filter(|p| procs::is_critical_name(&p.name)).map(|p| p.name.clone()).collect();
        crit.sort();
        crit.dedup();
        let (title, body, danger) = if !crit.is_empty() {
            let names = crit.join(", ");
            (format!("End {}?", job.label), format!("Windows needs {names}. Ending it can crash Windows, sign you out or restart the PC, and unsaved work in every app can be lost."), true)
        } else if let Some(b) = risky.filter(|_| self.prefs.ask_end) {
            (format!("End {}?", job.label), if job.count > 1 { format!("{b} All {} of its processes end.", job.count) } else { b }, false)
        } else if job.count > 1 {
            {
                // the row shows its own processes; the end also takes the helpers they started (console hosts...)
                let own = self.sel_group().filter(|g| g.label == job.label).map(|g| g.members.len()).unwrap_or(job.count).min(job.count);
                let helpers = job.count - own;
                let body = if helpers > 0 { format!("{} runs as {own} processes, plus {helpers} helper processes they started. All of them end together.", job.label) } else { format!("{} runs as {} processes. All of them end together.", job.label, job.count) };
                (format!("End {} processes?", job.count), body, false)
            }
        } else {
            self.run_end(job);
            return;
        };
        self.dialog = Some(Dialog { title, body, job, at: Instant::now(), danger, restart: None, cancel: if risky_hung { "Wait" } else { "Cancel" } });
        // Enter confirms a normal dialog; the red one starts on Cancel
        self.dlg_focus = if danger { 1 } else { 0 };
        self.animate(220);
    }

    /// End the job's processes. Returns how many really ended; the toast says what happened, honestly.
    pub fn run_end(&mut self, job: EndJob) -> usize {
        let me = std::process::id();
        let mut fails = 0;
        let mut blocked = 0;
        for p in &job.pids {
            if *p == me || *p <= 4 {
                // the window never ends itself or the kernel (ask_end already took them out)
                fails += 1;
                continue;
            }
            if self.test && !test_owned(&self.procs, *p) {
                // test mode: only processes started by the tester's own scripts may be ended
                fails += 1;
                blocked += 1;
                continue;
            }
            if procs::terminate(*p).is_err() {
                fails += 1;
            }
        }
        if let Some(name) = &job.disable {
            if let Err(e) = crate::startup::set_enabled(name, false) {
                self.toast_msg(&format!("Could not disable {name} at startup: {e}"), false);
            }
            self.ctl.reload_startup.store(true, Relaxed);
        }
        let n = job.pids.len();
        let ended = n - fails;
        self.last_action = format!("end {:?} fails={fails}", job.pids);
        let why = if blocked == fails && blocked > 0 {
            "test mode: not ended"
        } else if self.elevated {
            "access denied"
        } else {
            "needs administrator"
        };
        if ended == 0 && n > 0 {
            self.toast_msg(&format!("Could not end {} ({why})", job.label), false);
        } else if fails > 0 {
            let rest = if why == "needs administrator" { "need administrator" } else { why };
            self.toast_msg(&format!("Ended {ended} of {n} processes of {}; the rest {rest}", job.label), false);
        } else {
            let text = if job.mem >= 1 << 20 { format!("{} ended · freed {}", job.label, model::fmt_mem(job.mem)) } else { format!("{} ended", job.label) };
            self.toast = Some(Toast { text, code: String::new(), link: job.relaunch.clone().map(|(c, d)| ("Restart".to_string(), ToastAct::Relaunch(c, d))), ok: true, at: Instant::now(), secs: 8 });
            self.animate(220);
        }
        if ended > 0 && (matches!(self.sel, Some(Sel::Child(_))) || job.count > 0) {
            self.sel = None;
        }
        if ended > 0
            && let Some(p) = &self.pane
            && p.kind == PaneKind::Details
        {
            self.pane = None;
        }
        self.ctl.now.store(true, Relaxed);
        self.invalidate();
        ended
    }

    /// Restart: end the selection's tree and start its root again with the same command line and folder. It asks
    /// first like End task does (an app with unsaved work, or more than one process).
    pub fn restart_selected(&mut self) {
        let Some(g) = self.sel_group().cloned() else { return };
        let pid = self.sel_pid().unwrap_or(g.root());
        if g.kind == procs::Kind::Windows {
            self.toast_msg("Windows processes are not restarted here", false);
            return;
        }
        let Some(mut job) = self.end_job(matches!(self.sel, Some(Sel::Child(_)))) else { return };
        if job.pids.iter().filter_map(|p| self.proc(*p)).any(|p| procs::is_critical_name(&p.name)) {
            self.toast_msg("Windows needs this process; it is not restarted here", false);
            return;
        }
        if job.pids.contains(&std::process::id()) {
            self.toast_msg("Rust Task Manager does not restart itself", false);
            return;
        }
        let Some((cmd, dir)) = relaunch_info(pid) else {
            self.toast_msg("Cannot read its command line (run as administrator?)", false);
            return;
        };
        job.relaunch = None;
        let risky = (g.kind == procs::Kind::App || g.hung) && self.prefs.ask_end;
        if risky || job.count > 1 {
            let many = if job.count > 1 { format!("All {} of its processes end; only the main one starts again.", job.count) } else { String::new() };
            let body = if risky { format!("{} may have work you have not saved. It will be lost. {many}", job.label).trim_end().to_string() } else { format!("{} runs as {} processes. {many}", job.label, job.count) };
            self.dialog = Some(Dialog { title: format!("Restart {}?", job.label), body, job, at: Instant::now(), danger: false, restart: Some((cmd, dir)), cancel: if g.hung { "Wait" } else { "Cancel" } });
            self.dlg_focus = 0;
            self.animate(220);
        } else {
            self.finish_restart(job, cmd, dir);
        }
    }

    fn finish_restart(&mut self, job: EndJob, cmd: String, dir: String) {
        let label = job.label.clone();
        if self.run_end(job) == 0 {
            return;
        }
        let ok = self.test_or_spawn(&cmd, &dir, 300);
        self.toast = Some(Toast { text: if ok { format!("{label} restarted") } else { format!("Could not start {label} again") }, code: String::new(), link: None, ok, at: Instant::now(), secs: 4 });
        self.animate(220);
    }

    /// The dialog's action button (End / End anyway / Restart).
    pub fn dialog_confirm(&mut self) {
        let Some(d) = self.dialog.take() else { return };
        match d.restart {
            Some((cmd, dir)) => self.finish_restart(d.job, cmd, dir),
            None => {
                self.run_end(d.job);
            }
        }
    }

    /// Start a command again (after a short wait so the old process is gone).
    pub fn test_or_spawn(&mut self, cmd: &str, dir: &str, wait_ms: u64) -> bool {
        if self.test && !cmd.to_lowercase().contains("wt_test") {
            self.last_action = format!("spawn blocked in test mode: {cmd}");
            return true;
        }
        let (cmd, dir) = (cmd.to_string(), dir.to_string());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(wait_ms));
            spawn(&cmd, &dir);
        });
        true
    }

    pub fn toast_msg(&mut self, s: &str, ok: bool) {
        self.toast = Some(Toast { text: s.to_string(), code: String::new(), link: None, ok, at: Instant::now(), secs: 4 });
        self.animate(220);
    }

    pub fn copy(&mut self, s: &str) {
        if self.test {
            self.last_action = format!("copy (test mode, clipboard untouched): {s}");
        } else {
            set_clipboard(self.hwnd, s);
        }
        self.toast = Some(Toast { text: "Copied".into(), code: s.to_string(), link: None, ok: true, at: Instant::now(), secs: 4 });
        self.animate(220);
    }

    /// "Copy as command" for the selection.
    pub fn copy_command(&mut self) {
        let s = match &self.sel {
            Some(Sel::Child(pid)) => format!("wtm kill {pid}"),
            Some(Sel::Group(_)) => match self.sel_group() {
                Some(g) if g.members.len() > 1 => format!("wtm kill {} --tree", g.exe),
                Some(g) => format!("wtm kill {}", g.members[0].pid),
                None => return,
            },
            _ => return,
        };
        self.copy(&s);
    }

    /// Start an elevated copy of the window (Windows asks with UAC) and close this one when it starts.
    pub fn run_as_admin(&mut self) {
        if self.test {
            self.last_action = "run as administrator (test mode, not started)".into();
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let exe = wide(&exe.display().to_string());
        let r = unsafe { ShellExecuteW(Some(self.hwnd), w!("runas"), PCWSTR(exe.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
        if r.0 as isize > 32 {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        } else {
            self.toast_msg("Not started (the administrator prompt was declined)", false);
        }
    }

    pub fn open_location(&mut self) {
        let Some(g) = self.sel_group() else { return };
        let path = self.sel_pid().and_then(|p| self.proc(p)).map(|p| p.meta.path.clone()).filter(|p| !p.is_empty()).unwrap_or(g.path.clone());
        if path.is_empty() {
            self.toast_msg("Its file location can be read only as administrator", false);
            return;
        }
        if self.test {
            self.last_action = format!("open location (test mode, not launched): {path}");
            return;
        }
        let args = wide(&format!("/select,\"{path}\""));
        unsafe {
            ShellExecuteW(Some(self.hwnd), w!("open"), w!("explorer.exe"), PCWSTR(args.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }

    pub fn properties(&mut self) {
        let Some(g) = self.sel_group() else { return };
        let path = g.path.clone();
        if path.is_empty() {
            self.toast_msg("Its file can be read only as administrator", false);
            return;
        }
        if self.test {
            self.last_action = format!("properties (test mode, not shown): {path}");
            return;
        }
        let p = wide(&path);
        unsafe {
            let _ = windows::Win32::UI::Shell::SHObjectProperties(Some(self.hwnd), windows::Win32::UI::Shell::SHOP_FILEPATH, PCWSTR(p.as_ptr()), PCWSTR::null());
        }
    }

    /// Open the details pane for the selection. `open` = which sections start open (what, why, details).
    pub fn open_details(&mut self, open: [bool; 3]) {
        let Some(g) = self.sel_group().cloned() else { return };
        let pid = self.sel_pid().unwrap_or(g.root());
        let Some(p) = self.proc(pid).cloned() else { return };
        let was_open = self.pane.as_ref().map(|p| p.kind == PaneKind::Details).unwrap_or(false);
        self.pane = Some(Pane { kind: PaneKind::Details, key: g.key.clone(), pid, open, info: PaneInfo::default(), loading: true, ai_loading: false, keep_open: false, at: if was_open { Instant::now() - Duration::from_secs(1) } else { Instant::now() } });
        self.set_scroll(3, 0.0);
        self.animate(220);
        let out = self.pane_result.clone();
        let hwnd = self.hwnd.0 as isize;
        let procs = self.procs.clone();
        let startup = self.startup.clone();
        let services = self.services_all.clone();
        let cmd = self.cmdlines.get(&pid).cloned().unwrap_or_default();
        let ai = self.ai_on;
        std::thread::spawn(move || {
            let signed = if p.meta.path.is_empty() { None } else { Some(crate::explain::is_signed(&p.meta.path)) };
            let local = crate::explain::explain(&p, signed);
            let mut info = pane_facts(&p, &procs, &startup, &services, signed);
            info.cmdline = cmd.clone();
            info.answer = Some(local.clone());
            info.local = Some(local.clone());
            // the AI is offered only for unknown processes, and only when the user turned it on
            let want_ai = ai && local.source == "file-info";
            *out.lock().unwrap() = Some((pid, info.clone(), want_ai));
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_PANEL, WPARAM(0), LPARAM(0));
            }
            if want_ai && crate::ai::enabled() {
                if let Ok(a) = crate::ai::ask(&p, &cmd, &local) {
                    info.answer = Some(a);
                }
                *out.lock().unwrap() = Some((pid, info, false));
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_PANEL, WPARAM(1), LPARAM(0));
                }
            }
        });
        self.invalidate();
    }

    fn take_panel(&mut self) {
        let r = self.pane_result.lock().unwrap().take();
        if let (Some((pid, info, ai_pending)), Some(p)) = (r, self.pane.as_mut())
            && p.pid == pid
        {
            p.info = info;
            p.loading = false;
            p.ai_loading = ai_pending;
        }
        self.invalidate();
    }

    pub fn open_free(&mut self) {
        if self.pane.as_ref().map(|p| p.kind == PaneKind::Free).unwrap_or(false) {
            self.pane = None;
        } else {
            self.pane = Some(Pane { kind: PaneKind::Free, key: String::new(), pid: 0, open: [false; 3], info: PaneInfo::default(), loading: false, ai_loading: false, keep_open: false, at: Instant::now() });
            self.set_scroll(3, 0.0);
            self.flyout = None;
            self.animate(220);
        }
        self.invalidate();
    }

    /// Free up memory candidates: (safe to end, better to keep).
    pub fn free_lists(&self) -> (Vec<crate::insight::FreeItem>, Vec<crate::insight::FreeItem>) {
        let groups = crate::insight::merge(&self.procs, &self.cmdlines, &self.services);
        let growing: Vec<u32> = self.hist_anoms.iter().filter(|a| a.kind == "memory_growing").map(|a| a.pid).collect();
        let all = crate::insight::free_candidates(&groups, &self.procs, &growing, &self.startup);
        let fg = foreground_pid();
        let (mut go, mut keep) = (vec![], vec![]);
        for it in all {
            if it.verdict == crate::insight::FreeVerdict::Safe && !it.pids.contains(&fg) {
                go.push(it);
            } else {
                keep.push(it);
            }
        }
        (go, keep)
    }

    pub fn free_end(&mut self, i: usize, disable: bool) {
        let (go, _) = self.free_lists();
        let Some(it) = go.get(i) else { return };
        let path = it.pids.first().and_then(|p| self.proc(*p)).map(|p| p.meta.path.clone()).unwrap_or_default();
        let job = EndJob { pids: it.pids.clone(), label: it.label.clone(), mem: it.mem, icon: path, count: it.pids.len(), relaunch: it.pids.first().and_then(|p| relaunch_info(*p)), disable: if disable { it.startup_entry.clone() } else { None } };
        // more than one process asks first, like End task
        self.ask_end(job, None);
        self.pane = Some(Pane { kind: PaneKind::Free, key: String::new(), pid: 0, open: [false; 3], info: PaneInfo::default(), loading: false, ai_loading: false, keep_open: false, at: Instant::now() - Duration::from_secs(1) });
    }

    pub fn free_all(&mut self) {
        let (go, _) = self.free_lists();
        if go.is_empty() {
            return;
        }
        let mem: u64 = go.iter().map(|i| i.mem).sum();
        let pids: Vec<u32> = go.iter().flat_map(|i| i.pids.clone()).collect();
        let n = go.len();
        let job = EndJob { pids, label: format!("{n} apps"), mem, icon: String::new(), count: n, relaunch: None, disable: None };
        // ends several apps at once: always asks (ask_end confirms more than one process)
        self.ask_end(job, None);
        self.pane = Some(Pane { kind: PaneKind::Free, key: String::new(), pid: 0, open: [false; 3], info: PaneInfo::default(), loading: false, ai_loading: false, keep_open: false, at: Instant::now() - Duration::from_secs(1) });
    }

    pub fn toggle_startup(&mut self, i: usize) {
        let Some(e) = self.startup.get(i).cloned() else { return };
        let res = if self.test {
            // test mode: only this window's copy changes (the switch and the row move can be seen), never Windows
            self.last_action = format!("startup toggle in test mode (not written): {}", e.name);
            Ok(e.clone())
        } else if e.source == "Service" {
            self.toggle_service(&e)
        } else {
            crate::startup::set_enabled_in(&e.name, Some(e.source), !e.enabled)
        };
        match res {
            Ok(_) => {
                let mut v = (*self.startup).clone();
                v[i].enabled = !e.enabled;
                self.startup = Arc::new(v);
                // the row keeps its place while the switch slides, then moves to its group
                self.startup_hold = Some((i, e.enabled, Instant::now()));
                self.animate(super::pages::STARTUP_HOLD.as_millis() as u64 + 60);
                if !self.test {
                    self.ctl.reload_startup.store(true, Relaxed);
                }
            }
            Err(err) => self.toast_msg(&format!("Could not change {}: {err}", e.name), false),
        }
        self.invalidate();
    }

    /// A service at startup: one UAC prompt runs `RustTaskManager.exe --service-start <name> manual|auto[-delayed]`.
    /// Off = Manual (it keeps running now), on = Automatic again (delayed if it was).
    fn toggle_service(&mut self, e: &crate::startup::Entry) -> Result<crate::startup::Entry, String> {
        let svc = crate::startup::service_name(&e.name).ok_or_else(|| "the service is gone".to_string())?;
        let on = !e.enabled;
        let delayed_now = !on && crate::services::is_delayed(&svc);
        let mode = if !on {
            "manual"
        } else if crate::startup::switched_off().get(&svc.to_lowercase()).copied().unwrap_or(false) {
            "auto-delayed"
        } else {
            "auto"
        };
        match super::prefs::run_elevated_wait(self.hwnd, &format!("--service-start \"{svc}\" {mode}")) {
            Ok(0) => {
                crate::startup::remember_switch(&svc, !on, delayed_now);
                self.toast_msg(&if on { format!("{} starts with Windows again (from its next start)", e.name) } else { format!("{} will not start with Windows any more. It keeps running now.", e.name) }, true);
                Ok(crate::startup::Entry { enabled: on, ..e.clone() })
            }
            Ok(3) => Err("administrator rights were not granted".into()),
            Ok(c) => Err(format!("the administrator helper failed (code {c})")),
            Err(x) if x == "cancelled" => Err("the administrator prompt was cancelled; nothing changed".into()),
            Err(x) => Err(x),
        }
    }

    pub fn service_action(&mut self, i: usize) {
        let Some(s) = self.services_all.get(i).cloned() else { return };
        let act = if s.state == "running" { "stop" } else { "start" };
        if self.test {
            self.last_action = format!("service {act} blocked in test mode: {}", s.name);
            return;
        }
        let name = s.name.clone();
        match crate::services::control(&name, act) {
            Ok(()) => self.toast_msg(&format!("{} {}", s.display, if act == "stop" { "stopped" } else { "started" }), true),
            Err(e) => self.toast_msg(&format!("Could not {act} {}: {e}", s.display), false),
        }
        self.ctl.now.store(true, Relaxed);
    }

    pub fn set_default(&mut self, set: bool) {
        if self.test {
            self.last_action = format!("default {} blocked in test mode", if set { "set" } else { "restore" });
            return;
        }
        if !prefs::launch_helper(self.hwnd, set) {
            self.toast_msg("Windows did not allow the change", false);
        }
        // the helper runs elevated; read the result a moment later
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_TICK, 1500, None);
        }
    }

    /// Save the typed API key (stored DPAPI-encrypted by ai.rs; never shown again).
    pub fn key_save(&mut self) {
        let k = std::mem::take(&mut self.key_buf);
        let k = k.trim().to_string();
        if k.is_empty() {
            return;
        }
        self.key_result = None;
        if self.test {
            self.last_action = "api key save blocked in test mode (key not stored)".into();
            self.key_saved = true;
            return;
        }
        match crate::ai::set_key(&k) {
            Ok(()) => {
                self.key_saved = crate::ai::has_key();
                self.key_focus = false;
            }
            Err(e) => self.key_result = Some(Err(e)),
        }
        self.ai_on = crate::ai::enabled();
    }

    pub fn key_remove(&mut self) {
        self.key_result = None;
        if self.test {
            self.last_action = "api key remove blocked in test mode".into();
            self.key_saved = false;
            return;
        }
        if let Err(e) = crate::ai::clear_key() {
            self.toast_msg(&e, false);
        }
        self.key_saved = crate::ai::has_key();
        self.ai_on = crate::ai::enabled();
    }

    /// "Test key": the typed key, or the saved one. Runs off the UI thread.
    pub fn key_test(&mut self) {
        if self.key_testing {
            return;
        }
        if self.test {
            self.last_action = "api key test blocked in test mode".into();
            self.key_result = Some(Err("Not tested (test mode)".into()));
            return;
        }
        let typed = self.key_buf.trim().to_string();
        if typed.is_empty() && !self.key_saved {
            self.key_result = Some(Err("Type or save a key first".into()));
            return;
        }
        self.key_testing = true;
        self.key_result = None;
        let (out, hwnd) = (self.key_out.clone(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            let r = crate::ai::test_key(if typed.is_empty() { None } else { Some(&typed) });
            *out.lock().unwrap() = Some(r);
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_KEYTEST, WPARAM(0), LPARAM(0));
            }
        });
    }

    pub fn open_key_page(&mut self) {
        if self.test {
            self.last_action = format!("open {} (test mode, not launched)", crate::ai::GET_KEY_URL);
            return;
        }
        let url = wide(crate::ai::GET_KEY_URL);
        unsafe {
            ShellExecuteW(Some(self.hwnd), w!("open"), PCWSTR(url.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }

    pub fn set_ai(&mut self, on: bool) {
        if self.test {
            self.last_action = format!("ai switch blocked in test mode: {on}");
            return;
        }
        if let Err(e) = crate::ai::set_enabled(on) {
            self.toast_msg(&e, false);
        }
        self.ai_on = crate::ai::enabled();
    }

    pub fn set_page(&mut self, p: u8) {
        self.nav_peek = false;
        if self.page != p {
            self.page = p;
            self.menu = None;
            self.flyout = None;
        }
        if p == 2 {
            self.is_default = prefs::is_default();
            self.ai_on = crate::ai::enabled();
            self.key_saved = crate::ai::has_key();
        }
        self.invalidate();
    }

    pub fn set_sort(&mut self, s: SortKey) {
        self.sort = s;
        self.order_prev.clear();
        self.last_order = None;
        self.rebuild_groups(false);
    }

    pub fn set_pref(&mut self, f: impl FnOnce(&mut Prefs)) {
        let old = self.prefs.clone();
        f(&mut self.prefs);
        if !self.test {
            self.prefs.save();
        }
        if old.theme != self.prefs.theme {
            self.theme_override = None;
            self.apply_theme();
        }
        if old.interval_ms != self.prefs.interval_ms {
            self.ctl.interval_ms.store(self.prefs.interval_ms, Relaxed);
        }
        if old.merge != self.prefs.merge {
            self.order_prev.clear();
            self.rebuild_groups(false);
        }
        let p = &self.prefs;
        if (old.backdrop, old.corners, old.font, old.density) != (p.backdrop, p.corners, p.font, p.density) {
            self.apply_look();
        } else if (old.accent, old.transp, old.desktop, old.strength) != (p.accent, p.transp, p.desktop, p.strength) {
            self.apply_theme();
        }
        if old.sort != self.prefs.sort
            && let Some(s) = SortKey::parse(&self.prefs.sort)
        {
            self.set_sort(s);
        }
        self.invalidate();
    }

    /// Settings > Accent > Custom: set the accent from a hue (0..360) at a pleasant saturation.
    pub fn set_hue(&mut self, hue: f32) {
        let c = theme::from_hsl(hue, 0.78, 0.45);
        let rgb = ((c.0 * 255.0).round() as u32) << 16 | ((c.1 * 255.0).round() as u32) << 8 | (c.2 * 255.0).round() as u32;
        self.hex_buf = theme::hex(c);
        self.set_pref(|p| p.accent = Some(rgb));
    }

    /// Settings > Accent > Custom: the hex field is complete -> apply it.
    fn hex_apply(&mut self) {
        if let Some(rgb) = prefs::parse_hex(&self.hex_buf) {
            self.set_pref(|p| p.accent = Some(rgb));
        }
    }

    // ------------------------------------------------------------------ input
    pub fn hit_at(&self, x: f32, y: f32) -> Hit {
        for (r, h) in self.hits.iter().rev() {
            if r.has(x, y) {
                return *h;
            }
        }
        Hit::None
    }

    fn mouse_down(&mut self, x: f32, y: f32, double: bool) {
        let mut h = self.hit_at(x, y);
        // a row index from the last paint -> the same row now
        if let Hit::Row(i) | Hit::Chevron(i) = h {
            h = match self.row_now(i) {
                Some(j) if matches!(h, Hit::Row(_)) => Hit::Row(j),
                Some(j) => Hit::Chevron(j),
                None => Hit::None,
            };
        }
        self.last_input = Instant::now();
        // light dismiss: a click outside an open flyout or the sidebar opened over the page only closes it
        let in_nav = matches!(h, Hit::Tab(_) | Hit::NavToggle | Hit::Block) && x < self.nav_w + 1.0;
        if self.nav_peek && !in_nav {
            self.nav_peek = false;
            self.invalidate();
            return;
        }
        if self.flyout.is_some() && !matches!(h, Hit::Tile(_) | Hit::Block | Hit::FlyFree) {
            self.flyout = None;
            self.invalidate();
            return;
        }
        // clicks outside a popup close it (same as File Explorer)
        let in_menu = matches!(h, Hit::MenuItem(_)) || (matches!(h, Hit::Block) && self.menu_r.map(|r| r.has(x, y)).unwrap_or(false));
        if !in_menu && self.menu.is_some() {
            let anchor_was = self.menu.as_ref().and_then(|m| m.anchor);
            let by_was = self.menu.as_ref().map(|m| m.by);
            self.menu = None;
            self.invalidate();
            // a click on the same dropdown button just closes it: the button that opened it, or any hit drawn at
            // exactly the dropdown's anchor rectangle
            let hit_rect = self.hits.iter().rev().find(|x| x.1 == h).map(|x| x.0);
            if drop_toggle_click(h, by_was, anchor_was, hit_rect) {
                return;
            }
        }
        self.pressed = h;
        self.animate(motion::FAST as u64);
        self.key_focus = h == Hit::KeyField || (self.key_focus && matches!(h, Hit::KeySave | Hit::KeyTest));
        if self.hex_focus && h != Hit::HexField {
            self.hex_focus = false;
            self.hex_apply();
        }
        if h == Hit::Hue {
            self.hue_at(x);
        }
        // a click on a row (or anything in the list) moves the keyboard to the list
        self.search_focus = h == Hit::Search || h == Hit::SearchClear || (self.search_focus && self.searching() && !matches!(h, Hit::Tab(_) | Hit::Settings | Hit::Row(_) | Hit::Chevron(_) | Hit::StRow(_) | Hit::Header(_)));
        self.focus = None;
        match h {
            Hit::Row(i) if matches!(self.rows.get(i).map(|r| r.k), Some(RowK::More(..))) => {
                if let Some(RowK::More(k, _)) = self.rows.get(i).map(|r| r.k) {
                    self.toggle_section(k);
                }
            }
            Hit::Row(i) if matches!(self.rows.get(i).map(|r| r.k), Some(RowK::Kind(..))) => {}
            Hit::Row(i) | Hit::Chevron(i) => {
                if h == Hit::Chevron(i) {
                    self.toggle_row(i);
                } else {
                    self.select_row(i);
                    if double {
                        if matches!(self.rows.get(i).map(|r| r.k), Some(RowK::Group(_))) && self.sel_group().map(|g| g.members.len() > 1).unwrap_or(false) {
                            self.toggle_row(i);
                        } else {
                            self.open_details([true, false, false]);
                        }
                    }
                }
            }
            Hit::StRow(i) => {
                self.sel = Some(Sel::Startup(i));
                self.open_st_pane(i);
            }
            Hit::StPaneClose => self.st_pane = None,
            Hit::StSecHead(k) => self.set_pref(|p| p.st_fold[k as usize % 3] = !p.st_fold[k as usize % 3]),
            Hit::StMore(k) => self.set_pref(|p| p.st_all[k as usize % 3] = !p.st_all[k as usize % 3]),
            Hit::StFoldAll => {
                let fold = (0..3).any(|k| !self.prefs.st_fold[k]);
                self.set_pref(|p| p.st_fold = [fold; 3]);
            }
            Hit::Scroll(k) => {
                self.drag_scroll = Some((k, y, self.scroll[k as usize]));
                unsafe {
                    SetCapture(self.hwnd);
                }
            }
            _ => {}
        }
        unsafe {
            SetCapture(self.hwnd);
        }
        self.invalidate();
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let h = self.hit_at(x, y);
        let down = std::mem::replace(&mut self.pressed, Hit::None);
        unsafe {
            let _ = ReleaseCapture();
        }
        if self.drag_scroll.take().is_some() {
            self.invalidate();
            return;
        }
        if h != down {
            self.invalidate();
            return;
        }
        self.click(h, x, y);
        self.invalidate();
    }

    fn toggle_row(&mut self, i: usize) {
        if let Some(RowK::Group(gi)) = self.rows.get(i).map(|r| r.k) {
            let key = self.groups[gi].key.clone();
            if !self.expanded.remove(&key) {
                self.expanded.insert(key);
            }
            self.build_rows();
        }
    }

    /// The one way a dropdown opens (same as File Explorer's `open_drop`). It remembers the button that was pressed to
    /// open it (and its rectangle), so a second click on that button closes it instead of opening it again (see
    /// `drop_toggle_click`), and the menu opens under that button.
    pub fn open_menu(&mut self, mut m: Menu, by: Option<Hit>) {
        if let Some(h) = by {
            m.by = h;
            m.anchor = self.hits.iter().rev().find(|x| x.1 == h).map(|x| x.0);
        }
        self.menu = Some(m);
    }

    pub fn click(&mut self, h: Hit, x: f32, y: f32) {
        // an open menu takes the click (the mouse already closed it on mouse-down when the click was outside it)
        if self.menu.is_some() {
            if let Hit::MenuItem(i) = h {
                self.menu_action(i);
                return;
            }
            self.menu = None;
        }
        if self.dialog.is_some() {
            match h {
                Hit::DlgEnd => self.dialog_confirm(),
                Hit::DlgWait => self.dialog = None,
                _ => {}
            }
            return;
        }
        match h {
            Hit::Tab(i) => {
                self.set_page(i);
            }
            Hit::Settings => self.set_page(if self.page == 2 { 0 } else { 2 }),
            Hit::SearchClear => {
                self.query.clear();
                self.build_rows();
            }
            Hit::Tile(i) => {
                if self.flyout.map(|f| f.0) == Some(i) {
                    self.flyout = None;
                } else {
                    self.flyout = Some((i, Instant::now()));
                    self.animate(180);
                }
            }
            Hit::Chip(i) | Hit::ChipEnd(i) => {
                let chips = self.chips();
                if let Some(c) = chips.get(i as usize) {
                    let key = c.key.clone();
                    self.sel = Some(Sel::Group(key));
                    if let Some(idx) = self.sel_index() {
                        self.scroll_to(idx);
                    }
                    if h == Hit::ChipEnd(i) {
                        self.end_selected(false);
                    }
                }
            }
            Hit::ChipMore => self.open_menu(Menu { kind: MenuKind::Chips, x, y: y + 8.0, at: Instant::now(), anchor: None, by: Hit::None }, Some(h)),
            Hit::EndTask => self.end_selected(self.shift),
            Hit::Restart => self.restart_selected(),
            Hit::Details => {
                if self.pane.as_ref().map(|p| p.kind == PaneKind::Details).unwrap_or(false) {
                    self.pane = None;
                } else {
                    self.open_details([true, false, false]);
                }
            }
            Hit::More | Hit::PaneMore => {
                if self.sel_group().is_some() {
                    self.open_menu(Menu { kind: MenuKind::Row, x, y: y + 10.0, at: Instant::now(), anchor: None, by: Hit::None }, Some(h));
                    self.animate(180);
                }
            }
            Hit::Free | Hit::FlyFree => self.open_free(),
            Hit::Sort => {
                self.open_menu(Menu { kind: MenuKind::Sort, x: x - 40.0, y: y + 14.0, at: Instant::now(), anchor: None, by: Hit::None }, Some(h));
                self.animate(180);
            }
            Hit::Header(c) => {
                // first click sorts by the column, the second reverses it, the third goes back to Impact
                let s = [SortKey::Name, SortKey::Cpu, SortKey::Mem, SortKey::Disk, SortKey::Net, SortKey::Gpu][c as usize];
                if self.sort == s && !self.sort_rev {
                    self.sort_rev = true;
                    self.set_sort(s);
                } else if self.sort == s {
                    self.sort_rev = false;
                    self.set_sort(SortKey::Impact);
                } else {
                    self.sort_rev = false;
                    self.set_sort(s);
                }
            }
            Hit::SrStartupToggle(i) | Hit::StToggle(i) => self.toggle_startup(i),
            Hit::SrServiceBtn(i) => self.service_action(i),
            Hit::PaneClose => self.pane = None,
            Hit::PaneEnd => self.end_selected(false),
            Hit::PerfItem(i) => {
                if let Some(it) = self.perf_items().get(i as usize) {
                    self.perf_sel = it.key.clone();
                    self.set_scroll(2, 0.0);
                }
            }
            Hit::PerfTop(i) => {
                // show that process in the list
                if let Some(key) = self.perf_top_keys.get(i as usize).cloned() {
                    self.query.clear();
                    self.sel = Some(Sel::Group(key));
                    self.set_page(0);
                    self.build_rows();
                    self.want_scroll_sel = true;
                }
            }
            Hit::RunAdmin => self.run_as_admin(),
            Hit::PaneRestart => self.restart_selected(),
            Hit::PaneSec(i) => {
                if let Some(p) = self.pane.as_mut() {
                    p.open[i as usize] = !p.open[i as usize];
                }
            }
            Hit::PaneDisable(i) => {
                if let Some(e) = self.pane.as_ref().and_then(|p| p.info.startup.get(i).cloned()) {
                    if let Some(idx) = self.startup.iter().position(|s| s.name == e.name) {
                        self.toggle_startup(idx);
                    }
                    if let Some(p) = self.pane.as_mut()
                        && let Some(s) = p.info.startup.get_mut(i)
                        && !self.test
                    {
                        s.enabled = !s.enabled;
                    }
                }
            }
            Hit::FreeEnd(i) => self.free_end(i, false),
            Hit::FreeEndDisable(i) => self.free_end(i, true),
            Hit::FreeKeep => {
                if let Some(p) = self.pane.as_mut() {
                    p.keep_open = !p.keep_open;
                }
            }
            Hit::FreeAll => self.free_all(),
            Hit::ToastClose => self.toast = None,
            Hit::ToastLink => {
                if let Some(Toast { link: Some((_, ToastAct::Relaunch(c, d))), .. }) = self.toast.take() {
                    self.test_or_spawn(&c, &d, 0);
                }
            }
            Hit::StSort => self.open_menu(Menu { kind: MenuKind::StartupSort, x: x - 60.0, y: y + 14.0, at: Instant::now(), anchor: None, by: Hit::None }, Some(h)),
            Hit::SetDefault => self.set_default(true),
            Hit::RestoreDefault => self.set_default(false),
            Hit::Seg(g, i) => match g {
                0 => self.set_pref(|p| p.theme = i),
                1 => self.set_pref(|p| p.interval_ms = [500, 1000, 2000, 5000][i as usize]),
                3 => self.set_pref(|p| p.backdrop = i),
                4 => self.set_pref(|p| p.transp = i),
                5 => self.set_pref(|p| p.density = i),
                6 => self.set_pref(|p| p.font = i),
                7 => self.set_pref(|p| p.corners = i),
                8 => self.set_pref(|p| p.pane = i),
                9 => self.set_pref(|p| p.strength = i.min(2)),
                _ => {
                    let l = if i == 1 { "fa" } else { "en" };
                    if !self.test {
                        let _ = crate::ai::set_lang(l);
                    }
                    self.lang_fa = Some(i == 1);
                }
            },
            Hit::Toggle(i) => match i {
                0 => self.set_pref(|p| p.merge = !p.merge),
                1 => self.set_pref(|p| p.ask_end = !p.ask_end),
                2 => self.set_pref(|p| p.show_pid = !p.show_pid),
                4 => self.set_pref(|p| p.tiles = !p.tiles),
                5 => self.set_pref(|p| p.chips = !p.chips),
                6 => {
                    self.set_pref(|p| p.group = !p.group);
                    self.build_rows();
                }
                _ => {
                    let on = !self.ai_on;
                    self.set_ai(on)
                }
            },
            Hit::KeyField => self.key_focus = true,
            Hit::KeySave => self.key_save(),
            Hit::KeyTest => self.key_test(),
            Hit::KeyRemove => self.key_remove(),
            Hit::KeyLink => self.open_key_page(),
            Hit::Accent(i) => {
                if i == 0 {
                    self.custom_open = false;
                    self.set_pref(|p| {
                        p.accent = None;
                        p.desktop = false
                    });
                } else if i == 10 {
                    // Follow desktop: the wallpaper / DWM colour, refreshed when Windows changes it
                    self.custom_open = false;
                    self.set_pref(|p| p.desktop = true);
                } else if let Some((_, rgb)) = theme::PRESETS.get(i as usize - 1) {
                    self.custom_open = false;
                    let rgb = *rgb;
                    self.set_pref(|p| {
                        p.accent = Some(rgb);
                        p.desktop = false
                    });
                } else {
                    self.set_pref(|p| p.desktop = false);
                    self.custom_open = !self.custom_open;
                    if self.custom_open {
                        let cur = self.prefs.accent.unwrap_or(0x0078D4);
                        self.hex_buf = format!("#{cur:06X}");
                        if self.prefs.accent.is_none() {
                            self.set_pref(|p| p.accent = Some(cur));
                        }
                    }
                }
            }
            Hit::HexField => {
                self.hex_focus = true;
                self.key_focus = false;
                self.search_focus = false;
            }
            Hit::ColPill(i) => self.set_pref(|p| match i {
                0 => p.col_type = !p.col_type,
                1 => p.show_pid = !p.show_pid,
                2 => p.col_disk = !p.col_disk,
                3 => p.col_net = !p.col_net,
                _ => p.col_gpu = !p.col_gpu,
            }),
            Hit::NavToggle => {
                if self.nav_auto() {
                    // small window: open / close the sidebar over the page (the saved choice is for wider windows)
                    self.nav_peek = !self.nav_peek;
                } else {
                    let open = !self.prefs.nav_open;
                    self.set_pref(|p| p.nav_open = open);
                }
            }
            Hit::DefSort => {
                self.open_menu(Menu { kind: MenuKind::DefSort, x: x - 60.0, y: y + 14.0, at: Instant::now(), anchor: None, by: Hit::None }, Some(h));
            }
            _ => {}
        }
    }

    /// Hue strip: pick the hue under x.
    fn hue_at(&mut self, x: f32) {
        let r = self.hue_rect;
        if r.w > 0.0 {
            self.set_hue(((x - r.x) / r.w).clamp(0.0, 1.0) * 359.0);
        }
    }

    fn menu_action(&mut self, i: u8) {
        let Some(m) = self.menu.take() else { return };
        match m.kind {
            MenuKind::Sort => {
                if let Some(s) = SortKey::MENU.get(i as usize) {
                    self.sort_rev = false;
                    self.set_sort(*s);
                }
            }
            MenuKind::StartupSort => self.startup_sort = i,
            MenuKind::DefSort => {
                if let Some(s) = SortKey::MENU.get(i as usize) {
                    let name = s.label().to_lowercase();
                    self.set_pref(|p| p.sort = name);
                }
            }
            MenuKind::Chips => {
                let chips = self.chips();
                if let Some(c) = chips.get(3 + i as usize) {
                    self.sel = Some(Sel::Group(c.key.clone()));
                    if let Some(idx) = self.sel_index() {
                        self.scroll_to(idx);
                    }
                }
            }
            MenuKind::Row => match i {
                0 => self.end_selected(false),
                1 => self.end_all_with_name(),
                2 => self.restart_selected(),
                3 => self.open_details([true, false, false]),
                4 => self.open_details([false, true, false]),
                5 => self.open_location(),
                6 => self.properties(),
                7 => self.copy_command(),
                _ => {}
            },
        }
        self.invalidate();
    }

    fn end_all_with_name(&mut self) {
        let Some(g) = self.sel_group() else { return };
        let exe = g.exe.to_lowercase();
        let pids: Vec<u32> = self.procs.iter().filter(|p| p.name.to_lowercase() == exe).map(|p| p.pid).collect();
        let mem = self.procs.iter().filter(|p| p.name.to_lowercase() == exe).map(|p| p.mem).sum();
        let job = EndJob { count: pids.len(), pids, label: g.exe.clone(), mem, icon: g.path.clone(), relaunch: None, disable: None };
        self.ask_end(job, None);
    }

    fn wheel(&mut self, x: f32, y: f32, d: f32) {
        self.menu = None;
        let k = if self.page == 1 {
            1
        } else if self.page == 2 || self.page == 3 {
            2
        } else if self.pane.is_some() && self.pane_rect.has(x, y) {
            3
        } else {
            0
        };
        self.scroll_by(k, -d / 120.0 * self.row_h * 3.0);
        self.invalidate();
    }

    /// The caret position in the search text (chars).
    pub fn qcaret(&self) -> usize {
        self.caret.min(self.query.chars().count())
    }

    fn q_byte(&self, c: usize) -> usize {
        self.query.char_indices().nth(c).map(|(i, _)| i).unwrap_or(self.query.len())
    }

    /// Insert text at the caret (replaces everything after Ctrl+A).
    fn q_insert(&mut self, txt: &str) {
        if self.q_all {
            self.query.clear();
            self.q_all = false;
        }
        let c = self.qcaret();
        let b = self.q_byte(c);
        self.query.insert_str(b, txt);
        self.caret = c + txt.chars().count();
    }

    /// Delete in the search text: `back` = Backspace (before the caret) else Delete; `word` = Ctrl (a whole word).
    fn q_delete(&mut self, back: bool, word: bool) {
        if self.q_all {
            self.query.clear();
            self.q_all = false;
            self.caret = 0;
            return;
        }
        let chars: Vec<char> = self.query.chars().collect();
        let c = self.qcaret();
        let (a, b) = if back {
            let mut a = c.saturating_sub(1);
            if word {
                while a > 0 && chars[a - 1] == ' ' {
                    a -= 1;
                }
                while a > 0 && chars[a - 1] != ' ' {
                    a -= 1;
                }
            }
            (a, c)
        } else {
            let mut b = (c + 1).min(chars.len());
            if word {
                while b < chars.len() && chars[b] != ' ' {
                    b += 1;
                }
            }
            (c, b)
        };
        if a < b {
            self.query = chars[..a].iter().chain(chars[b..].iter()).collect();
            self.caret = a;
        }
    }

    fn key(&mut self, vk: u16) -> bool {
        self.last_input = Instant::now();
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let alt = unsafe { GetKeyState(VK_MENU.0 as i32) } < 0;
        if VIRTUAL_KEY(vk) == VK_SHIFT {
            self.shift = true;
            self.invalidate();
            return false;
        }
        match VIRTUAL_KEY(vk) {
            VK_BACK if self.hex_focus => {
                self.hex_buf.pop();
            }
            VK_RETURN if self.hex_focus => {
                self.hex_focus = false;
                self.hex_apply();
            }
            VK_ESCAPE if self.hex_focus => self.hex_focus = false,
            VK_BACK if self.key_focus => {
                self.key_buf.pop();
            }
            VK_RETURN if self.key_focus => self.key_save(),
            VK_ESCAPE if self.key_focus => {
                self.key_focus = false;
                self.key_buf.clear();
            }
            VK_ESCAPE => {
                if self.menu.is_some() {
                    self.menu = None;
                } else if self.dialog.is_some() {
                    self.dialog = None;
                } else if self.flyout.is_some() {
                    self.flyout = None;
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.search_focus = false;
                    self.build_rows();
                } else if self.pane.is_some() {
                    self.pane = None;
                } else if self.page == 1 && self.st_pane.is_some() {
                    self.st_pane = None;
                } else {
                    self.sel = None;
                }
            }
            // dialog keys: Tab moves between its two buttons, Enter presses the focused one
            VK_TAB if self.dialog.is_some() => self.dlg_focus ^= 1,
            // Tab / Shift+Tab: the next / previous control, in the order they are drawn (with a focus ring)
            VK_TAB => self.tab_focus(shift),
            // Enter or Space presses the focused control (the list and the search box keep their own keys)
            VK_RETURN | VK_SPACE if self.focus.map(|f| !matches!(f, Hit::List | Hit::Search | Hit::KeyField | Hit::HexField)).unwrap_or(false) => {
                let f = self.focus.unwrap();
                if VIRTUAL_KEY(vk) == VK_SPACE {
                    self.eat_char = true;
                }
                if let Some(r) = self.hits.iter().rev().find(|(_, h)| *h == f).map(|(r, _)| *r) {
                    self.click(f, r.cx(), r.cy());
                }
            }
            VK_RETURN if self.dialog.is_some() => {
                if self.dlg_focus == 0 {
                    self.dialog_confirm();
                } else {
                    self.dialog = None;
                }
            }
            // each page moves its own selection: Startup apps its entries, Settings scrolls, Processes its rows
            VK_DOWN | VK_UP if self.page == 1 => self.move_startup(if VIRTUAL_KEY(vk) == VK_DOWN { 1 } else { -1 }),
            VK_DOWN | VK_UP if self.page == 2 => self.scroll_by(2, if VIRTUAL_KEY(vk) == VK_DOWN { 48.0 } else { -48.0 }),
            VK_DOWN | VK_UP if self.page == 3 => self.perf_move(if VIRTUAL_KEY(vk) == VK_DOWN { 1 } else { -1 }),
            VK_RETURN | VK_F1 if self.page != 0 && !alt => {}
            VK_DOWN => self.move_sel(1),
            VK_UP => self.move_sel(-1),
            VK_PRIOR => self.scroll_by(0, -(self.list_rect.h - self.row_h)),
            VK_NEXT => self.scroll_by(0, self.list_rect.h - self.row_h),
            // the search box has focus: editing keys go to its text, never to the list
            VK_LEFT if self.search_focus => {
                self.caret = self.qcaret().saturating_sub(1);
                self.q_all = false;
            }
            VK_RIGHT if self.search_focus => {
                self.caret = (self.qcaret() + 1).min(self.query.chars().count());
                self.q_all = false;
            }
            VK_HOME if self.search_focus => {
                self.caret = 0;
                self.q_all = false;
            }
            VK_END if self.search_focus => {
                self.caret = usize::MAX;
                self.q_all = false;
            }
            VK_A if ctrl && self.search_focus => self.q_all = !self.query.is_empty(),
            VK_DELETE if self.search_focus => {
                self.q_delete(false, ctrl);
                self.set_scroll(0, 0.0);
                self.build_rows();
            }
            VK_DELETE if self.key_focus || self.hex_focus => {}
            // Menu key / Shift+F10: the row menu for the selection, next to its row
            VK_APPS if self.page == 0 => self.menu_for_sel(),
            VK_F10 if shift && self.page == 0 => self.menu_for_sel(),
            VK_HOME if self.page == 0 => self.sel_edge(true),
            VK_END if self.page == 0 => self.sel_edge(false),
            VK_RIGHT if !self.search_focus => {
                if let Some(Sel::Group(k)) = self.sel.clone() {
                    self.expanded.insert(k);
                    self.build_rows();
                }
            }
            VK_LEFT if !self.search_focus => {
                if let Some(Sel::Group(k)) = self.sel.clone() {
                    self.expanded.remove(&k);
                    self.build_rows();
                }
            }
            VK_RETURN if alt => self.properties(),
            VK_RETURN | VK_F1 => self.open_details([true, false, false]),
            VK_DELETE if self.page == 0 => self.end_selected(shift),
            VK_F5 => self.ctl.now.store(true, Relaxed),
            VK_F if ctrl => self.search_focus = true,
            VK_OEM_COMMA if ctrl => self.set_page(2),
            VK_1 if ctrl => self.set_page(0),
            VK_2 if ctrl => self.set_page(3),
            VK_3 if ctrl => self.set_page(1),
            VK_4 if ctrl => self.set_page(2),
            VK_C if ctrl && shift => self.copy_command(),
            VK_BACK => {
                self.q_delete(true, ctrl);
                self.set_scroll(0, 0.0);
                self.build_rows();
            }
            _ => return false,
        }
        self.clamp_scroll();
        self.invalidate();
        true
    }

    fn char_in(&mut self, ch: char) {
        if ch.is_control() {
            if ch == '\u{16}' && self.key_focus {
                if let Some(s) = get_clipboard(self.hwnd) {
                    self.key_buf.push_str(s.trim());
                }
                self.invalidate();
            } else if ch == '\u{16}'
                && let Some(s) = get_clipboard(self.hwnd)
            {
                self.q_insert(s.lines().next().unwrap_or("").trim());
                self.search_focus = true;
                self.build_rows();
                self.invalidate();
            }
            return;
        }
        if self.dialog.is_some() {
            return;
        }
        if self.key_focus {
            self.key_buf.push(ch);
            self.invalidate();
            return;
        }
        if self.hex_focus {
            if ch.is_ascii_hexdigit() && self.hex_buf.trim_start_matches('#').len() < 6 {
                if !self.hex_buf.starts_with('#') {
                    self.hex_buf.insert(0, '#');
                }
                self.hex_buf.push(ch.to_ascii_uppercase());
                if self.hex_buf.len() == 7 {
                    self.hex_apply();
                }
            }
            self.invalidate();
            return;
        }
        // typing anywhere searches
        if !self.search_focus {
            self.caret = usize::MAX;
        }
        self.q_insert(&ch.to_string());
        self.search_focus = true;
        self.set_scroll(0, 0.0);
        // the Startup apps page filters its own list; other pages search processes
        if self.page != 1 {
            self.page = 0;
        }
        self.flyout = None;
        self.build_rows();
        self.invalidate();
    }

    // ------------------------------------------------------------------ IPC (`wtm gui ...`)
    fn state_json(&self) -> serde_json::Value {
        use serde_json::json;
        let sel = self.sel_group().map(|g| json!({"label": g.label, "exe": g.exe, "pids": g.pids(), "kind": g.type_label(), "mem": g.mem, "cpu": g.cpu, "hung": g.hung}));
        let visible: Vec<serde_json::Value> = self
            .rows
            .iter()
            .filter(|r| r.y + r.h > self.scroll[0] && r.y < self.scroll[0] + self.list_rect.h)
            .take(40)
            .map(|r| match r.k {
                RowK::Group(gi) => {
                    let g = &self.groups[gi];
                    json!({"row": "group", "label": g.label, "count": g.members.len(), "cpu": g.cpu, "mem": g.mem, "hung": g.hung})
                }
                RowK::Child(gi, mi) => {
                    let m = &self.groups[gi].members[mi];
                    json!({"row": "child", "pid": m.pid, "name": m.name, "sub": m.sub})
                }
                RowK::PinSep => json!({"row": "pinned-separator"}),
                RowK::Kind(k, n) => json!({"row": "section", "name": SECTION_NAMES[k as usize % 3], "count": n}),
                RowK::More(k, n) => json!({"row": if n > 0 { "show-all" } else { "show-fewer" }, "section": k, "count": n}),
                RowK::Sec(i, n) => {
                    let name = ["Processes", "Startup apps", "Services"][i as usize];
                    json!({"row": "section", "name": name, "count": n})
                }
                RowK::Startup(i) => json!({"row": "startup", "name": self.startup.get(i).map(|e| e.name.clone())}),
                RowK::Service(i) => json!({"row": "service", "name": self.services_all.get(i).map(|s| s.name.clone())}),
            })
            .collect();
        let chips: Vec<String> = self.chips().iter().map(|c| c.text.clone()).collect();
        let page_name = ["processes", "startup", "settings"][self.page as usize];
        let flyout = self.flyout.map(|f| ["cpu", "memory", "disk", "network", "gpu"][f.0 as usize]);
        json!({
            "ok": true,
            "page": page_name,
            "size": [self.w, self.h],
            "theme": if self.t.dark { "dark" } else { "light" },
            "mica": self.mica,
            "query": self.query,
            "sort": self.sort.label(),
            "selected": sel,
            "pane": self.pane.as_ref().map(|p| json!({"kind": if p.kind == PaneKind::Free { "free" } else { "details" }, "pid": p.pid, "open": p.open, "loading": p.loading, "verdict": p.info.answer.as_ref().map(|a| a.verdict.as_str()), "source": p.info.answer.as_ref().map(|a| a.source)})),
            "flyout": flyout,
            "menu": self.menu.as_ref().map(|m| format!("{:?}", m.kind)),
            "dialog": self.dialog.as_ref().map(|d| d.title.clone()),
            "toast": self.toast.as_ref().map(|t| format!("{} {}", t.text, t.code).trim().to_string()),
            "chips": chips,
            "groups": self.groups.len(),
            "processes": self.procs.len(),
            "rows": self.rows.len(),
            "visible_rows": visible,
            "startup_entries": self.startup.len(),
            "ai_enabled": self.ai_on,
            "lang": if self.lang_is_fa() { "fa" } else { "en" },
            "default_task_manager": if self.is_default { "win tools" } else { "windows" },
            "first_paint_ms": self.first_paint_ms,
            "full_data_ms": self.full_data_ms,
            "test_mode": self.test,
            "offscreen": self.offscreen,
            "focus": self.focus.map(|f| format!("{f:?}")),
            "active": self.is_active(),
            "caption": {"hover": format!("{:?}", self.nc_hover), "down": format!("{:?}", self.nc_down)},
            "sysmenu": SYSMENU_STATE.lock().unwrap().clone(),
            "prefs": self.prefs.to_json(),
            "backdrop_on": self.backdrop_on(),
            "row_h": self.row_h,
            "animating": Instant::now() < self.anim_until.get(),
            "last_action": self.last_action,
        })
    }

    fn ipc(&mut self, line: &str) -> String {
        use serde_json::json;
        let err = |e: &str| json!({"ok": false, "error": e}).to_string();
        let parts: Vec<&str> = line.split_whitespace().collect();
        let cmd = parts.first().copied().unwrap_or("");
        let arg = parts.get(1..).map(|a| a.join(" ")).unwrap_or_default();
        let a1 = parts.get(1).copied().unwrap_or("");
        unsafe {
            match cmd {
                "show" => {
                    if self.offscreen {
                        let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                    } else {
                        let _ = ShowWindow(self.hwnd, SW_RESTORE);
                        let _ = SetForegroundWindow(self.hwnd);
                    }
                }
                "shot-real" => {
                    if arg.is_empty() {
                        return err("shot-real needs a .bmp path");
                    }
                    // draw the latest frame first, then let DWM compose it
                    let _ = RedrawWindow(Some(self.hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW);
                    let _ = DwmFlush();
                    return match self.shot_real(&arg) {
                        Ok(()) => json!({"ok": true, "file": arg, "note": self.last_action}).to_string(),
                        Err(e) => err(&e),
                    };
                }
                "nc" => {
                    // caption buttons, the same code paths WM_NCMOUSEMOVE / WM_NCLBUTTONDOWN / WM_NCMOUSELEAVE use:
                    // nc hover|down|leave min|max|close  (no real mouse, no message to any other window)
                    let h = match parts.get(2).copied().unwrap_or("") {
                        "min" => Hit::Min,
                        "max" => Hit::Max,
                        "close" => Hit::Close,
                        _ => Hit::None,
                    };
                    match a1 {
                        "hover" => self.nc_move(h),
                        "down" => {
                            self.nc_move(h);
                            self.nc_down = h;
                        }
                        "leave" => {
                            self.nc_hover = Hit::None;
                            self.nc_down = Hit::None;
                        }
                        _ => return err("nc hover|down|leave min|max|close"),
                    }
                }
                "hit" => {
                    // what a click at client DIP x y would hit (from the last frame)
                    let x: f32 = a1.parse().unwrap_or(0.0);
                    let y: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    return json!({"ok": true, "hit": format!("{:?}", self.hit_at(x, y))}).to_string();
                }
                "perf" => {
                    // perf cpu|mem|disk|net|gpu [n]: show that resource on the Performance page (n-th of its kind)
                    let items = self.perf_items();
                    let n: usize = parts.get(2).and_then(|x| x.parse().ok()).unwrap_or(0);
                    let Some(it) = items.iter().filter(|i| i.key.starts_with(a1)).nth(n) else { return err("perf cpu|mem|disk|net|gpu [n]") };
                    self.perf_sel = it.key.clone();
                    self.set_page(3);
                    self.set_scroll(2, 0.0);
                }
                "order" => {
                    // the keys of all list rows, in order (to check that the list stays calm)
                    let keys: Vec<String> = self.rows.iter().filter_map(|r| match r.k {
                        RowK::Group(gi) => self.groups.get(gi).map(|g| g.key.clone()),
                        RowK::Child(gi, mi) => self.groups.get(gi).and_then(|g| g.members.get(mi)).map(|m| format!("  {}", m.pid)),
                        _ => None,
                    }).collect();
                    let all: Vec<String> = self.groups.iter().map(|g| g.key.clone()).collect();
                    let vals: serde_json::Map<String, serde_json::Value> = self.smooth.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                    return json!({"ok": true, "rows": keys, "groups": all, "smooth": vals}).to_string();
                }
                "hits" => {
                    // the clickable areas of the last frame whose name contains <filter>, with their centres
                    let list: Vec<_> = self
                        .hits
                        .iter()
                        .map(|(r, h)| (format!("{h:?}"), r))
                        .filter(|(n, _)| n.contains(a1))
                        .map(|(n, r)| json!({"hit": n, "x": r.x + r.w / 2.0, "y": r.y + r.h / 2.0, "w": r.w, "h": r.h}))
                        .collect();
                    return json!({"ok": true, "hits": list}).to_string();
                }
                "nchit" => {
                    // what WM_NCHITTEST answers at client DIP x y
                    let x: f32 = a1.parse().unwrap_or(0.0);
                    let y: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let code = self.nc_hit(x, y, IsZoomed(self.hwnd).as_bool());
                    let name = match code {
                        HTMINBUTTON => "HTMINBUTTON",
                        HTMAXBUTTON => "HTMAXBUTTON",
                        HTCLOSE => "HTCLOSE",
                        HTCAPTION => "HTCAPTION",
                        HTCLIENT => "HTCLIENT",
                        HTTOP => "HTTOP",
                        HTTOPRIGHT => "HTTOPRIGHT",
                        HTTOPLEFT => "HTTOPLEFT",
                        _ => "other",
                    };
                    return json!({"ok": true, "code": code, "name": name}).to_string();
                }
                "active" => {
                    self.active_force = match a1 {
                        "on" => Some(true),
                        "off" => Some(false),
                        _ => None,
                    }
                }
                "sysmenu" => {
                    if a1 == "close" {
                        let _ = EndMenu();
                    } else if self.offscreen {
                        return err("the system menu would open on a monitor; use the hidden desktop for it");
                    } else {
                        // the same message Alt+Space produces
                        let _ = PostMessageW(Some(self.hwnd), WM_SYSCOMMAND, WPARAM(SC_KEYMENU as usize), LPARAM(0x20));
                    }
                }
                "sysmenu-cmd" => {
                    let sc = match a1 {
                        "restore" => SC_RESTORE,
                        "min" | "minimize" => SC_MINIMIZE,
                        "max" | "maximize" => SC_MAXIMIZE,
                        "close" => SC_CLOSE,
                        _ => return err("sysmenu-cmd restore|min|max|close"),
                    };
                    let _ = PostMessageW(Some(self.hwnd), WM_SYSCOMMAND, WPARAM(sc as usize), LPARAM(0));
                }
                "section" => {
                    // section apps|background|windows open|close (Show all / Show fewer)
                    let Some(k) = ["apps", "background", "windows"].iter().position(|x| *x == a1) else { return err("section apps|background|windows open|close") };
                    let want = parts.get(2) == Some(&"open");
                    if self.prefs.sec_open[k] != want {
                        self.toggle_section(k as u8);
                    }
                }
                "mouse" => {
                    // real mouse messages to this window only: mouse <x> <y> [left|right|move|down|up] (DIPs)
                    let x: f32 = a1.parse().unwrap_or(0.0);
                    let y: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let sc = self.gfx.scale();
                    let lp = LPARAM((((y * sc) as i32 as u32 as isize) << 16) | ((x * sc) as i32 as u16 as isize));
                    let msgs: &[(u32, usize)] = match parts.get(3).copied().unwrap_or("left") {
                        "right" => &[(WM_MOUSEMOVE, 0), (WM_RBUTTONDOWN, 2), (WM_RBUTTONUP, 0)],
                        "move" => &[(WM_MOUSEMOVE, 0)],
                        "down" => &[(WM_MOUSEMOVE, 0), (WM_LBUTTONDOWN, 1)],
                        "up" => &[(WM_LBUTTONUP, 0)],
                        _ => &[(WM_MOUSEMOVE, 0), (WM_LBUTTONDOWN, 1), (WM_LBUTTONUP, 0)],
                    };
                    for (m, w) in msgs {
                        let _ = PostMessageW(Some(self.hwnd), *m, WPARAM(*w), lp);
                    }
                }
                "glyphs" => {
                    let (font, missing) = self.gfx.icon_check(&GLYPHS_USED);
                    return json!({"ok": true, "icon_font": font, "checked": GLYPHS_USED.len(), "missing": missing.iter().map(|c| format!("{c:X}")).collect::<Vec<_>>()}).to_string();
                }
                "quit" => {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                "state" => return self.state_json().to_string(),
                "shot" | "shot-alpha" => {
                    if arg.is_empty() {
                        return err("shot needs a .bmp path");
                    }
                    self.take_data();
                    return match self.shot(&arg, cmd == "shot-alpha") {
                        Ok(()) => json!({"ok": true, "file": arg}).to_string(),
                        Err(e) => err(&e),
                    };
                }
                "search" => {
                    self.query = arg.clone();
                    self.caret = usize::MAX;
                    self.search_focus = !arg.is_empty();
                    self.page = 0;
                    self.set_scroll(0, 0.0);
                    self.build_rows();
                }
                "select" => {
                    if let Err(e) = self.select_target(&arg) {
                        return err(&e);
                    }
                }
                "expand" | "collapse" => {
                    if !arg.is_empty()
                        && let Err(e) = self.select_target(&arg)
                    {
                        return err(&e);
                    }
                    let Some(g) = self.sel_group() else { return err("nothing selected") };
                    let k = g.key.clone();
                    if cmd == "expand" {
                        self.expanded.insert(k);
                    } else {
                        self.expanded.remove(&k);
                    }
                    self.build_rows();
                }
                "tab" => match a1 {
                    "processes" => self.set_page(0),
                    "startup" => self.set_page(1),
                    "performance" => self.set_page(3),
                    "settings" => self.set_page(2),
                    _ => return err("tab processes|startup|settings"),
                },
                "pane" => match a1 {
                    "whatis" | "details" => self.open_details([true, false, false]),
                    "why" => self.open_details([false, true, false]),
                    "free" => {
                        self.pane = None;
                        self.open_free();
                    }
                    "close" => self.pane = None,
                    _ => return err("pane whatis|why|free|close"),
                },
                "flyout" => {
                    let i = ["cpu", "memory", "disk", "network", "gpu"].iter().position(|x| *x == a1);
                    match (a1, i) {
                        ("close", _) => self.flyout = None,
                        (_, Some(i)) => self.flyout = Some((i as u8, Instant::now())),
                        _ => return err("flyout cpu|memory|disk|network|gpu|close"),
                    }
                }
                "menu" => {
                    let (x, y) = (parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(330.0), parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(322.0));
                    match a1 {
                        "row" => self.menu = Some(Menu { kind: MenuKind::Row, x, y, at: Instant::now(), anchor: None, by: Hit::None }),
                        "sort" => self.open_menu(Menu { kind: MenuKind::Sort, x: self.w - 190.0, y: TITLE_H + TILES_H + 70.0, at: Instant::now(), anchor: None, by: Hit::None }, Some(Hit::Sort)),
                        "close" => self.menu = None,
                        _ => return err("menu row [x y]|sort|close"),
                    }
                }
                "menu-click" => {
                    let Ok(i) = a1.parse::<u8>() else { return err("menu-click <index>") };
                    if self.menu.is_none() {
                        return err("no menu is open");
                    }
                    self.menu_action(i);
                }
                "hover" => {
                    // highlight what is under a point (for screenshots): hover X Y
                    let x: f32 = a1.parse().unwrap_or(0.0);
                    let y: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    self.hover = self.hit_at(x, y);
                    // a resting pointer there (shows its tooltip at once)
                    self.mouse = (x, y);
                    self.rest_at = Instant::now() - Duration::from_secs(1);
                }
                "click" => {
                    let x: f32 = a1.parse().unwrap_or(-1.0);
                    let y: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(-1.0);
                    self.mouse_down(x, y, false);
                    self.mouse_up(x, y);
                }
                "sort" => match SortKey::parse(a1) {
                    Some(s) => {
                        self.sort_rev = false;
                        self.set_sort(s)
                    }
                    None => return err("sort impact|cpu|memory|disk|network|gpu|name"),
                },
                "theme" => {
                    self.theme_override = match a1 {
                        "light" => Some(false),
                        "dark" => Some(true),
                        "system" => None,
                        _ => return err("theme light|dark|system"),
                    };
                    self.apply_theme();
                }
                "lang" => {
                    self.lang_fa = match a1 {
                        "fa" => Some(true),
                        "en" => Some(false),
                        _ => None,
                    }
                }
                "size" => {
                    let w: f32 = a1.parse().unwrap_or(1180.0);
                    let h: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(760.0);
                    let s = self.gfx.scale();
                    let _ = SetWindowPos(self.hwnd, None, 0, 0, (w * s) as i32, (h * s) as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
                "end" => {
                    if !arg.is_empty() && a1 != "process" {
                        if let Err(e) = self.select_target(&arg) {
                            return err(&e);
                        }
                    }
                    if self.sel_group().is_none() {
                        return err("nothing selected");
                    }
                    self.end_selected(a1 == "process");
                }
                "confirm" => {
                    if self.dialog.is_none() {
                        return err("no dialog is open");
                    }
                    if a1 == "end" {
                        self.dialog_confirm();
                    } else {
                        self.dialog = None;
                    }
                }
                "restart" => {
                    if !arg.is_empty()
                        && let Err(e) = self.select_target(&arg)
                    {
                        return err(&e);
                    }
                    self.restart_selected();
                }
                "copy-command" => self.copy_command(),
                "scroll" => {
                    // scroll <px> [pane]: absolute scroll of the current page (or the right pane)
                    let k = if parts.get(2) == Some(&"pane") { 3 } else { self.page as usize };
                    if parts.get(2) == Some(&"smooth") || parts.get(3) == Some(&"smooth") {
                        self.scroll_smooth(k, a1.parse().unwrap_or(0.0));
                    } else {
                        self.set_scroll(k, a1.parse().unwrap_or(0.0));
                    }
                }
                "toast" => {
                    if a1 == "close" {
                        self.toast = None;
                    } else if a1 == "restart" {
                        self.click(Hit::ToastLink, 0.0, 0.0);
                    } else {
                        self.toast_msg(&arg, true);
                    }
                }
                "free-end" => {
                    let i: usize = a1.parse().unwrap_or(usize::MAX);
                    self.free_end(i, parts.get(2) == Some(&"disable"));
                }
                "free-all" => self.free_all(),
                "stsec" => {
                    // stsec <0 apps|1 tasks|2 services> fold|open|all|top
                    let k = a1.parse::<usize>().unwrap_or(0).min(2);
                    match parts.get(2).copied().unwrap_or("") {
                        "fold" => self.set_pref(|p| p.st_fold[k] = true),
                        "open" => self.set_pref(|p| p.st_fold[k] = false),
                        "all" => self.set_pref(|p| p.st_all[k] = true),
                        "top" => self.set_pref(|p| p.st_all[k] = false),
                        _ => return err("stsec <0|1|2> fold|open|all|top"),
                    }
                }
                "stpane" => {
                    // stpane <name> | stpane close: the details pane of a startup entry
                    let name = parts.get(1..).map(|v| v.join(" ")).unwrap_or_default();
                    if name == "close" {
                        self.st_pane = None;
                    } else {
                        let Some(i) = self.startup.iter().position(|e| e.name.eq_ignore_ascii_case(&name) || e.name.rsplit('\\').next().is_some_and(|n| n.eq_ignore_ascii_case(&name))) else { return err("no startup entry with that name") };
                        self.sel = Some(Sel::Startup(i));
                        self.open_st_pane(i);
                    }
                }
                "startup" => {
                    // startup <name> on|off  (changes the real startup entry)
                    let name = parts.get(1..parts.len().saturating_sub(1)).map(|v| v.join(" ")).unwrap_or_default();
                    let on = parts.last() == Some(&"on");
                    let Some(i) = self.startup.iter().position(|e| e.name.eq_ignore_ascii_case(&name)) else { return err("no startup entry with that name") };
                    if self.startup[i].enabled != on {
                        self.toggle_startup(i);
                    }
                }
                "set" => {
                    let v = parts.get(2).copied().unwrap_or("");
                    let b = v == "on" || v == "true" || v == "1";
                    match a1 {
                        "merge" => self.set_pref(|p| p.merge = b),
                        "ask_end" => self.set_pref(|p| p.ask_end = b),
                        "show_pid" => self.set_pref(|p| p.show_pid = b),
                        "interval" => {
                            let ms = v.parse::<u64>().unwrap_or(1000);
                            self.set_pref(|p| p.interval_ms = ms)
                        }
                        "ai" => self.set_ai(b),
                        "theme" => {
                            let i = ["system", "light", "dark"].iter().position(|x| *x == v).unwrap_or(0) as u8;
                            self.set_pref(|p| p.theme = i)
                        }
                        "backdrop" => {
                            let Some(i) = ["mica", "mica-alt", "acrylic", "solid"].iter().position(|x| *x == v) else { return err("set backdrop mica|mica-alt|acrylic|solid") };
                            self.set_pref(|p| p.backdrop = i as u8)
                        }
                        "transparency" => {
                            let Some(i) = ["off", "low", "medium", "high"].iter().position(|x| *x == v) else { return err("set transparency off|low|medium|high") };
                            self.set_pref(|p| p.transp = i as u8)
                        }
                        "accent" if v == "desktop" => self.set_pref(|p| p.desktop = true),
                        "accent" => {
                            let acc = if v == "system" {
                                None
                            } else if let Some(i) = theme::PRESETS.iter().position(|p| p.0.eq_ignore_ascii_case(v)) {
                                Some(theme::PRESETS[i].1)
                            } else if let Some(rgb) = prefs::parse_hex(v) {
                                Some(rgb)
                            } else {
                                return err("set accent system|blue|purple|pink|red|orange|gold|green|teal|#RRGGBB");
                            };
                            self.custom_open = acc.is_some() && !theme::PRESETS.iter().any(|p| Some(p.1) == acc);
                            if let Some(a) = acc {
                                self.hex_buf = format!("#{a:06X}");
                            }
                            self.set_pref(|p| {
                                p.accent = acc;
                                p.desktop = false
                            })
                        }
                        "density" => self.set_pref(|p| p.density = (v != "compact") as u8),
                        "text" => {
                            let i = ["small", "medium", "large"].iter().position(|x| *x == v).unwrap_or(1) as u8;
                            self.set_pref(|p| p.font = i)
                        }
                        "corners" => self.set_pref(|p| p.corners = (v == "square") as u8),
                        "pane" => {
                            let i = ["narrow", "medium", "wide"].iter().position(|x| *x == v).unwrap_or(1) as u8;
                            self.set_pref(|p| p.pane = i)
                        }
                        "summary" => self.set_pref(|p| p.tiles = b),
                        "chips" => self.set_pref(|p| p.chips = b),
                        "nav" => self.set_pref(|p| p.nav_open = b),
                        "strength" => {
                            let Some(i) = ["subtle", "medium", "strong"].iter().position(|x| *x == v) else { return err("set strength subtle|medium|strong") };
                            self.set_pref(|p| p.strength = i as u8)
                        }
                        "group" => {
                            self.set_pref(|p| p.group = b);
                            self.build_rows();
                        }
                        "col" => {
                            let on = parts.get(3).map(|x| *x == "on").unwrap_or(true);
                            match v {
                                "type" => self.set_pref(|p| p.col_type = on),
                                "pid" => self.set_pref(|p| p.show_pid = on),
                                "disk" => self.set_pref(|p| p.col_disk = on),
                                "network" => self.set_pref(|p| p.col_net = on),
                                "gpu" => self.set_pref(|p| p.col_gpu = on),
                                _ => return err("set col type|pid|disk|network|gpu on|off"),
                            }
                        }
                        "sort" => {
                            if SortKey::parse(v).is_none() {
                                return err("set sort impact|cpu|memory|disk|network|gpu|name");
                            }
                            let name = v.to_lowercase();
                            self.set_pref(|p| p.sort = name)
                        }
                        _ => return err("set merge|ask_end|show_pid|ai|summary|chips on|off, set interval <ms>, set theme|backdrop|transparency|accent|density|text|corners|pane|sort <value>, set col <name> on|off"),
                    }
                }
                "ai" => match a1 {
                    // UI states only; the key itself never travels over this channel
                    "test" => self.key_test(),
                    "remove" => self.key_remove(),
                    "focus" => {
                        self.set_page(2);
                        self.key_focus = true;
                    }
                    "lang" => {
                        let l = parts.get(2).copied().unwrap_or("en");
                        if !self.test {
                            let _ = crate::ai::set_lang(l);
                        }
                        self.lang_fa = Some(l == "fa");
                    }
                    _ => return err("ai test|remove|focus|lang en|fa"),
                },
                "default" => match a1 {
                    "set" => self.set_default(true),
                    "restore" => self.set_default(false),
                    _ => return json!({"ok": true, "win_tools_is_default": prefs::is_default()}).to_string(),
                },
                "key" => {
                    let vk = match a1.to_lowercase().as_str() {
                        "esc" | "escape" => VK_ESCAPE,
                        "down" => VK_DOWN,
                        "up" => VK_UP,
                        "enter" => VK_RETURN,
                        "f1" => VK_F1,
                        "del" | "delete" => VK_DELETE,
                        "right" => VK_RIGHT,
                        "left" => VK_LEFT,
                        "home" => VK_HOME,
                        "end" => VK_END,
                        "back" | "backspace" => VK_BACK,
                        "apps" | "menu" => VK_APPS,
                        "tab" => VK_TAB,
                        "space" => VK_SPACE,
                        "ctrl+a" => {
                            self.q_all = self.search_focus && !self.query.is_empty();
                            return json!({"ok": true}).to_string();
                        }
                        _ => return err("key esc|down|up|enter|f1|del|right|left|home|end|back|apps|ctrl+a"),
                    };
                    self.key(vk.0);
                }
                "type" => {
                    for ch in arg.chars() {
                        self.char_in(ch);
                    }
                }
                _ => return err("unknown gui command: show|quit|state|shot|search|type|select|expand|collapse|tab|pane|flyout|menu|menu-click|hover|click|sort|theme|lang|size|end|confirm|restart|copy-command|toast|free-end|free-all|startup|set|ai|default|key"),
            }
        }
        self.invalidate();
        json!({"ok": true}).to_string()
    }
}

/// Facts for the details pane, read locally (no network).
fn pane_facts(p: &Proc, procs: &[Proc], startup: &[crate::startup::Entry], services: &[crate::services::Service], signed: Option<bool>) -> PaneInfo {
    let mut info = PaneInfo { signed, ..Default::default() };
    if let Some(par) = procs.iter().find(|q| q.pid == p.ppid && q.pid != p.pid && q.create_time <= p.create_time && q.pid != 0) {
        let label = if par.meta.description.is_empty() { par.name.clone() } else { par.meta.description.clone() };
        info.parent = Some((label, par.name.clone(), par.create_time));
    }
    let path = p.meta.path.to_lowercase();
    if !path.is_empty() {
        let file = path.rsplit('\\').next().unwrap_or("").to_string();
        for e in startup {
            let c = e.command.to_lowercase();
            if c.contains(&path) || (!file.is_empty() && e.source != "Service" && c.contains(&file) && c.ends_with(&file)) {
                let hit = StartupHit { name: e.name.clone(), source: source_label(e.source).into(), command: e.command.clone(), enabled: e.enabled };
                if e.source == "Scheduled task" {
                    info.tasks.push(e.name.clone());
                } else if e.source == "Service" {
                    info.services.push(e.name.clone());
                } else {
                    info.startup.push(hit);
                }
            }
        }
    }
    for s in services.iter().filter(|s| s.pid == p.pid && s.pid != 0) {
        if !info.services.contains(&s.display) {
            info.services.push(s.display.clone());
        }
    }
    let w = crate::insight::why(p, procs, services, startup, signed);
    info.flags = w.flags;
    info
}

/// "HKCU Run" -> "Registry · this user", ...
pub fn source_label(s: &str) -> &'static str {
    match s {
        "HKCU Run" => "Registry · this user",
        "HKLM Run" => "Registry · all users",
        "HKLM Run32" => "Registry · all users, 32-bit",
        "User folder" | "All users folder" => "Startup folder",
        "Scheduled task" => "Scheduled task",
        "Service" => "Service",
        _ => "Other",
    }
}

/// Command line + working folder to start a process again later.
fn relaunch_info(pid: u32) -> Option<(String, String)> {
    let cmd = procs::command_line(pid)?;
    let path = procs::image_path(pid)?;
    if path.to_lowercase().starts_with(r"c:\windows\") {
        return None;
    }
    let dir = crate::insight::current_dir(pid).unwrap_or_else(|| std::path::Path::new(&path).parent().map(|d| d.display().to_string()).unwrap_or_default());
    Some((cmd, dir))
}

fn spawn(cmd: &str, dir: &str) -> bool {
    use windows::Win32::System::Threading::*;
    let mut wcmd: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
    let wdir = wide(dir);
    let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut pi = PROCESS_INFORMATION::default();
    unsafe {
        let ok = CreateProcessW(None, Some(windows::core::PWSTR(wcmd.as_mut_ptr())), None, None, false, PROCESS_CREATION_FLAGS(0), None, if dir.is_empty() { PCWSTR::null() } else { PCWSTR(wdir.as_ptr()) }, &si, &mut pi).is_ok();
        if ok {
            let _ = CloseHandle(pi.hThread);
            let _ = CloseHandle(pi.hProcess);
        }
        ok
    }
}

/// In test mode only processes whose command line carries the marker "wt_test" may be ended.
fn test_owned(procs: &[Proc], pid: u32) -> bool {
    let _ = procs;
    procs::command_line(pid).map(|c| c.to_lowercase().contains("wt_test")).unwrap_or(false)
}

fn foreground_pid() -> u32 {
    unsafe {
        let h = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        pid
    }
}

fn set_clipboard(hwnd: HWND, s: &str) {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    let w = wide(s);
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            let _ = GlobalUnlock(h);
            let _ = SetClipboardData(13, Some(HANDLE(h.0)));
        }
        let _ = CloseClipboard();
    }
}

fn get_clipboard(hwnd: HWND) -> Option<String> {
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let r = GetClipboardData(13).ok().map(|h| {
            let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(HGLOBAL(h.0));
            s
        });
        let _ = CloseClipboard();
        r
    }
}

// ---------------------------------------------------------------------- window plumbing
pub fn trace(msg: &str) {
    if std::env::var("WT_TRACE").is_ok() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("wintask-trace.log")) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

fn lparam_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

/// Section names of the grouped list.
pub const SECTION_NAMES: [&str; 3] = ["Apps", "Background processes", "Windows processes"];

/// Caption buttons: Windows 11 size (46 x 32 DIPs) at the top right.
pub const CAP_W: f32 = 46.0;
pub const CAP_H: f32 = 32.0;

/// Glyphs the window draws from the icon font (checked by `wtm gui glyphs`).
pub const GLYPHS_USED: [u32; 27] = [
    0xE8A0, 0xE7E8,
    0xE921, 0xE922, 0xE923, 0xE8BB, 0xE713, 0xE721, 0xE711, 0xE72C, 0xE946, 0xE712, 0xEA99, 0xE8CB, 0xE70D, 0xE76C, 0xE790, 0xE771, 0xE81E, 0xE8D2, 0xE8A9, 0xE739, 0xE89F, 0xE895, 0xE9D2, 0xE70F, 0xE8FD,
];

/// Last window menu shown: its items and whether it is open (for `wtm gui state`).
static SYSMENU_STATE: Mutex<serde_json::Value> = Mutex::new(serde_json::Value::Null);

impl App {
    pub fn is_active(&self) -> bool {
        self.active_force.unwrap_or(self.active)
    }

    /// What WM_NCHITTEST answers at client point (x, y) in DIPs.
    pub fn nc_hit(&self, x: f32, y: f32, maxed: bool) -> u32 {
        let caps = x >= self.w - CAP_W * 3.0 && y < CAP_H;
        if !maxed {
            // resize edges; over the caption buttons only a thin top band resizes, the corner still does
            let b = 6.0;
            let top = if caps && x < self.w - b { 2.0 } else { b };
            let (l, r, t, bt) = (x < b, x > self.w - b, y < top, y > self.h - b);
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
                return code;
            }
        }
        if caps {
            // answered even under a dialog, so Snap Layouts and the buttons always work
            let i = ((x - (self.w - CAP_W * 3.0)) / CAP_W) as usize;
            return [HTMINBUTTON, HTMAXBUTTON, HTCLOSE][i.min(2)];
        }
        if y < TITLE_H && self.dialog.is_none() {
            return match self.hit_at(x, y) {
                Hit::None => HTCAPTION,
                _ => HTCLIENT,
            };
        }
        HTCLIENT
    }

    /// The pointer moved onto caption button `h` (or off them, Hit::None).
    pub fn nc_move(&mut self, h: Hit) {
        if h != self.nc_hover {
            self.nc_hover = h;
            if h != Hit::None {
                self.hover = Hit::None;
            }
            if self.nc_down != h {
                self.nc_down = Hit::None;
            }
            self.invalidate();
        }
    }

    /// Minimize / maximize / restore from a caption button (blocked for the off-screen test window).
    fn caption_cmd(&mut self, sc: u32) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_SYSCOMMAND, WPARAM(sc as usize), LPARAM(0));
        }
    }
}

/// Where Alt+Space opens the window menu: the window's top-left, under the title bar.
fn sysmenu_origin(hwnd: HWND, s: f32) -> (i32, i32) {
    let mut wr = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut wr);
    }
    (wr.left + (8.0 * s) as i32, wr.top + (TITLE_H * s) as i32)
}

/// The window menu (Restore, Move, Size, Minimize, Maximize, Close). The window has no WS_SYSMENU (so DWM
/// draws no caption buttons over Mica), so it is shown here and its choice is sent as WM_SYSCOMMAND.
fn show_sysmenu(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let mut m = GetSystemMenu(hwnd, false);
        let own = m.is_invalid() || GetMenuItemCount(Some(m)) <= 0;
        if own {
            m = CreatePopupMenu().unwrap_or_default();
            for (id, label) in [(SC_RESTORE, "&Restore"), (SC_MOVE, "&Move"), (SC_SIZE, "&Size"), (SC_MINIMIZE, "Mi&nimize"), (SC_MAXIMIZE, "Ma&ximize"), (0, ""), (SC_CLOSE, "&Close	Alt+F4")] {
                if id == 0 {
                    let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
                } else {
                    let w = wide(label);
                    let _ = AppendMenuW(m, MF_STRING, id as usize, PCWSTR(w.as_ptr()));
                }
            }
        }
        let maxed = IsZoomed(hwnd).as_bool();
        let iconic = IsIconic(hwnd).as_bool();
        let st = |on: bool| MF_BYCOMMAND | if on { MF_ENABLED } else { MF_GRAYED };
        let items = [(SC_RESTORE, maxed || iconic, "restore"), (SC_MOVE, !maxed && !iconic, "move"), (SC_SIZE, !maxed && !iconic, "size"), (SC_MINIMIZE, !iconic, "minimize"), (SC_MAXIMIZE, !maxed, "maximize"), (SC_CLOSE, true, "close")];
        for (id, on, _) in items {
            let _ = EnableMenuItem(m, id, st(on));
        }
        let _ = SetMenuDefaultItem(m, SC_CLOSE, 0);
        let list: serde_json::Map<String, serde_json::Value> = items.iter().map(|(_, on, n)| (n.to_string(), serde_json::Value::Bool(*on))).collect();
        *SYSMENU_STATE.lock().unwrap() = serde_json::json!({"open": true, "own_menu": own, "enabled": list});
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, x, y, None, hwnd, None);
        SYSMENU_STATE.lock().unwrap()["open"] = serde_json::Value::Bool(false);
        if own {
            let _ = DestroyMenu(m);
        }
        if cmd.0 != 0 {
            let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd.0 as usize), LPARAM(0));
        }
    }
}

/// DWMWA_SYSTEMBACKDROP_TYPE: 1 none, 2 Mica, 3 Acrylic, 4 Mica Alt.
fn set_backdrop(hwnd: HWND, kind: i32) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const i32 as *const _, 4);
    }
}

/// DWMWA_WINDOW_CORNER_PREFERENCE: 2 round, 1 do not round.
fn set_corners(hwnd: HWND, round: bool) {
    let v: i32 = if round { 2 } else { 1 };
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &v as *const i32 as *const _, 4);
    }
}

/// Save 32-bit top-down BGRA pixels as a BMP file.
fn write_bmp(path: &str, w: i32, h: i32, px: &[u8]) -> Result<(), String> {
    let n = px.len();
    let mut f = Vec::with_capacity(54 + n);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&((54 + n) as u32).to_le_bytes());
    f.extend_from_slice(&[0, 0, 0, 0]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&(-h).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    f.extend_from_slice(px);
    std::fs::write(path, f).map_err(|e| e.to_string())
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
        let s = app.gfx.scale();
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
                LRESULT(app.nc_hit(x, y, IsZoomed(hwnd).as_bool()) as isize)
            }
            WM_NCMOUSEMOVE => {
                let h = match wp.0 as u32 {
                    HTMINBUTTON => Hit::Min,
                    HTMAXBUTTON => Hit::Max,
                    HTCLOSE => Hit::Close,
                    _ => Hit::None,
                };
                if h != app.nc_hover {
                    app.nc_move(h);
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                }
                // DefWindowProc keeps the Windows 11 Snap Layouts flyout working over HTMAXBUTTON
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCMOUSELEAVE => {
                app.nc_hover = Hit::None;
                app.nc_down = Hit::None;
                app.invalidate();
                LRESULT(0)
            }
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => match wp.0 as u32 {
                // caption buttons are pressed and released here, so Windows never draws its own over Mica
                HTMINBUTTON | HTMAXBUTTON | HTCLOSE => {
                    app.nc_down = app.nc_hover;
                    app.animate(motion::FAST as u64);
                    app.invalidate();
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            },
            WM_NCLBUTTONUP => {
                let d = std::mem::replace(&mut app.nc_down, Hit::None);
                app.invalidate();
                match (wp.0 as u32, d) {
                    (HTMINBUTTON, Hit::Min) => app.caption_cmd(SC_MINIMIZE),
                    (HTMAXBUTTON, Hit::Max) => app.caption_cmd(if IsZoomed(hwnd).as_bool() { SC_RESTORE } else { SC_MAXIMIZE }),
                    (HTCLOSE, Hit::Close) => {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    _ => return DefWindowProcW(hwnd, msg, wp, lp),
                }
                LRESULT(0)
            }
            WM_NCRBUTTONUP if wp.0 as u32 == HTCAPTION => {
                // right-click on the title bar: the window menu
                let (sx, sy) = lparam_xy(lp);
                if !app.offscreen {
                    show_sysmenu(hwnd, sx, sy);
                }
                LRESULT(0)
            }
            WM_SYSCHAR if wp.0 == 0x20 => {
                // Alt+Space: the window menu at the top-left, under the title bar
                if !app.offscreen {
                    let (x, y) = sysmenu_origin(hwnd, s);
                    show_sysmenu(hwnd, x, y);
                }
                LRESULT(0)
            }
            WM_SYSCOMMAND => {
                let sc = (wp.0 & 0xFFF0) as u32;
                if sc == SC_KEYMENU && lp.0 == 0x20 {
                    if !app.offscreen {
                        let (x, y) = sysmenu_origin(hwnd, s);
                        show_sysmenu(hwnd, x, y);
                    }
                    return LRESULT(0);
                }
                if sc == SC_CLOSE {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    return LRESULT(0);
                }
                if app.offscreen && [SC_MINIMIZE, SC_MAXIMIZE, SC_RESTORE, SC_MOVE, SC_SIZE].contains(&sc) {
                    // the test window never moves onto a monitor
                    app.last_action = format!("window command {sc:#X} blocked (offscreen test window)");
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCACTIVATE => {
                app.active = wp.0 != 0;
                app.invalidate();
                // with a custom frame DWM draws nothing here; -1 keeps DefWindowProc from repainting a frame
                DefWindowProcW(hwnd, msg, wp, LPARAM(-1))
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_xy(lp);
                let (x, y) = (x as f32 / s, y as f32 / s);
                // tooltips: a move hides the tip, a rest of 500 ms shows it
                if (x - app.mouse.0).abs() + (y - app.mouse.1).abs() > 2.0 {
                    let was = app.rest_at.elapsed() >= Duration::from_millis(500) && app.tips.iter().any(|(r, _)| r.has(app.mouse.0, app.mouse.1));
                    app.mouse = (x, y);
                    app.rest_at = Instant::now();
                    SetTimer(Some(hwnd), TIMER_TIP, 520, None);
                    if was {
                        app.invalidate();
                    }
                }
                if let Some((k, y0, s0)) = app.drag_scroll {
                    let (view, ch) = (app.view_h[k as usize], app.content_h[k as usize]);
                    let th = (view * view / ch.max(1.0)).max(30.0);
                    app.set_scroll(k as usize, s0 + (y - y0) * (ch - view) / (view - th).max(1.0));
                    app.invalidate();
                    return LRESULT(0);
                }
                app.nc_hover = Hit::None;
                app.nc_down = Hit::None;
                if app.pressed == Hit::Hue {
                    app.hue_at(x);
                    app.invalidate();
                    return LRESULT(0);
                }
                app.pointer_in_list = app.page == 0 && app.list_rect.has(x, y);
                let h = app.hit_at(x, y);
                if h != app.hover {
                    app.hover = h;
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                    app.invalidate();
                }
                LRESULT(0)
            }
            WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
                // I-beam over text fields, a hand over links, the arrow elsewhere
                let h = app.hit_at(app.mouse.0, app.mouse.1);
                let more = matches!(h, Hit::Row(i) if matches!(app.rows.get(i).map(|r| r.k), Some(RowK::More(..))));
                let id = match h {
                    Hit::Search | Hit::KeyField | Hit::HexField => IDC_IBEAM,
                    Hit::KeyLink | Hit::ToastLink => IDC_HAND,
                    _ if more => IDC_HAND,
                    _ => IDC_ARROW,
                };
                SetCursor(LoadCursorW(None, id).ok());
                LRESULT(1)
            }
            WM_MOUSELEAVE => {
                app.mouse = (-1.0, -1.0);
                app.hover = Hit::None;
                app.pointer_in_list = false;
                app.invalidate();
                LRESULT(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_xy(lp);
                app.shift = (wp.0 as u32) & 0x0004 != 0;
                app.mouse_down(x as f32 / s, y as f32 / s, msg == WM_LBUTTONDBLCLK);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_xy(lp);
                app.mouse_up(x as f32 / s, y as f32 / s);
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let (x, y) = lparam_xy(lp);
                let (x, y) = (x as f32 / s, y as f32 / s);
                app.menu = None;
                app.last_input = Instant::now();
                if let Hit::Row(i) | Hit::Chevron(i) = app.hit_at(x, y)
                    && let Some(i) = app.row_now(i)
                    && matches!(app.rows.get(i).map(|r| r.k), Some(RowK::Group(_)) | Some(RowK::Child(..)))
                {
                    app.select_row(i);
                    app.menu = Some(Menu { kind: MenuKind::Row, x, y, at: Instant::now(), anchor: None, by: Hit::None });
                    app.animate(180);
                }
                app.invalidate();
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
                let (sx, sy) = lparam_xy(lp);
                let mut pt = POINT { x: sx, y: sy };
                let _ = ScreenToClient(hwnd, &mut pt);
                app.wheel(pt.x as f32 / s, pt.y as f32 / s, d);
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if app.key(wp.0 as u16) {
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_KEYUP => {
                if wp.0 as u16 == VK_SHIFT.0 {
                    app.shift = false;
                    app.invalidate();
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CHAR => {
                // UTF-16 surrogate pairs come as two messages: keep the high half until the low one arrives
                let u = wp.0 as u32;
                if std::mem::take(&mut app.eat_char) && u == 0x20 {
                    return LRESULT(0);
                }
                if (0xD800..0xDC00).contains(&u) {
                    app.hi_surrogate = u as u16;
                } else if (0xDC00..0xE000).contains(&u) {
                    if let Some(Ok(c)) = char::decode_utf16([app.hi_surrogate, u as u16]).next() {
                        app.char_in(c);
                    }
                    app.hi_surrogate = 0;
                } else if let Some(c) = char::from_u32(u) {
                    app.char_in(c);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                app.paint();
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                if wp.0 as u32 == SIZE_MINIMIZED {
                    trim_ws();
                }
                app.invalidate();
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == TIMER_ANIM {
                    let toast_live = app.toast.as_ref().map(|t| t.at.elapsed().as_secs() < t.secs || app.hover == Hit::ToastLink || app.hover == Hit::ToastClose).unwrap_or(false);
                    if app.toast.is_some() && !toast_live {
                        app.toast = None;
                    }
                    if Instant::now() > app.anim_until.get() && app.toast.is_none() {
                        let _ = KillTimer(Some(hwnd), TIMER_ANIM);
                    }
                    if app.toast.is_some() && Instant::now() > app.anim_until.get() {
                        // keep a slow tick for the toast's auto-hide
                        SetTimer(Some(hwnd), TIMER_ANIM, 250, None);
                    }
                    app.invalidate();
                } else if wp.0 == TIMER_TIP {
                    let _ = KillTimer(Some(hwnd), TIMER_TIP);
                    if app.tips.iter().any(|(r, _)| r.has(app.mouse.0, app.mouse.1)) {
                        app.invalidate();
                    }
                } else if wp.0 == TIMER_TRIM {
                    let _ = KillTimer(Some(hwnd), TIMER_TRIM);
                    trim_ws();
                } else if wp.0 == TIMER_TICK {
                    let _ = KillTimer(Some(hwnd), TIMER_TICK);
                    app.is_default = prefs::is_default();
                    app.invalidate();
                }
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let mi = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                mi.ptMinTrackSize = POINT { x: (MIN_W * dpi) as i32, y: (MIN_H * dpi) as i32 };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                app.ctl.icon_px.store((16.0 * GetDpiForWindow(hwnd) as f32 / 96.0).round() as u32, Relaxed);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                // theme or accent changed
                app.apply_theme();
                LRESULT(0)
            }
            WM_DWMCOLORIZATIONCOLORCHANGED => {
                app.apply_theme();
                LRESULT(0)
            }
            WM_ACTIVATE => {
                app.active = (wp.0 & 0xFFFF) != 0;
                if (wp.0 & 0xFFFF) == 0 {
                    app.menu = None;
                    app.shift = false;
                    app.invalidate();
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            m if m == WM_DATA => {
                app.take_data();
                app.invalidate();
                LRESULT(0)
            }
            m if m == WM_PANEL => {
                app.take_panel();
                LRESULT(0)
            }
            m if m == WM_KEYTEST => {
                app.key_testing = false;
                app.key_result = app.key_out.lock().unwrap().take();
                app.invalidate();
                LRESULT(0)
            }
            m if m == WM_IPC => {
                let jobs: Vec<(String, std::sync::mpsc::Sender<String>)> = app.ipc_in.lock().unwrap().drain(..).collect();
                for (line, tx) in jobs {
                    let r = app.ipc(&line);
                    let _ = tx.send(r);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                if !app.test {
                    save_placement(hwnd);
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Smallest window (DIPs).
const MIN_W: f32 = 600.0;
const MIN_H: f32 = 480.0;

fn placement_path() -> std::path::PathBuf {
    crate::ai::config_dir().join("taskman-window.json")
}

/// Remember the normal (restored) rectangle and whether the window is maximized.
fn save_placement(hwnd: HWND) {
    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if unsafe { GetWindowPlacement(hwnd, &mut wp) }.is_err() {
        return;
    }
    let r = wp.rcNormalPosition;
    let v = serde_json::json!({"left": r.left, "top": r.top, "right": r.right, "bottom": r.bottom, "max": wp.showCmd == SW_SHOWMAXIMIZED.0 as u32});
    let _ = std::fs::create_dir_all(crate::ai::config_dir());
    let _ = std::fs::write(placement_path(), v.to_string());
}

/// The saved rectangle, if it is still on a monitor and not smaller than the minimum.
fn load_placement() -> Option<(RECT, bool)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(placement_path()).ok()?).ok()?;
    let g = |k: &str| v[k].as_i64().map(|x| x as i32);
    let r = RECT { left: g("left")?, top: g("top")?, right: g("right")?, bottom: g("bottom")? };
    if r.right - r.left < 300 || r.bottom - r.top < 240 {
        return None;
    }
    // a monitor must show a good part of its title bar
    let title = RECT { left: r.left + 40, top: r.top, right: r.right - 40, bottom: r.top + 30 };
    if unsafe { MonitorFromRect(&title, MONITOR_DEFAULTTONULL) }.is_invalid() {
        return None;
    }
    Some((r, v["max"].as_bool().unwrap_or(false)))
}

/// Return pages that are not in use to Windows (start-up only pages, or everything while minimized).
fn trim_ws() {
    unsafe {
        let _ = windows::Win32::System::ProcessStatus::EmptyWorkingSet(windows::Win32::System::Threading::GetCurrentProcess());
    }
}

fn set_dark_frame(hwnd: HWND, dark: bool) {
    let v: i32 = dark as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v as *const i32 as *const _, 4);
    }
}

/// Mica backdrop (Windows 11 22H2+). Returns false when DWM refuses it (solid fallback).
fn set_mica(hwnd: HWND) -> bool {
    let v: i32 = 2; // DWMSBT_MAINWINDOW
    unsafe {
        if DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &v as *const i32 as *const _, 4).is_err() {
            return false;
        }
        // the whole client area is "frame", so transparent pixels show the backdrop
        DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }).is_ok()
    }
}

/// Create the window and run the message loop. Returns the exit code.
/// Args: `--software` (CPU renderer), `--theme light|dark`, `--no-mica`, `--test` (no clipboard, no launching
/// other programs, ends only processes marked wt_test).
pub fn run(software: bool) -> i32 {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let arg_after = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    // --offscreen: a test window far off every monitor, no taskbar button, never activated (implies --test)
    let offscreen = args.iter().any(|a| a == "--offscreen");
    let test = offscreen || args.iter().any(|a| a == "--test");
    let theme_override = match arg_after("--theme").as_deref() {
        Some("light") => Some(false),
        Some("dark") => Some(true),
        _ => None,
    };
    let want_mica = !args.iter().any(|a| a == "--no-mica") && std::env::var("WT_NOMICA").is_err();
    // test windows start from the defaults (reproducible tests; the user's saved settings are never read or written)
    let prefs = if test { Prefs::default() } else { Prefs::load() };
    let prefs_nav_open = prefs.nav_open;
    let (shared, ctl) = data::start();
    ctl.interval_ms.store(prefs.interval_ms, Relaxed);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let cls = w!("WinToolsTaskManager");
        let dark0 = match (theme_override, prefs.theme) {
            (Some(d), _) => d,
            (None, 1) => false,
            (None, 2) => true,
            _ => theme::system_dark(),
        };
        let t = theme::make(dark0, prefs.accent, prefs.transp);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(inst.into()), PCWSTR(1 as _)).unwrap_or_default(),
            // the exe's icon resource at small size too (title bar, Alt+Tab small, taskbar on small icons)
            hIconSm: LoadImageW(Some(inst.into()), PCWSTR(1 as _), IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR).map(|h| HICON(h.0)).unwrap_or_default(),
            lpszClassName: cls,
            hbrBackground: CreateSolidBrush(COLORREF(((t.mica.2 * 255.0) as u32) << 16 | ((t.mica.1 * 255.0) as u32) << 8 | (t.mica.0 * 255.0) as u32)),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // 1180x760 DIPs on the primary monitor, centred. No WS_SYSMENU: DWM must not draw its own caption
        // buttons over the Mica frame (the window draws Fluent ones and answers HTMINBUTTON/HTMAXBUTTON/HTCLOSE).
        // the last size and place (not for test windows), else 1180x760 fitted into the work area and centred
        let dpi = GetDpiForSystem() as f32 / 96.0;
        let mut work = RECT::default();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), Default::default()).is_err() {
            work = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        }
        let (ww, wh) = (work.right - work.left, work.bottom - work.top);
        let (w, h) = (((1180.0 * dpi) as i32).min(ww - 16), ((760.0 * dpi) as i32).min(wh - 16));
        let saved = if test { None } else { load_placement() };
        let (sx, sy, w, h) = match saved {
            Some((r, _)) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
            None => (work.left + (ww - w) / 2, work.top + (wh - h) / 2, w, h),
        };
        let style = if want_mica { WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_CLIPCHILDREN } else { WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN };
        let (ex, x0, y0) = if offscreen { (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, -20000, -20000) } else { (WINDOW_EX_STYLE(0), sx, sy) };
        let Ok(hwnd) = CreateWindowExW(ex, cls, w!("Rust Task Manager"), style, x0, y0, w, h, None, None, Some(inst.into()), None) else { return 1 };
        set_dark_frame(hwnd, t.dark);
        let mica = want_mica && set_mica(hwnd);
        if !mica {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });
        }
        trace(&format!("{:.1} ms: hwnd (mica {mica})", started.elapsed().as_secs_f64() * 1000.0));
        let Ok(mut gfx) = Gfx::new(hwnd, software) else { return 1 };
        gfx.alpha = mica;
        trace(&format!("{:.1} ms: gfx", started.elapsed().as_secs_f64() * 1000.0));
        let app = Box::new(App {
            hwnd,
            gfx,
            t,
            prefs,
            theme_override,
            mica,
            shared,
            ctl: ctl.clone(),
            data_gen: 0,
            startup_gen: 0,
            procs: Arc::new(vec![]),
            perf: Default::default(),
            services: HashMap::new(),
            services_all: Arc::new(vec![]),
            startup: Arc::new(vec![]),
            cmdlines: HashMap::new(),
            hung: vec![],
            hist_anoms: vec![],
            gpu_names: vec![],
            icon_px: HashMap::new(),
            icons: HashMap::new(),
            icon_target: 0,
            icon_asked: HashSet::new(),
            groups: vec![],
            order_prev: HashMap::new(),
            last_order: None,
            hist: HashMap::new(),
            child_hist: HashMap::new(),
            tot: Default::default(),
            rows: vec![],
            sel: None,
            expanded: HashSet::new(),
            sort: SortKey::Impact,
            startup_sort: 0,
            startup_hold: None,
            st_pane: None,
            menu_r: None,
            query: String::new(),
            search_focus: false,
            caret: usize::MAX,
            hi_surrogate: 0,
            dlg_focus: 0,
            sort_rev: false,
            perf_hist: HashMap::new(),
            perf_sel: "cpu".into(),
            perf_top_keys: vec![],
            nav_peek: false,
            focus: None,
            eat_char: false,
            smooth: HashMap::new(),
            smooth_sort: SortKey::Impact,
            elevated: crate::sys::is_elevated(),
            q_all: false,
            scroll: [0.0; 4],
            drag_scroll: None,
            hits: vec![],
            tips: vec![],
            mouse: (-1.0, -1.0),
            rest_at: Instant::now(),
            hover: Hit::None,
            pressed: Hit::None,
            nc_hover: Hit::None,
            nc_down: Hit::None,
            page: 0,
            pane: None,
            pane_result: Arc::new(Mutex::new(None)),
            flyout: None,
            menu: None,
            dialog: None,
            toast: None,
            mo: std::cell::RefCell::new(Mo::default()),
            anim_until: std::cell::Cell::new(Instant::now()),
            offscreen,
            active: false,
            active_force: None,
            hex_focus: false,
            hex_buf: String::new(),
            hue_rect: Rect::default(),
            custom_open: false,
            w: 1180.0,
            h: 760.0,
            ipc_in: Arc::new(Mutex::new(vec![])),
            first_paint_ms: None,
            full_data_ms: None,
            started,
            test,
            last_action: String::new(),
            ai_on: false,
            lang_fa: None,
            key_buf: String::new(),
            key_focus: false,
            key_saved: false,
            key_testing: false,
            key_result: None,
            key_out: Arc::new(Mutex::new(None)),
            shift: false,
            is_default: false,
            shooting: false,
            list_rect: Rect::default(),
            pane_rect: Rect::default(),
            content_h: [0.0; 4],
            view_h: [600.0; 4],
            want_scroll_sel: false,
            pointer_in_list: false,
            last_input: Instant::now(),
            painted: vec![],
            nav_tip: None,
            nav_w: if prefs_nav_open { NAV_W_OPEN } else { NAV_W },
            row_h: ROW_H,
        });
        ctl.icon_px.store((16.0 * GetDpiForWindow(hwnd) as f32 / 96.0).round() as u32, Relaxed);
        let ipc_in = app.ipc_in.clone();
        let ptr = Box::into_raw(app);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        ctl.hwnd.store(hwnd.0 as isize, Relaxed);
        // local control pipe for `wtm gui ...`
        let hw = hwnd.0 as isize;
        std::thread::spawn(move || {
            crate::ipc::serve(|line| {
                let (tx, rx) = std::sync::mpsc::channel();
                ipc_in.lock().unwrap().push((line, tx));
                let _ = PostMessageW(Some(HWND(hw as *mut _)), WM_IPC, WPARAM(0), LPARAM(0));
                rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| r#"{"ok":false,"error":"window busy"}"#.into())
            })
        });
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        // saved look: backdrop, corners, text size, density, accent; and the default sort
        if let Some(sk) = SortKey::parse(&(*ptr).prefs.sort) {
            (*ptr).sort = sk;
        }
        (*ptr).apply_look();
        // data may already be there (the first sample takes a few ms)
        (*ptr).take_data();
        trace(&format!("{:.1} ms: data taken ({} procs)", started.elapsed().as_secs_f64() * 1000.0, (*ptr).procs.len()));
        let maxed = saved.map(|p| p.1).unwrap_or(false);
        let _ = ShowWindow(hwnd, if offscreen { SW_SHOWNOACTIVATE } else if maxed { SW_SHOWMAXIMIZED } else { SW_SHOW });
        trace(&format!("{:.1} ms: shown", started.elapsed().as_secs_f64() * 1000.0));
        let _ = UpdateWindow(hwnd);
        trace(&format!("{:.1} ms: first paint done", started.elapsed().as_secs_f64() * 1000.0));
        // slow facts after the first frame
        (*ptr).ai_on = crate::ai::enabled();
        (*ptr).key_saved = crate::ai::has_key();
        (*ptr).is_default = prefs::is_default();
        // after start-up settles, give back pages that were only needed to start (fonts, first samples)
        SetTimer(Some(hwnd), TIMER_TRIM, 4000, None);
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

/// Is this mouse-down on the button of the dropdown that is open (so it must only close it)? `by` = the hit that
/// opened it, `anchor` = its rectangle, `hit_rect` = where the clicked hit is drawn. (Copied from File Explorer.)
pub fn drop_toggle_click(h: Hit, by: Option<Hit>, anchor: Option<Rect>, hit_rect: Option<Rect>) -> bool {
    if matches!(h, Hit::None | Hit::Block) {
        return false;
    }
    if by == Some(h) {
        return true;
    }
    match (anchor, hit_rect) {
        (Some(a), Some(r)) => (a.x - r.x).abs() < 1.0 && (a.y - r.y).abs() < 1.0 && (a.w - r.w).abs() < 1.0,
        _ => false,
    }
}
