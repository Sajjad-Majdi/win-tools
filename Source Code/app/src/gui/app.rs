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
pub const FREE_W: f32 = 432.0;
const WM_PANEL: u32 = WM_DATA + 1;
const WM_IPC: u32 = WM_DATA + 2;
const WM_KEYTEST: u32 = WM_DATA + 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const TIMER_ANIM: usize = 1;
const TIMER_TICK: usize = 2;
const TIMER_TRIM: usize = 3;

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
    SetDefault,
    RestoreDefault,
    Seg(u8, u8),
    Toggle(u8),
    CliCopy(u8),
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
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RowK {
    Group(usize),
    Child(usize, usize),
    PinSep,
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
    pub query: String,
    pub search_focus: bool,
    /// list, startup page, settings page, pane
    pub scroll: [f32; 4],
    drag_scroll: Option<(u8, f32, f32)>,
    pub hits: Vec<(Rect, Hit)>,
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
    pub is_default: bool,
    /// set while painting a screenshot: animations show their end state
    pub shooting: bool,
    pub list_rect: Rect,
    pub pane_rect: Rect,
    pub content_h: [f32; 4],
    pub view_h: [f32; 4],
    /// bring the selected row into view on the next paint (after the layout is known)
    pub want_scroll_sel: bool,
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
        // a row that changes place dims while it passes other rows, so crossing rows do not read as a jumble
        let (dy, dim) = match mo.row_y.get(&id) {
            Some(tw) if !tw.done(now) => {
                self.animate(30);
                let far = ((tw.to - tw.from).abs() / self.row_h).min(3.0) / 3.0;
                (tw.value(now, motion::standard), 1.0 - 0.65 * far.max(0.4) * (tw.progress(now) * std::f32::consts::PI).sin())
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
        let transp = if self.backdrop_on() || self.gfx.offscreen() { self.prefs.transp } else { self.prefs.transp.min(1) };
        theme::make(dark, self.prefs.accent, transp)
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
        model::sort_groups(&mut v, self.sort);
        if self.sort == SortKey::Impact {
            // move rows at most every 2 s, and only when they changed rank by 2+ places
            let fresh = self.last_order.map(|t| t.elapsed() >= Duration::from_secs(2)).unwrap_or(true);
            if fresh {
                model::stable_order(&mut v, &self.order_prev);
                self.last_order = Some(Instant::now());
            } else {
                let prev = &self.order_prev;
                v.sort_by_key(|g| prev.get(&g.key).copied().unwrap_or(usize::MAX));
            }
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
            push(&mut rows, &mut y, RowK::Sec(0, hits.len()), 32.0);
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
        self.tick_scroll();
        self.track_ghosts();
        let t = self.t;
        self.gfx.begin();
        // Mica shows through transparent pixels of the window target; screenshots use the solid fallback
        if self.backdrop_on() && !self.gfx.offscreen() {
            self.gfx.clear(super::gfx::C(0.0, 0.0, 0.0, 0.0));
        } else {
            self.gfx.clear(t.mica);
        }
        self.paint_title();
        if first {
            trace(&format!("  paint title: {:.1} ms", self.started.elapsed().as_secs_f64() * 1000.0));
        }
        match self.page {
            0 => self.paint_processes(),
            1 => self.paint_startup(),
            _ => self.paint_settings(),
        }
        self.paint_overlays();
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
    fn shot(&mut self, path: &str) -> Result<(), String> {
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
            // two passes: the first one loads icons for the new target
            self.paint();
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
            if !matches!(self.rows[i as usize].k, RowK::PinSep | RowK::Sec(..)) {
                break;
            }
        }
        self.select_row(i as usize);
        self.scroll_to(i as usize);
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
        if risky && self.prefs.ask_end {
            let hung = g.as_ref().map(|g| g.hung).unwrap_or(false);
            let body = if hung { format!("{} is not responding. Text you have not sent or saved may be lost.", job.label) } else { format!("{} may have work you have not saved. It will be lost.", job.label) };
            self.dialog = Some(Dialog { title: format!("End {}?", job.label), body, job, at: Instant::now() });
            self.animate(220);
        } else {
            self.run_end(job);
        }
    }

    pub fn run_end(&mut self, job: EndJob) {
        let me = std::process::id();
        let mut fails = 0;
        for p in &job.pids {
            if *p == me || *p <= 4 {
                // the window never ends itself or the kernel
                fails += 1;
                continue;
            }
            if self.test && !test_owned(&self.procs, *p) {
                // test mode: only processes started by the tester's own scripts may be ended
                fails += 1;
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
        self.last_action = format!("end {:?} fails={fails}", job.pids);
        if fails == job.pids.len() && fails > 0 {
            self.toast_msg(&format!("Could not end {} (needs admin?)", job.label), false);
        } else {
            let text = if job.mem >= 1 << 20 { format!("{} ended · freed {}", job.label, model::fmt_mem(job.mem)) } else { format!("{} ended", job.label) };
            self.toast = Some(Toast { text, code: String::new(), link: job.relaunch.clone().map(|(c, d)| ("Restart".to_string(), ToastAct::Relaunch(c, d))), ok: true, at: Instant::now(), secs: 8 });
            self.animate(220);
        }
        if matches!(self.sel, Some(Sel::Child(_))) || job.count > 0 {
            self.sel = None;
        }
        if let Some(p) = &self.pane
            && p.kind == PaneKind::Details
        {
            self.pane = None;
        }
        self.ctl.now.store(true, Relaxed);
        self.invalidate();
    }

    /// Restart: end the selection's tree and start its root again with the same command line and folder.
    pub fn restart_selected(&mut self) {
        let Some(g) = self.sel_group().cloned() else { return };
        let pid = self.sel_pid().unwrap_or(g.root());
        if g.kind == procs::Kind::Windows {
            self.toast_msg("Windows processes are not restarted here", false);
            return;
        }
        let Some((cmd, dir)) = relaunch_info(pid) else {
            self.toast_msg("Cannot read its command line (run as administrator?)", false);
            return;
        };
        let Some(mut job) = self.end_job(matches!(self.sel, Some(Sel::Child(_)))) else { return };
        job.relaunch = None;
        let label = job.label.clone();
        self.run_end(job);
        let ok = self.test_or_spawn(&cmd, &dir, 300);
        self.toast = Some(Toast { text: if ok { format!("{label} restarted") } else { format!("Could not start {label} again") }, code: String::new(), link: None, ok, at: Instant::now(), secs: 4 });
        self.animate(220);
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

    pub fn open_location(&mut self) {
        let Some(g) = self.sel_group() else { return };
        let path = self.sel_pid().and_then(|p| self.proc(p)).map(|p| p.meta.path.clone()).filter(|p| !p.is_empty()).unwrap_or(g.path.clone());
        if path.is_empty() {
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
        self.run_end(job);
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
        self.run_end(job);
        self.pane = Some(Pane { kind: PaneKind::Free, key: String::new(), pid: 0, open: [false; 3], info: PaneInfo::default(), loading: false, ai_loading: false, keep_open: false, at: Instant::now() - Duration::from_secs(1) });
    }

    pub fn toggle_startup(&mut self, i: usize) {
        let Some(e) = self.startup.get(i).cloned() else { return };
        if self.test {
            self.last_action = format!("startup toggle blocked in test mode: {}", e.name);
            self.toast_msg(&format!("{} {} at startup (test mode: not changed)", e.name, if e.enabled { "off" } else { "on" }), true);
            return;
        }
        match crate::startup::set_enabled(&e.name, !e.enabled) {
            Ok(_) => {
                let mut v = (*self.startup).clone();
                v[i].enabled = !e.enabled;
                self.startup = Arc::new(v);
                self.ctl.reload_startup.store(true, Relaxed);
            }
            Err(err) => self.toast_msg(&format!("Could not change {}: {err}", e.name), false),
        }
        self.invalidate();
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
        } else if (old.accent, old.transp) != (p.accent, p.transp) {
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
        let h = self.hit_at(x, y);
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
        self.search_focus = h == Hit::Search || h == Hit::SearchClear || (self.search_focus && self.searching() && !matches!(h, Hit::Tab(_) | Hit::Settings));
        match h {
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
            Hit::StRow(i) => self.sel = Some(Sel::Startup(i)),
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

    pub fn click(&mut self, h: Hit, x: f32, y: f32) {
        // an open menu takes the click
        if self.menu.is_some() {
            if let Hit::MenuItem(i) = h {
                self.menu_action(i);
                return;
            }
            self.menu = None;
            if matches!(h, Hit::Block | Hit::None) {
                return;
            }
        }
        if self.dialog.is_some() {
            match h {
                Hit::DlgEnd => {
                    let d = self.dialog.take().unwrap();
                    self.run_end(d.job);
                }
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
            Hit::ChipMore => self.menu = Some(Menu { kind: MenuKind::Chips, x, y: y + 8.0, at: Instant::now() }),
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
                    self.menu = Some(Menu { kind: MenuKind::Row, x, y: y + 10.0, at: Instant::now() });
                    self.animate(180);
                }
            }
            Hit::Free | Hit::FlyFree => self.open_free(),
            Hit::Sort => {
                self.menu = Some(Menu { kind: MenuKind::Sort, x: x - 40.0, y: y + 14.0, at: Instant::now() });
                self.animate(180);
            }
            Hit::Header(c) => {
                let s = [SortKey::Name, SortKey::Cpu, SortKey::Mem, SortKey::Disk, SortKey::Net, SortKey::Gpu][c as usize];
                self.set_sort(if self.sort == s { SortKey::Impact } else { s });
            }
            Hit::SrStartupToggle(i) | Hit::StToggle(i) => self.toggle_startup(i),
            Hit::SrServiceBtn(i) => self.service_action(i),
            Hit::PaneClose => self.pane = None,
            Hit::PaneEnd => self.end_selected(false),
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
            Hit::StSort => self.menu = Some(Menu { kind: MenuKind::StartupSort, x: x - 60.0, y: y + 14.0, at: Instant::now() }),
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
                _ => {
                    let on = !self.ai_on;
                    self.set_ai(on)
                }
            },
            Hit::CliCopy(i) => {
                let c = super::pages::CLI.get(i as usize).map(|c| c.0.to_string()).unwrap_or_default();
                self.copy(&c);
            }
            Hit::KeyField => self.key_focus = true,
            Hit::KeySave => self.key_save(),
            Hit::KeyTest => self.key_test(),
            Hit::KeyRemove => self.key_remove(),
            Hit::KeyLink => self.open_key_page(),
            Hit::Accent(i) => {
                if i == 0 {
                    self.custom_open = false;
                    self.set_pref(|p| p.accent = None);
                } else if let Some((_, rgb)) = theme::PRESETS.get(i as usize - 1) {
                    self.custom_open = false;
                    let rgb = *rgb;
                    self.set_pref(|p| p.accent = Some(rgb));
                } else {
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
            Hit::DefSort => {
                self.menu = Some(Menu { kind: MenuKind::DefSort, x: x - 60.0, y: y + 14.0, at: Instant::now() });
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
        self.run_end(job);
    }

    fn wheel(&mut self, x: f32, y: f32, d: f32) {
        self.menu = None;
        let k = if self.page == 1 {
            1
        } else if self.page == 2 {
            2
        } else if self.pane.is_some() && self.pane_rect.has(x, y) {
            3
        } else {
            0
        };
        self.scroll_by(k, -d / 120.0 * self.row_h * 3.0);
        self.invalidate();
    }

    fn key(&mut self, vk: u16) -> bool {
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
                } else {
                    self.sel = None;
                }
            }
            VK_RETURN if self.dialog.is_some() => self.dialog = None,
            VK_DOWN => self.move_sel(1),
            VK_UP => self.move_sel(-1),
            VK_PRIOR => self.scroll_by(0, -(self.list_rect.h - self.row_h)),
            VK_NEXT => self.scroll_by(0, self.list_rect.h - self.row_h),
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
            VK_C if ctrl && shift => self.copy_command(),
            VK_BACK => {
                if ctrl {
                    let t = self.query.trim_end().rfind(' ').map(|i| i + 1).unwrap_or(0);
                    self.query.truncate(t);
                } else {
                    self.query.pop();
                }
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
                self.query.push_str(s.lines().next().unwrap_or("").trim());
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
        self.query.push(ch);
        self.search_focus = true;
        self.set_scroll(0, 0.0);
        self.page = 0;
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
                "glyphs" => {
                    let (font, missing) = self.gfx.icon_check(&GLYPHS_USED);
                    return json!({"ok": true, "icon_font": font, "checked": GLYPHS_USED.len(), "missing": missing.iter().map(|c| format!("{c:X}")).collect::<Vec<_>>()}).to_string();
                }
                "quit" => {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                "state" => return self.state_json().to_string(),
                "shot" => {
                    if arg.is_empty() {
                        return err("shot needs a .bmp path");
                    }
                    self.take_data();
                    return match self.shot(&arg) {
                        Ok(()) => json!({"ok": true, "file": arg}).to_string(),
                        Err(e) => err(&e),
                    };
                }
                "search" => {
                    self.query = arg.clone();
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
                        "row" => self.menu = Some(Menu { kind: MenuKind::Row, x, y, at: Instant::now() }),
                        "sort" => self.menu = Some(Menu { kind: MenuKind::Sort, x: self.w - 190.0, y: TITLE_H + TILES_H + 70.0, at: Instant::now() }),
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
                }
                "click" => {
                    let x: f32 = a1.parse().unwrap_or(-1.0);
                    let y: f32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(-1.0);
                    self.mouse_down(x, y, false);
                    self.mouse_up(x, y);
                }
                "sort" => match SortKey::parse(a1) {
                    Some(s) => self.set_sort(s),
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
                    let Some(d) = self.dialog.take() else { return err("no dialog is open") };
                    if a1 == "end" {
                        self.run_end(d.job);
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
                            self.set_pref(|p| p.accent = acc)
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
                        _ => return err("key esc|down|up|enter|f1|del|right|left"),
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

/// Caption buttons: Windows 11 size (46 x 32 DIPs) at the top right.
pub const CAP_W: f32 = 46.0;
pub const CAP_H: f32 = 32.0;

/// Glyphs the window draws from the icon font (checked by `wtm gui glyphs`).
pub const GLYPHS_USED: [u32; 24] = [
    0xE921, 0xE922, 0xE923, 0xE8BB, 0xE713, 0xE721, 0xE711, 0xE72C, 0xE946, 0xE712, 0xEA99, 0xE8CB, 0xE70D, 0xE76C, 0xE790, 0xE771, 0xE81E, 0xE8D2, 0xE8A9, 0xE739, 0xE89F, 0xE895, 0xE9D2, 0xE70F,
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
                let h = app.hit_at(x, y);
                if h != app.hover {
                    app.hover = h;
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                    app.invalidate();
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                app.hover = Hit::None;
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
                if let Hit::Row(i) | Hit::Chevron(i) = app.hit_at(x, y)
                    && matches!(app.rows.get(i).map(|r| r.k), Some(RowK::Group(_)) | Some(RowK::Child(..)))
                {
                    app.select_row(i);
                    app.menu = Some(Menu { kind: MenuKind::Row, x, y, at: Instant::now() });
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
                if let Some(c) = char::from_u32(wp.0 as u32) {
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
                mi.ptMinTrackSize = POINT { x: (900.0 * dpi) as i32, y: (620.0 * dpi) as i32 };
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
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
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
    let prefs = Prefs::load();
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
            lpszClassName: cls,
            hbrBackground: CreateSolidBrush(COLORREF(((t.mica.2 * 255.0) as u32) << 16 | ((t.mica.1 * 255.0) as u32) << 8 | (t.mica.0 * 255.0) as u32)),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // 1180x760 DIPs on the primary monitor, centred. No WS_SYSMENU: DWM must not draw its own caption
        // buttons over the Mica frame (the window draws Fluent ones and answers HTMINBUTTON/HTMAXBUTTON/HTCLOSE).
        let dpi = GetDpiForSystem() as f32 / 96.0;
        let (w, h) = ((1180.0 * dpi) as i32, (760.0 * dpi) as i32);
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let style = if want_mica { WS_CAPTION | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_CLIPCHILDREN } else { WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN };
        let (ex, x0, y0) = if offscreen { (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, -20000, -20000) } else { (WINDOW_EX_STYLE(0), (sw - w) / 2, (sh - h) / 2) };
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
            query: String::new(),
            search_focus: false,
            scroll: [0.0; 4],
            drag_scroll: None,
            hits: vec![],
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
        let _ = ShowWindow(hwnd, if offscreen { SW_SHOWNOACTIVATE } else { SW_SHOW });
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
