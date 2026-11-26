//! The picture viewer: a second top-level window of the same process (same thread, same message loop). Opening a
//! picture from a folder list shows it here and steps through the other pictures of that list, in the list's own
//! order (sort column, direction, filter, search results), with the arrow keys, the wheel, the side arrows or the
//! filmstrip. Decoding (WIC, fmcore::picture) runs on two worker threads; the next and previous pictures are decoded
//! ahead, so stepping is instant; a big picture shows its shell thumbnail until it is ready.
//!
//! Ownership: the window owns its `Viewer` (a Box in GWLP_USERDATA, freed in WM_NCDESTROY). The main window reaches it
//! through `get()`; the viewer never touches the main window's state directly, it posts `Msg`s to it.
use crate::app::{App, CAP_W, Msg, lparam_xy, mods, post_bg, set_dark_frame};
use crate::gfx::{Align, C, Font, Gfx, Rect};
use crate::motion::{Fades, Follow};
use crate::theme::{R_CARD, R_CTRL, R_SMALL, Theme};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
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

const TITLE_H: f32 = 40.0;
const BAR_H: f32 = 44.0;
const STRIP_H: f32 = 76.0;
const THUMB: f32 = 52.0;
const GAP: f32 = 6.0;
/// pixel budget of one decoded picture (a 50 MP photo; bigger ones are scaled down to it)
const MAX_PX: u64 = 50_000_000;
const MIN_W: f32 = 480.0;
const MIN_H: f32 = 360.0;

const WM_VBG: u32 = WM_APP + 21;
const T_ANIM: usize = 1;
const T_TIP: usize = 2;
const T_TOAST: usize = 3;
const T_IDLE: usize = 4;
const T_LOAD: usize = 5;

/// light ink on the dark picture area (the same in both themes)
const INK: C = C(0.95, 0.96, 0.98, 1.0);
const INK2: C = C(0.95, 0.96, 0.98, 0.64);

static VIEWER: AtomicIsize = AtomicIsize::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VHit {
    None,
    Min,
    Max,
    Close,
    Title,
    Prev,
    Next,
    ZoomOut,
    ZoomIn,
    Fit,
    Rotate,
    Delete,
    OpenApp,
    Reveal,
    Strip,
    Full,
    ArrowL,
    ArrowR,
    Thumb(usize),
    StripBg,
    Image,
    Stage,
    Undo,
    ToastClose,
    Yes,
    No,
    Block,
}

enum VMsg {
    /// `full` = decoded at its own size (asked when zooming in past the screen-size copy)
    Decoded { path: PathBuf, full: bool, res: Result<(fmcore::picture::Decoded, bool), String>, ms: f64 },
    Thumb { path: PathBuf, img: Option<(u32, u32, Vec<u8>)>, dims: Option<(u32, u32)> },
    Deleted { path: PathBuf, at: usize, res: Result<(), String> },
    Restored { path: PathBuf, at: usize, res: Result<(), String> },
    Error(String),
}

/// One decoded picture: upright size of the original, the bitmap (made on the UI thread from `px`), or the error.
/// Pictures are decoded at most screen size first (small, quick, the neighbours too); zooming in past that asks for
/// the current one at its own size (`full`).
struct Pic {
    w: u32,
    h: u32,
    /// the bitmap's own width, and whether it is the whole picture (not the screen-size copy)
    bw: u32,
    full: bool,
    px: Option<(u32, u32, Vec<u8>)>,
    bmp: Option<ID2D1Bitmap>,
    err: Option<String>,
    /// shown from the shell thumbnail because Windows has no decoder for it
    approx: bool,
    ms: f64,
    at: Instant,
}

struct Thumb {
    w: u32,
    h: u32,
    px: Vec<u8>,
    bmp: Option<(usize, ID2D1Bitmap)>,
}

/// A decode job: the picture and whether it is wanted at its own size (else at most screen size).
type Job = (PathBuf, bool);

#[derive(Default)]
struct DecodeQ {
    want: Vec<Job>,
    busy: HashSet<Job>,
    /// decoded, posted, not yet taken by the window (not asked again meanwhile)
    done: HashSet<Job>,
    /// longest side of a full decode (the GPU's limit) and of a screen-size one (the monitor's long side)
    cap: u32,
    cap_view: u32,
    quit: bool,
}

struct VToast {
    text: String,
    undo: Option<(PathBuf, usize)>,
    err: bool,
    at: Instant,
}

pub struct Viewer {
    pub hwnd: HWND,
    main: HWND,
    main_bg: Arc<Mutex<Vec<Msg>>>,
    gfx: Gfx,
    t: Theme,
    stage: C,
    mica: bool,
    /// the DWM backdrop type set on the window (None: no Mica on this Windows); put back after full screen
    bd: Option<i32>,
    pa: f32,
    pub list: Vec<PathBuf>,
    pub idx: usize,
    pics: HashMap<PathBuf, Pic>,
    thumbs: HashMap<PathBuf, Thumb>,
    dims: HashMap<PathBuf, (u32, u32)>,
    thumb_asked: HashSet<PathBuf>,
    rot: HashMap<PathBuf, u8>,
    /// turning animation: (start, from angle, to angle), degrees clockwise
    rot_anim: Option<(Instant, f32, f32)>,
    fit: bool,
    zs: Follow,
    zx: Follow,
    zy: Follow,
    drag: Option<(f32, f32, f32, f32)>,
    dragged: bool,
    pressed: VHit,
    pub full: bool,
    saved: Option<WINDOWPLACEMENT>,
    pub strip: bool,
    ask_delete: bool,
    confirm: bool,
    w: f32,
    h: f32,
    hits: Vec<(Rect, VHit)>,
    hover: VHit,
    nc_hover: VHit,
    nc_down: VHit,
    hov_f: Fades<VHit>,
    press_f: Fades<VHit>,
    /// the pointer is over the picture area (side arrows show)
    area_f: Fades<u8>,
    tracking: bool,
    mx: f32,
    my: f32,
    tip: Option<(VHit, String, f32, f32)>,
    toast: Option<VToast>,
    strip_x: Follow,
    wheel_acc: f32,
    bg: Arc<Mutex<Vec<VMsg>>>,
    dq: Arc<(Mutex<DecodeQ>, Condvar)>,
    tq: Arc<(Mutex<Vec<PathBuf>>, Condvar)>,
    quit: Arc<AtomicBool>,
    bmp_target: usize,
    shooting: bool,
    /// handling a `wfm gui view-...` command (synthetic mouse)
    synthetic: bool,
    shot_hover: bool,
    shot_bmp: Option<ID2D1Bitmap>,
    offscreen: bool,
    test: bool,
    shown_at: Instant,
    last_move: Instant,
    apps: HashMap<String, String>,
    pub last_error: Option<String>,
    opened: Instant,
    first_ms: Option<f64>,
}

/// The open viewer, if any.
pub fn get() -> Option<&'static mut Viewer> {
    let h = VIEWER.load(Ordering::Relaxed);
    if h == 0 {
        return None;
    }
    unsafe {
        let hw = HWND(h as *mut _);
        if !IsWindow(Some(hw)).as_bool() {
            return None;
        }
        let p = GetWindowLongPtrW(hw, GWLP_USERDATA) as *mut Viewer;
        if p.is_null() { None } else { Some(&mut *p) }
    }
}

/// Show `list` (pictures in the list's order) from `idx`: in the open viewer (its list is replaced), else a new one.
pub fn open(a: &mut App, list: Vec<PathBuf>, idx: usize) {
    if list.is_empty() {
        return;
    }
    if let Some(v) = get() {
        v.ask_delete = a.ui.ask_delete;
        v.set_list(list, idx);
        v.activate();
        return;
    }
    if let Err(e) = create(a, list, idx) {
        a.toast_err(&e);
    }
}

/// The main window is closing: the viewer goes with it (one process, one message loop).
pub fn close() {
    if let Some(v) = get() {
        unsafe {
            let _ = DestroyWindow(v.hwnd);
        }
    }
}

/// Theme, accent, backdrop or a setting changed in the main window: the viewer follows at once.
pub fn sync(a: &App) {
    let Some(v) = get() else { return };
    v.t = a.t.clone();
    v.stage = stage_colour(a);
    let mica = a.can_mica && a.look.backdrop != 3;
    v.pa = if mica { a.look.layer_alpha() } else { 1.0 };
    v.ask_delete = a.ui.ask_delete;
    unsafe {
        set_dark_frame(v.hwnd, a.t.dark);
        if a.can_mica {
            let b: i32 = a.look.dwm_backdrop();
            let _ = DwmSetWindowAttribute(v.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &b as *const i32 as *const _, 4);
        }
    }
    v.mica = mica;
    v.bd = if a.can_mica { Some(a.look.dwm_backdrop()) } else { None };
    v.gfx.set_alpha(mica);
    v.gfx.corner = a.gfx.corner;
    v.invalidate();
}

/// The picture area's colour: the accent-tinted dark window colour a few steps darker, in both themes.
fn stage_colour(a: &App) -> C {
    let custom = if a.look.desktop { a.desk.map(crate::theme::to_hex) } else { a.look.accent };
    let d = crate::theme::make(true, custom, a.tint(), a.look.strength);
    d.mica_a.mix(C(0.0, 0.0, 0.0, 1.0), 0.30)
}

fn settings_path() -> PathBuf {
    fmcore::util::data_dir().join("viewer.json")
}

fn load_settings() -> serde_json::Value {
    std::fs::read_to_string(settings_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null)
}

fn save_setting(key: &str, v: serde_json::Value) {
    let mut cfg = load_settings();
    if !cfg.is_object() {
        cfg = json!({});
    }
    cfg[key] = v;
    let p = settings_path();
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(&cfg).unwrap()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

fn vpost(bg: &Arc<Mutex<Vec<VMsg>>>, hw: isize, m: VMsg) {
    bg.lock().unwrap().push(m);
    unsafe {
        let _ = PostMessageW(Some(HWND(hw as *mut _)), WM_VBG, WPARAM(0), LPARAM(0));
    }
}

fn normal_style() -> WINDOW_STYLE {
    WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_SYSMENU.0) | WS_CLIPCHILDREN
}

fn create(a: &mut App, list: Vec<PathBuf>, idx: usize) -> Result<(), String> {
    static CLASS: std::sync::Once = std::sync::Once::new();
    let test = std::env::var("WFILES_TEST").is_ok();
    let offscreen = a.offscreen;
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let cls = w!("RustFileExplorerViewer");
        CLASS.call_once(|| {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: inst.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: LoadIconW(Some(inst.into()), PCWSTR(1 as _)).unwrap_or_default(),
                lpszClassName: cls,
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                ..Default::default()
            };
            RegisterClassExW(&wc);
        });
        let dpi = GetDpiForWindow(a.hwnd).max(96) as f32 / 96.0;
        let (dw, dh) = (1120.0, 780.0);
        let (pw, ph) = ((dw * dpi) as i32, (dh * dpi) as i32);
        // centred on the main window's monitor
        let hm = MonitorFromWindow(a.hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(hm, &mut mi);
        let wa = mi.rcWork;
        let (pw, ph) = (pw.min(wa.right - wa.left), ph.min(wa.bottom - wa.top));
        let (mut x, mut y) = (wa.left + (wa.right - wa.left - pw) / 2, wa.top + (wa.bottom - wa.top - ph) / 2);
        let mut ex = WINDOW_EX_STYLE(0);
        if offscreen {
            ex = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
            x = -20000;
            y = -20000;
        }
        let hwnd = CreateWindowExW(ex, cls, w!("Rust File Explorer"), normal_style(), x, y, pw, ph, None, None, Some(inst.into()), None).map_err(|e| e.message())?;
        set_dark_frame(hwnd, a.t.dark);
        let mica = a.can_mica && a.look.backdrop != 3;
        if a.can_mica {
            let b: i32 = a.look.dwm_backdrop();
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &b as *const i32 as *const _, 4);
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
        } else {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });
        }
        let Ok(mut gfx) = Gfx::new(hwnd, a.gfx.software, mica, a.gfx.shared_device()) else {
            let _ = DestroyWindow(hwnd);
            return Err("the viewer could not start drawing".into());
        };
        gfx.corner = a.gfx.corner;
        let cfg = load_settings();
        let now = Instant::now();
        let v = Box::new(Viewer {
            hwnd,
            main: a.hwnd,
            main_bg: a.bg.clone(),
            gfx,
            t: a.t.clone(),
            stage: stage_colour(a),
            mica,
            bd: if a.can_mica { Some(a.look.dwm_backdrop()) } else { None },
            pa: if mica { a.look.layer_alpha() } else { 1.0 },
            list: vec![],
            idx: 0,
            pics: HashMap::new(),
            thumbs: HashMap::new(),
            dims: HashMap::new(),
            thumb_asked: HashSet::new(),
            rot: HashMap::new(),
            rot_anim: None,
            fit: true,
            zs: Follow::default(),
            zx: Follow::default(),
            zy: Follow::default(),
            drag: None,
            dragged: false,
            pressed: VHit::None,
            full: false,
            saved: None,
            strip: cfg["filmstrip"].as_bool().unwrap_or(true),
            ask_delete: a.ui.ask_delete,
            confirm: false,
            w: dw,
            h: dh,
            hits: vec![],
            hover: VHit::None,
            nc_hover: VHit::None,
            nc_down: VHit::None,
            hov_f: Fades::new(crate::motion::FAST, crate::motion::NORMAL),
            press_f: Fades::new(60.0, crate::motion::NORMAL),
            area_f: Fades::new(crate::motion::NORMAL, crate::motion::SLOW),
            tracking: false,
            mx: -1.0,
            my: -1.0,
            tip: None,
            toast: None,
            strip_x: Follow::default(),
            wheel_acc: 0.0,
            bg: Arc::new(Mutex::new(vec![])),
            dq: Arc::new((Mutex::new(DecodeQ { cap: 16384, cap_view: 2560, ..Default::default() }), Condvar::new())),
            tq: Arc::new((Mutex::new(vec![]), Condvar::new())),
            quit: Arc::new(AtomicBool::new(false)),
            bmp_target: 0,
            shooting: false,
            synthetic: false,
            shot_hover: false,
            shot_bmp: None,
            offscreen,
            test,
            shown_at: now,
            last_move: now,
            apps: HashMap::new(),
            last_error: None,
            opened: now,
            first_ms: None,
        });
        let ptr = Box::into_raw(v);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        VIEWER.store(hwnd.0 as isize, Ordering::Relaxed);
        let v = &mut *ptr;
        v.start_workers();
        v.set_list(list, idx);
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        // the last place and size (only while still on a monitor)
        let mut show = if offscreen || test { SW_SHOWNOACTIVATE } else { SW_SHOW };
        if !offscreen
            && !test
            && let Some(a5) = cfg["window"].as_array().filter(|a| a.len() == 5)
        {
            let n = |i: usize| a5[i].as_i64().unwrap_or(0) as i32;
            let r = RECT { left: n(0), top: n(1), right: n(2), bottom: n(3) };
            if r.right - r.left >= 300 && r.bottom - r.top >= 200 && !MonitorFromRect(&r, MONITOR_DEFAULTTONULL).is_invalid() {
                let wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, showCmd: SW_HIDE.0 as u32, rcNormalPosition: r, ..Default::default() };
                let _ = SetWindowPlacement(hwnd, &wp);
                if n(4) == 1 {
                    show = SW_SHOWMAXIMIZED;
                }
            }
        }
        let _ = ShowWindow(hwnd, show);
        let _ = UpdateWindow(hwnd);
        if !offscreen && !test {
            let _ = SetForegroundWindow(hwnd);
        }
    }
    Ok(())
}

impl Viewer {
    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn anim(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), T_ANIM, 15, None);
        }
    }

    fn timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), id, ms, None);
        }
    }

    fn k(&self) -> f32 {
        self.gfx.scale()
    }

    /// Bring the window up (a new list while it is open). Never activated in tests.
    fn activate(&self) {
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, if self.test || self.offscreen { SW_SHOWNOACTIVATE } else { SW_RESTORE });
            }
            if !self.test && !self.offscreen {
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
    }

    fn start_workers(&mut self) {
        let hw = self.hwnd.0 as isize;
        // two decoders: the current picture never waits behind a neighbour being decoded
        for _ in 0..2 {
            let dq = self.dq.clone();
            let bg = self.bg.clone();
            std::thread::spawn(move || {
                let _c = fmcore::util::com();
                loop {
                    let (job, cap) = {
                        let (m, cv) = &*dq;
                        let mut q = m.lock().unwrap();
                        loop {
                            if q.quit {
                                return;
                            }
                            if let Some(i) = q.want.iter().position(|p| !q.busy.contains(p)) {
                                let p = q.want.remove(i);
                                q.busy.insert(p.clone());
                                let cap = if p.1 { q.cap } else { q.cap_view.min(q.cap) };
                                break (p, cap);
                            }
                            q = cv.wait(q).unwrap();
                        }
                    };
                    let (path, full) = job.clone();
                    let t0 = Instant::now();
                    let res = match fmcore::picture::decode(&path, cap, MAX_PX, true) {
                        Ok(d) => Ok((d, false)),
                        // no codec (HEIC without its extension, odd RAW): the shell's own large thumbnail
                        Err(e) => match fmcore::thumb::thumbnail(&path, 2048, fmcore::thumb::ThumbKind::ThumbnailOnly) {
                            Ok(mut i) => {
                                crate::preview::premultiply(&mut i.bgra);
                                Ok((fmcore::picture::Decoded { width: i.width, height: i.height, bgra: i.bgra, full_w: i.width, full_h: i.height, orientation: 1, frames: 1 }, true))
                            }
                            Err(_) => Err(e),
                        },
                    };
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    {
                        let (m, _) = &*dq;
                        let mut q = m.lock().unwrap();
                        q.busy.remove(&job);
                        q.want.retain(|p| p != &job);
                        q.done.insert(job);
                    }
                    vpost(&bg, hw, VMsg::Decoded { path, full, res, ms });
                }
            });
        }
        // thumbnails for the filmstrip (and the first look at a big picture): newest request first
        let tq = self.tq.clone();
        let bg = self.bg.clone();
        let quit = self.quit.clone();
        let px = (THUMB * 2.0 * unsafe { GetDpiForWindow(self.hwnd) } as f32 / 96.0).round().max(96.0) as u32;
        std::thread::spawn(move || {
            let _c = fmcore::util::com();
            loop {
                let path = {
                    let (m, cv) = &*tq;
                    let mut g = m.lock().unwrap();
                    loop {
                        if quit.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Some(p) = g.pop() {
                            break p;
                        }
                        g = cv.wait(g).unwrap();
                    }
                };
                let img = fmcore::thumb::thumbnail(&path, px, fmcore::thumb::ThumbKind::Auto).ok().map(|mut i| {
                    crate::preview::premultiply(&mut i.bgra);
                    (i.width, i.height, i.bgra)
                });
                let dims = crate::preview::image_size(&path);
                vpost(&bg, hw, VMsg::Thumb { path, img, dims });
            }
        });
    }

    fn stop_workers(&self) {
        self.quit.store(true, Ordering::Relaxed);
        {
            let (m, cv) = &*self.dq;
            m.lock().unwrap().quit = true;
            cv.notify_all();
        }
        let (_, cv) = &*self.tq;
        cv.notify_all();
    }

    /// A new list (another picture opened while the viewer is open, or the first one).
    pub fn set_list(&mut self, list: Vec<PathBuf>, idx: usize) {
        self.list = list;
        self.confirm = false;
        self.thumb_asked.retain(|p| self.thumbs.contains_key(p));
        let n = self.list.len();
        self.go(idx.min(n.saturating_sub(1)));
        // the filmstrip jumps to the new place (no long slide through another folder)
        let t = self.strip_target();
        self.strip_x.set(t);
    }

    fn cur(&self) -> Option<&PathBuf> {
        self.list.get(self.idx)
    }

    fn go(&mut self, i: usize) {
        if self.list.is_empty() {
            return;
        }
        let i = i.min(self.list.len() - 1);
        self.idx = i;
        self.tip = None;
        self.fit = true;
        self.rot_anim = None;
        self.drag = None;
        self.shown_at = Instant::now();
        self.wheel_acc = 0.0;
        let st = self.stage_rect();
        let fs = self.fit_scale(st);
        self.zs.set(fs);
        self.zx.set(0.0);
        self.zy.set(0.0);
        let t = self.strip_target();
        self.strip_x.to(t);
        self.request();
        self.ask_thumb(i);
        self.update_title();
        self.timer(T_LOAD, 320);
        self.anim();
        self.invalidate();
    }

    fn step(&mut self, d: i32) {
        if self.list.is_empty() {
            return;
        }
        let n = self.list.len() as i32;
        let i = (self.idx as i32 + d).clamp(0, n - 1);
        if i as usize != self.idx {
            self.go(i as usize);
        }
    }

    fn update_title(&self) {
        let name = self.cur().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let s = fmcore::util::wide(&format!("{name} - Rust File Explorer"));
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(s.as_ptr()));
        }
    }

    /// Ask the decoders for the current picture, then the next and the previous; forget pictures far away.
    fn request(&mut self) {
        let n = self.list.len();
        if n == 0 {
            return;
        }
        let near = |d: i32| -> Option<PathBuf> {
            let i = self.idx as i32 + d;
            (i >= 0 && (i as usize) < n).then(|| self.list[i as usize].clone())
        };
        let keep: HashSet<PathBuf> = (-2..=2).filter_map(near).collect();
        let before = self.pics.len();
        self.pics.retain(|p, _| keep.contains(p));
        if self.pics.len() < before {
            // pictures left behind: their memory goes back at once, not at some later frame
            self.gfx.release_memory(false);
        }
        let cap = self.gfx.max_bitmap().clamp(2048, 16384);
        // a screen-size copy is enough until the picture is zoomed past it (the monitor's long side, in pixels)
        let cap_view = unsafe {
            let hm = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(hm, &mut mi);
            let m = mi.rcMonitor;
            (m.right - m.left).max(m.bottom - m.top).max(1024) as u32
        };
        // the current picture at its own size when the view needs more pixels than its copy has
        let full: Option<PathBuf> = self.cur().filter(|p| self.pics.get(*p).is_some_and(|x| !x.full && x.err.is_none() && self.needs_full(x))).cloned();
        let (m, cv) = &*self.dq;
        {
            let mut q = m.lock().unwrap();
            q.cap = cap;
            q.cap_view = cap_view;
            let mut want: Vec<Job> = full.map(|p| (p, true)).into_iter().collect();
            want.extend([0, 1, -1].into_iter().filter_map(near).filter(|p| !self.pics.contains_key(p)).map(|p| (p, false)));
            q.want = want.into_iter().filter(|j| !q.busy.contains(j) && !q.done.contains(j)).collect();
        }
        cv.notify_all();
    }

    fn ask_thumb(&mut self, i: usize) {
        let Some(p) = self.list.get(i).cloned() else { return };
        if self.thumbs.contains_key(&p) || !self.thumb_asked.insert(p.clone()) {
            return;
        }
        let (m, cv) = &*self.tq;
        {
            let mut q = m.lock().unwrap();
            q.retain(|x| x != &p);
            q.push(p);
            // only the newest requests matter (a long folder scrolled fast)
            let len = q.len();
            if len > 96 {
                let old: Vec<PathBuf> = q.drain(..len - 96).collect();
                for o in old {
                    self.thumb_asked.remove(&o);
                }
            }
        }
        cv.notify_one();
    }

    fn take_bg(&mut self) {
        let msgs: Vec<VMsg> = std::mem::take(&mut *self.bg.lock().unwrap());
        for m in msgs {
            self.apply(m);
        }
    }

    fn apply(&mut self, m: VMsg) {
        match m {
            VMsg::Decoded { path, full, res, ms } => {
                {
                    let (q, _) = &*self.dq;
                    q.lock().unwrap().done.remove(&(path.clone(), full));
                }
                let near = self.list.iter().position(|p| p == &path).is_some_and(|i| (i as i32 - self.idx as i32).abs() <= if full { 0 } else { 2 });
                // a screen-size copy never replaces the whole picture
                if !near || (!full && self.pics.get(&path).is_some_and(|p| p.full)) {
                    return;
                }
                let at = if full { self.pics.get(&path).map(|p| p.at).unwrap_or_else(Instant::now) } else { Instant::now() };
                let pic = match res {
                    Ok((d, approx)) => Pic { w: d.full_w, h: d.full_h, bw: d.width, full: full || approx || (d.width, d.height) == (d.full_w, d.full_h), px: Some((d.width, d.height, d.bgra)), bmp: None, err: None, approx, ms, at },
                    // the whole picture failed (too big for memory): keep the screen-size copy
                    Err(_) if full && self.pics.contains_key(&path) => {
                        if let Some(p) = self.pics.get_mut(&path) {
                            p.full = true;
                        }
                        return;
                    }
                    Err(e) => Pic { w: 0, h: 0, bw: 0, full: true, px: None, bmp: None, err: Some(e), approx: false, ms, at },
                };
                if full {
                    // the copy's bitmap goes before the big one is made
                    self.pics.remove(&path);
                    self.gfx.release_memory(false);
                }
                let is_cur = self.cur() == Some(&path);
                self.pics.insert(path.clone(), pic);
                self.upload(&path);
                if is_cur {
                    if self.first_ms.is_none() {
                        self.first_ms = Some(self.opened.elapsed().as_secs_f64() * 1000.0);
                    }
                    // the real size is known now: fit again (the thumbnail was only a guess)
                    if self.fit {
                        let st = self.stage_rect();
                        let fs = self.fit_scale(st);
                        self.zs.set(fs);
                    }
                    self.anim();
                }
                self.invalidate();
            }
            VMsg::Thumb { path, img, dims } => {
                if let Some(d) = dims {
                    self.dims.insert(path.clone(), d);
                }
                if let Some((w, h, px)) = img {
                    self.thumbs.insert(path.clone(), Thumb { w, h, px, bmp: None });
                    // keep the strip's cache small: drop the thumbnails farthest from here
                    if self.thumbs.len() > 400 {
                        let pos: HashMap<&PathBuf, usize> = self.list.iter().enumerate().map(|(i, p)| (p, i)).collect();
                        let idx = self.idx as i64;
                        let mut far: Vec<(i64, PathBuf)> = self.thumbs.keys().map(|p| (pos.get(p).map(|i| (*i as i64 - idx).abs()).unwrap_or(i64::MAX), p.clone())).collect();
                        far.sort_by(|a, b| b.0.cmp(&a.0));
                        for (_, p) in far.into_iter().take(100) {
                            self.thumbs.remove(&p);
                            self.thumb_asked.remove(&p);
                        }
                    }
                }
                if self.cur() == Some(&path) && self.fit && !self.pics.contains_key(&path) {
                    let st = self.stage_rect();
                    let fs = self.fit_scale(st);
                    self.zs.set(fs);
                }
                self.invalidate();
            }
            VMsg::Deleted { path, at, res } => match res {
                Ok(()) => {
                    post_bg(&self.main_bg, self.main.0 as isize, Msg::Refresh);
                    if self.list.is_empty() {
                        // nothing left to show: the main window offers the undo
                        post_bg(&self.main_bg, self.main.0 as isize, Msg::Toast { text: "Moved to the Recycle Bin: 1 item".into(), undo: true });
                        unsafe {
                            let _ = DestroyWindow(self.hwnd);
                        }
                        return;
                    }
                    self.show_toast("Moved to the Recycle Bin", Some((path, at)), false);
                }
                Err(e) => {
                    // it is still there: back into the list where it was
                    let i = at.min(self.list.len());
                    self.list.insert(i, path);
                    self.go(i);
                    self.show_toast(&crate::app::humanize(&e), None, true);
                }
            },
            VMsg::Restored { path, at, res } => match res {
                Ok(()) => {
                    post_bg(&self.main_bg, self.main.0 as isize, Msg::Refresh);
                    if path.exists() && !self.list.contains(&path) {
                        let i = at.min(self.list.len());
                        self.list.insert(i, path);
                        self.go(i);
                    }
                    self.toast = None;
                }
                Err(e) => self.show_toast(&crate::app::humanize(&e), None, true),
            },
            VMsg::Error(e) => self.show_toast(&crate::app::humanize(&e), None, true),
        }
    }

    /// Make the Direct2D bitmap of a decoded picture (on the window's target) and let the pixels go.
    fn upload(&mut self, path: &Path) {
        if self.gfx.target_id() == 0 || self.gfx.offscreen() {
            return;
        }
        let Some(pic) = self.pics.get_mut(path) else { return };
        if let Some((w, h, px)) = pic.px.take() {
            match self.gfx.make_bitmap(w, h, &px) {
                Some(b) => pic.bmp = Some(b),
                None => pic.err = Some(format!("The picture is too big to show ({w} × {h})")),
            }
        }
    }

    fn show_toast(&mut self, text: &str, undo: Option<(PathBuf, usize)>, err: bool) {
        self.toast = Some(VToast { text: text.to_string(), undo, err, at: Instant::now() });
        if err {
            self.last_error = Some(text.to_string());
        }
        self.timer(T_TOAST, 500);
        self.anim();
        self.invalidate();
    }

    // ------------------------------------------------------------------ geometry
    fn stage_rect(&self) -> Rect {
        if self.full {
            return Rect::new(0.0, 0.0, self.w, self.h);
        }
        let top = TITLE_H + BAR_H;
        let bottom = if self.strip { STRIP_H } else { 0.0 };
        Rect::new(0.0, top, self.w, (self.h - top - bottom).max(1.0))
    }

    fn rot_of(&self) -> u8 {
        self.cur().and_then(|p| self.rot.get(p)).copied().unwrap_or(0)
    }

    /// Upright size of the current picture before the view's own turn: decoded size, else the header's size (turned
    /// like the shell thumbnail when EXIF turns it), else the thumbnail's.
    fn base_dims(&self) -> Option<(f32, f32)> {
        let p = self.cur()?;
        if let Some(pic) = self.pics.get(p)
            && pic.err.is_none()
            && pic.w > 0
        {
            return Some((pic.w as f32, pic.h as f32));
        }
        let th = self.thumbs.get(p).map(|t| (t.w as f32, t.h as f32));
        if let Some((w, h)) = self.dims.get(p).map(|d| (d.0 as f32, d.1 as f32)) {
            if let Some((tw, thh)) = th
                && (tw > thh) != (w > h)
                && (tw - thh).abs() > 2.0
            {
                return Some((h, w));
            }
            return Some((w, h));
        }
        th
    }

    /// Size on screen (DIPs per picture pixel = 1) after the view's turn.
    fn dims(&self) -> Option<(f32, f32)> {
        let (w, h) = self.base_dims()?;
        if self.rot_of() % 2 == 1 { Some((h, w)) } else { Some((w, h)) }
    }

    /// Scale that fits the picture in the area, never above 100 % (a small picture keeps its real size).
    fn fit_scale(&self, st: Rect) -> f32 {
        let Some((w, h)) = self.dims() else { return 1.0 / self.k() };
        (st.w / w).min(st.h / h).min(1.0 / self.k()).max(1e-4)
    }

    fn min_scale(&self, st: Rect) -> f32 {
        self.fit_scale(st).min(0.05 / self.k())
    }

    fn max_scale(&self) -> f32 {
        32.0 / self.k()
    }

    /// Offsets kept so the picture never leaves an empty band on a side it overflows.
    fn clamp_off(&self, s: f32, ox: f32, oy: f32) -> (f32, f32) {
        let st = self.stage_rect();
        let Some((w, h)) = self.dims() else { return (0.0, 0.0) };
        let (dw, dh) = (w * s, h * s);
        let cx = if dw <= st.w { 0.0 } else { ox.clamp(-(dw - st.w) / 2.0, (dw - st.w) / 2.0) };
        let cy = if dh <= st.h { 0.0 } else { oy.clamp(-(dh - st.h) / 2.0, (dh - st.h) / 2.0) };
        (cx, cy)
    }

    /// The view shows this picture with more screen pixels than its screen-size copy has.
    fn needs_full(&self, p: &Pic) -> bool {
        p.bw > 0 && p.w as f32 * self.zs.target * self.k() > p.bw as f32 * 1.02
    }

    fn pannable(&self) -> bool {
        let st = self.stage_rect();
        let Some((w, h)) = self.dims() else { return false };
        !self.fit && (w * self.zs.value > st.w + 0.5 || h * self.zs.value > st.h + 0.5)
    }

    /// Zoom to `s` keeping the point (ax, ay) where it is.
    fn zoom_to(&mut self, s: f32, ax: f32, ay: f32, animate: bool) {
        let st = self.stage_rect();
        let s = s.clamp(self.min_scale(st), self.max_scale());
        let s0 = self.zs.target.max(1e-6);
        let (px, py) = (ax - st.cx(), ay - st.cy());
        let ox = px + (self.zx.target - px) * s / s0;
        let oy = py + (self.zy.target - py) * s / s0;
        let fs = self.fit_scale(st);
        self.fit = (s - fs).abs() < fs * 0.002;
        let (ox, oy) = if self.fit { (0.0, 0.0) } else { self.clamp_off(s, ox, oy) };
        if animate && !self.shooting {
            self.zs.to(s);
            self.zx.to(ox);
            self.zy.to(oy);
            self.anim();
        } else {
            self.zs.set(s);
            self.zx.set(ox);
            self.zy.set(oy);
        }
        self.invalidate();
    }

    fn zoom_by(&mut self, f: f32, ax: f32, ay: f32) {
        let s = self.zs.target * f;
        self.zoom_to(s, ax, ay, true);
    }

    fn zoom_center(&mut self, f: f32) {
        let st = self.stage_rect();
        self.zoom_by(f, st.cx(), st.cy());
    }

    fn to_fit(&mut self) {
        let st = self.stage_rect();
        let fs = self.fit_scale(st);
        self.zoom_to(fs, st.cx(), st.cy(), true);
        self.fit = true;
    }

    /// 100 % (one picture pixel = one screen pixel) around (ax, ay); a picture smaller than the window goes to 200 %.
    fn to_actual(&mut self, ax: f32, ay: f32) {
        let st = self.stage_rect();
        let fs = self.fit_scale(st);
        let one = 1.0 / self.k();
        let s = if one > fs * 1.01 { one } else { fs * 2.0 };
        self.zoom_to(s, ax, ay, true);
    }

    fn toggle_zoom(&mut self, ax: f32, ay: f32) {
        if self.fit {
            self.to_actual(ax, ay);
        } else {
            self.to_fit();
        }
    }

    fn rotate(&mut self, d: i32) {
        let Some(p) = self.cur().cloned() else { return };
        let r = self.rot_of() as i32;
        let from = r as f32 * 90.0;
        let nr = (r + d).rem_euclid(4) as u8;
        self.rot.insert(p, nr);
        // the turn moves (from the old angle to the new one) while the picture refits
        self.rot_anim = if self.shooting { None } else { Some((Instant::now(), from, from + d as f32 * 90.0)) };
        self.fit = true;
        let st = self.stage_rect();
        let fs = self.fit_scale(st);
        self.zs.to(fs);
        self.zx.to(0.0);
        self.zy.to(0.0);
        if self.shooting {
            self.zs.set(fs);
        }
        self.anim();
        self.invalidate();
    }

    fn strip_pitch() -> f32 {
        THUMB + GAP
    }

    /// Scroll of the filmstrip that shows the current picture in the middle (within the ends).
    fn strip_target(&self) -> f32 {
        let n = self.list.len() as f32;
        let total = n * Self::strip_pitch() - GAP;
        let room = self.w - 32.0;
        if total <= room {
            return 0.0;
        }
        (self.idx as f32 * Self::strip_pitch() + THUMB / 2.0 - room / 2.0).clamp(0.0, total - room)
    }

    // ------------------------------------------------------------------ actions
    fn delete_current(&mut self) {
        if self.list.is_empty() {
            return;
        }
        if self.ask_delete && !self.confirm {
            self.confirm = true;
            self.invalidate();
            return;
        }
        self.confirm = false;
        let at = self.idx;
        let path = self.list.remove(at);
        self.pics.remove(&path);
        if !self.list.is_empty() {
            // the next picture takes its place (the previous one at the end)
            self.go(at.min(self.list.len() - 1));
        }
        self.toast = None;
        let bg = self.bg.clone();
        let hw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let _c = fmcore::util::com();
            let res = fmcore::ops::delete(std::slice::from_ref(&path), false).map(|_| ());
            vpost(&bg, hw, VMsg::Deleted { path, at, res });
        });
        self.invalidate();
    }

    fn undo(&mut self) {
        let Some((path, at)) = self.toast.as_ref().and_then(|t| t.undo.clone()) else { return };
        self.toast = None;
        let bg = self.bg.clone();
        let hw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let _c = fmcore::util::com();
            let res = fmcore::journal::undo_last().map(|_| ());
            vpost(&bg, hw, VMsg::Restored { path, at, res });
        });
    }

    fn open_app(&mut self) {
        let Some(p) = self.cur().cloned() else { return };
        let bg = self.bg.clone();
        let hw = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let _c = fmcore::util::com();
            if let Err(e) = fmcore::shell::open_file(&p) {
                vpost(&bg, hw, VMsg::Error(e));
            }
        });
    }

    fn reveal(&mut self) {
        let Some(p) = self.cur().cloned() else { return };
        post_bg(&self.main_bg, self.main.0 as isize, Msg::ViewerReveal(p));
    }

    fn toggle_strip(&mut self) {
        self.strip = !self.strip;
        save_setting("filmstrip", json!(self.strip));
        let t = self.strip_target();
        self.strip_x.set(t);
        self.invalidate();
    }

    /// Apply the window frame as `create` does: dark title bar, the backdrop, and the frame drawn into the client area.
    unsafe fn reframe(&self) {
        unsafe {
            set_dark_frame(self.hwnd, self.t.dark);
            if let Some(b) = self.bd {
                let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &b as *const i32 as *const _, 4);
                let _ = DwmExtendFrameIntoClientArea(self.hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
            } else {
                let _ = DwmExtendFrameIntoClientArea(self.hwnd, &MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 });
            }
        }
    }

    fn toggle_full(&mut self) {
        self.tip = None;
        unsafe {
            let hw = self.hwnd;
            if !self.full {
                let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                let _ = GetWindowPlacement(hw, &mut wp);
                self.saved = Some(wp);
                let hm = MonitorFromWindow(hw, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(hm, &mut mi);
                let m = mi.rcMonitor;
                self.full = true;
                SetWindowLongPtrW(hw, GWL_STYLE, (WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN).0 as isize);
                let quiet = if self.test || self.offscreen { SWP_NOACTIVATE } else { SET_WINDOW_POS_FLAGS(0) };
                if self.offscreen {
                    // tests off every monitor: the monitor's size, but never onto the screen
                    let mut r = RECT::default();
                    let _ = GetWindowRect(hw, &mut r);
                    let _ = SetWindowPos(hw, Some(HWND_TOP), r.left, r.top, m.right - m.left, m.bottom - m.top, SWP_FRAMECHANGED | SWP_NOOWNERZORDER | quiet);
                } else {
                    let _ = SetWindowPos(hw, Some(HWND_TOP), m.left, m.top, m.right - m.left, m.bottom - m.top, SWP_FRAMECHANGED | SWP_NOOWNERZORDER | quiet);
                }
                self.last_move = Instant::now();
                self.timer(T_IDLE, 2600);
            } else {
                self.full = false;
                SetWindowLongPtrW(hw, GWL_STYLE, (normal_style() | WS_VISIBLE).0 as isize);
                if let Some(mut wp) = self.saved.take() {
                    if self.test || self.offscreen {
                        wp.showCmd = if wp.showCmd == SW_SHOWMAXIMIZED.0 as u32 { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNOACTIVATE.0 as u32 };
                    }
                    let _ = SetWindowPlacement(hw, &wp);
                }
                let _ = SetWindowPos(hw, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            // a style change drops DWM's dark frame, backdrop and glass margins: put them back, or the window's
            // colours come back wrong after full screen (Sajjad, 2026-11-26)
            self.reframe();
        }
        self.fit = true;
        self.invalidate();
    }

    fn close(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    fn app_name(&mut self) -> String {
        let ext = self.cur().map(|p| fmcore::util::ext_of(&p.to_string_lossy())).unwrap_or_default();
        if let Some(n) = self.apps.get(&ext) {
            return n.clone();
        }
        let n = fmcore::shell::default_app(&ext).0;
        self.apps.insert(ext, n.clone());
        n
    }

    fn tip_text(&mut self, h: VHit) -> Option<String> {
        let s = match h {
            VHit::Prev | VHit::ArrowL => "Previous (Left)".to_string(),
            VHit::Next | VHit::ArrowR => "Next (Right)".to_string(),
            VHit::ZoomOut => "Zoom out (Ctrl+Minus)".to_string(),
            VHit::ZoomIn => "Zoom in (Ctrl+Plus)".to_string(),
            VHit::Fit => if self.fit { "Actual size (Ctrl+1, double-click)".to_string() } else { "Fit to window (Ctrl+0, double-click)".to_string() },
            VHit::Rotate => "Rotate (Ctrl+R)".to_string(),
            VHit::Delete => "Delete (Delete)".to_string(),
            VHit::OpenApp => {
                let n = self.app_name();
                if n.is_empty() { "Open in the default app (Ctrl+E)".to_string() } else { format!("Open in {n} (Ctrl+E)") }
            }
            VHit::Reveal => "Show in folder".to_string(),
            VHit::Strip => if self.strip { "Hide filmstrip".to_string() } else { "Show filmstrip".to_string() },
            VHit::Full => if self.full { "Exit full screen (F11)".to_string() } else { "Full screen (F11)".to_string() },
            VHit::Min => "Minimize".to_string(),
            VHit::Max => if unsafe { IsZoomed(self.hwnd).as_bool() } { "Restore down".to_string() } else { "Maximize".to_string() },
            VHit::Close => "Close (Esc)".to_string(),
            VHit::Thumb(i) => self.list.get(i)?.file_name()?.to_string_lossy().into_owned(),
            _ => return None,
        };
        Some(s)
    }

    // ------------------------------------------------------------------ input
    fn hit_at(&self, x: f32, y: f32) -> VHit {
        for (r, h) in self.hits.iter().rev() {
            if r.has(x, y) {
                return *h;
            }
        }
        VHit::None
    }

    fn hit(&mut self, r: Rect, h: VHit) {
        self.hits.push((r, h));
    }

    fn hk(&self, h: VHit) -> f32 {
        if self.shooting {
            if self.shot_hover && self.hover == h { 1.0 } else { 0.0 }
        } else {
            self.hov_f.get(h)
        }
    }

    fn pk(&self, h: VHit) -> f32 {
        if self.shooting { 0.0 } else { self.press_f.get(h) }
    }

    fn set_hover(&mut self, h: VHit) {
        self.hover = h;
        let key = if self.nc_hover != VHit::None { Some(self.nc_hover) } else if matches!(h, VHit::None | VHit::Block | VHit::Stage | VHit::Image | VHit::StripBg | VHit::Title) { None } else { Some(h) };
        self.hov_f.only(key);
        // tooltip: hide now, show the new one after a short rest
        let th = if self.nc_hover != VHit::None { self.nc_hover } else { h };
        if self.tip.as_ref().map(|t| t.0) != Some(th) {
            self.tip = None;
            if key.is_some() {
                self.timer(T_TIP, 500);
            }
        }
        self.anim();
        self.invalidate();
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.mx = x;
        self.my = y;
        if self.full {
            self.last_move = Instant::now();
            self.timer(T_IDLE, 2600);
            self.invalidate();
        }
        if let Some((x0, y0, ox, oy)) = self.drag {
            if (x - x0).abs() + (y - y0).abs() > 3.0 {
                self.dragged = true;
            }
            if self.dragged {
                let (cx, cy) = self.clamp_off(self.zs.value, ox + x - x0, oy + y - y0);
                self.zx.set(cx);
                self.zy.set(cy);
                self.invalidate();
            }
            return;
        }
        let h = self.hit_at(x, y);
        if h != self.hover {
            self.set_hover(h);
        }
        let in_area = self.stage_rect().has(x, y) && !matches!(h, VHit::Thumb(_) | VHit::StripBg);
        self.area_f.only(in_area.then_some(0));
        self.anim();
        // (a test's synthetic mouse is not over the window: no leave tracking, or it would end the hover at once)
        if !self.tracking && !self.synthetic {
            self.tracking = true;
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
        }
    }

    fn mouse_leave(&mut self) {
        self.tracking = false;
        if self.drag.is_none() {
            self.hover = VHit::None;
            self.hov_f.only(None);
            self.area_f.only(None);
            self.tip = None;
            self.anim();
            self.invalidate();
        }
    }

    pub fn mouse_down(&mut self, x: f32, y: f32, dbl: bool) {
        self.tip = None;
        let h = self.hit_at(x, y);
        if self.confirm && !matches!(h, VHit::Yes | VHit::No) {
            return;
        }
        self.pressed = h;
        if !matches!(h, VHit::Image | VHit::Stage | VHit::None | VHit::Block | VHit::StripBg) {
            self.press_f.only(Some(h));
            self.anim();
        }
        match h {
            VHit::Image if dbl => {
                self.pressed = VHit::None;
                self.toggle_zoom(x, y);
            }
            VHit::Stage if dbl => {
                self.pressed = VHit::None;
                self.toggle_full();
            }
            VHit::Image if self.pannable() => {
                self.drag = Some((x, y, self.zx.value, self.zy.value));
                self.dragged = false;
                self.zx.set(self.zx.value);
                self.zy.set(self.zy.value);
                self.zs.set(self.zs.value);
            }
            _ => {}
        }
        self.invalidate();
    }

    pub fn mouse_up(&mut self, x: f32, y: f32) {
        let was_drag = self.drag.take().is_some() && self.dragged;
        self.dragged = false;
        let p = std::mem::replace(&mut self.pressed, VHit::None);
        self.press_f.only(None);
        self.anim();
        let h = self.hit_at(x, y);
        if !was_drag && h == p {
            self.click(h);
        }
        self.invalidate();
    }

    fn click(&mut self, h: VHit) {
        match h {
            VHit::Prev | VHit::ArrowL => self.step(-1),
            VHit::Next | VHit::ArrowR => self.step(1),
            VHit::ZoomIn => self.zoom_center(1.25),
            VHit::ZoomOut => self.zoom_center(0.8),
            VHit::Fit => {
                let st = self.stage_rect();
                self.toggle_zoom(st.cx(), st.cy());
            }
            VHit::Rotate => self.rotate(1),
            VHit::Delete => self.delete_current(),
            VHit::OpenApp => self.open_app(),
            VHit::Reveal => self.reveal(),
            VHit::Strip => self.toggle_strip(),
            VHit::Full => self.toggle_full(),
            VHit::Thumb(i) => self.go(i),
            VHit::Undo => self.undo(),
            VHit::ToastClose => self.toast = None,
            VHit::Yes => self.delete_current(),
            VHit::No => self.confirm = false,
            VHit::Min => unsafe {
                let _ = ShowWindow(self.hwnd, SW_MINIMIZE);
            },
            VHit::Max => unsafe {
                let _ = ShowWindow(self.hwnd, if IsZoomed(self.hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
            },
            VHit::Close => self.close(),
            _ => {}
        }
        self.invalidate();
    }

    /// Wheel: Ctrl zooms around the pointer; over the filmstrip it scrolls the strip; else one picture per notch.
    pub fn wheel(&mut self, x: f32, y: f32, d: f32, ctrl: bool) {
        if self.confirm {
            return;
        }
        if ctrl {
            self.zoom_by(1.2f32.powf(d / 120.0), x, y);
            return;
        }
        let h = self.hit_at(x, y);
        if matches!(h, VHit::Thumb(_) | VHit::StripBg) && self.strip && !self.full {
            let total = self.list.len() as f32 * Self::strip_pitch() - GAP;
            let room = self.w - 32.0;
            if total > room {
                let t = (self.strip_x.target - d / 120.0 * Self::strip_pitch() * 3.0).clamp(0.0, total - room);
                self.strip_x.to(t);
                self.anim();
            }
            return;
        }
        // a precision touchpad sends small steps: one picture per full notch
        self.wheel_acc += d;
        while self.wheel_acc >= 120.0 {
            self.wheel_acc -= 120.0;
            self.step(-1);
        }
        while self.wheel_acc <= -120.0 {
            self.wheel_acc += 120.0;
            self.step(1);
        }
    }

    pub fn key(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        if alt {
            return false;
        }
        self.tip = None;
        if self.confirm {
            match vk {
                0x0D | 0x2E => self.delete_current(),
                0x1B => self.confirm = false,
                _ => return true,
            }
            self.invalidate();
            return true;
        }
        let st = self.stage_rect();
        match vk {
            // Left / PageUp, Right / PageDown, Home / End
            0x25 | 0x21 => self.step(-1),
            0x27 | 0x22 => self.step(1),
            0x24 => self.go(0),
            0x23 => self.go(self.list.len().saturating_sub(1)),
            // Up / Down move a zoomed picture
            0x26 | 0x28 if self.pannable() => {
                let dy = if vk == 0x26 { 90.0 } else { -90.0 };
                let (cx, cy) = self.clamp_off(self.zs.target, self.zx.target, self.zy.target + dy);
                self.zx.to(cx);
                self.zy.to(cy);
                self.anim();
            }
            // + / - (main keys and keypad), Ctrl or not; Ctrl+0 fit, Ctrl+1 actual size
            0xBB | 0x6B => self.zoom_center(1.25),
            0xBD | 0x6D => self.zoom_center(0.8),
            0x30 | 0x60 if ctrl => self.to_fit(),
            0x31 | 0x61 if ctrl => self.to_actual(st.cx(), st.cy()),
            0x52 if ctrl || !shift => self.rotate(if shift { -1 } else { 1 }),
            0x52 => self.rotate(-1),
            0x45 if ctrl => self.open_app(),
            0x2E => self.delete_current(),
            0x7A => self.toggle_full(),
            0x57 if ctrl => self.close(),
            0x1B => {
                if self.full {
                    self.toggle_full();
                } else {
                    self.close();
                }
            }
            _ => return false,
        }
        self.invalidate();
        true
    }

    fn on_timer(&mut self, id: usize) {
        unsafe {
            match id {
                T_ANIM => {
                    let _ = KillTimer(Some(self.hwnd), T_ANIM);
                    self.invalidate();
                }
                T_TIP => {
                    let _ = KillTimer(Some(self.hwnd), T_TIP);
                    let h = if self.nc_hover != VHit::None { self.nc_hover } else { self.hover };
                    if self.pressed == VHit::None
                        && self.drag.is_none()
                        && let Some(s) = self.tip_text(h)
                    {
                        self.tip = Some((h, s, self.mx, self.my));
                        self.invalidate();
                    }
                }
                T_TOAST => {
                    match &self.toast {
                        Some(t) if t.at.elapsed() < Duration::from_secs(6) || self.hover == VHit::Undo || self.hover == VHit::ToastClose => {}
                        _ => {
                            self.toast = None;
                            let _ = KillTimer(Some(self.hwnd), T_TOAST);
                        }
                    }
                    self.invalidate();
                }
                T_IDLE | T_LOAD => {
                    let _ = KillTimer(Some(self.hwnd), id);
                    self.anim();
                    self.invalidate();
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------ painting
    pub fn paint(&mut self) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        let (pw, ph) = ((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32);
        self.gfx.dpi = unsafe { GetDpiForWindow(self.hwnd) } as f32;
        if !self.gfx.ensure(pw, ph) {
            return;
        }
        let k = self.k();
        let (ow, oh) = (self.w, self.h);
        self.w = pw as f32 / k;
        self.h = ph as f32 / k;
        if (ow - self.w).abs() > 0.5 || (oh - self.h).abs() > 0.5 {
            let t = self.strip_target();
            self.strip_x.set(t);
        }
        // a new render target (device lost, renderer change): the bitmaps were made for the old one
        let tid = self.gfx.target_id();
        if !self.gfx.offscreen() && tid != self.bmp_target {
            self.bmp_target = tid;
            let lost: Vec<PathBuf> = self.pics.iter().filter(|(_, p)| p.bmp.is_some()).map(|(k, _)| k.clone()).collect();
            for p in lost {
                self.pics.remove(&p);
            }
            self.request();
        }
        if !self.gfx.offscreen() {
            let paths: Vec<PathBuf> = self.pics.iter().filter(|(_, p)| p.px.is_some()).map(|(k, _)| k.clone()).collect();
            for p in paths {
                self.upload(&p);
            }
        }
        self.hits.clear();
        let st = self.stage_rect();
        let mut moving = false;
        if self.fit && self.drag.is_none() {
            let fs = self.fit_scale(st);
            if self.zs.settled() {
                self.zs.set(fs);
                self.zx.set(0.0);
                self.zy.set(0.0);
            } else {
                self.zs.to(fs);
                self.zx.to(0.0);
                self.zy.to(0.0);
            }
        }
        if self.shooting {
            for f in [&mut self.zs, &mut self.zx, &mut self.zy, &mut self.strip_x] {
                let t = f.target;
                f.set(t);
            }
        }
        moving |= self.zs.step();
        moving |= self.zx.step();
        moving |= self.zy.step();
        moving |= self.strip_x.step();
        let t = self.t.clone();
        self.gfx.begin();
        if self.mica && !self.gfx.offscreen() {
            self.gfx.clear(C(0.0, 0.0, 0.0, 0.0));
        } else {
            self.gfx.clear(t.mica_a);
        }
        self.gfx.fill(st, self.stage);
        moving |= self.paint_picture(st);
        self.paint_arrows(st);
        if !self.full {
            if self.strip {
                self.paint_strip(&t);
            }
            self.paint_title(&t);
            self.paint_bar(&t);
        } else {
            self.paint_full_chrome();
        }
        if self.confirm {
            self.paint_confirm(&t, st);
        }
        moving |= self.paint_toast(&t, st);
        self.paint_tip(&t);
        self.gfx.end();
        moving |= self.hov_f.busy() | self.press_f.busy() | self.area_f.busy();
        if moving && !self.shooting {
            self.anim();
        }
    }

    /// The picture (or its thumbnail while it decodes, or what went wrong). True while something still moves.
    fn paint_picture(&mut self, st: Rect) -> bool {
        let Some(path) = self.cur().cloned() else { return false };
        self.hit(st, VHit::Stage);
        let mut moving = false;
        let s = self.zs.value;
        let (cx, cy) = (st.cx() + self.zx.value, st.cy() + self.zy.value);
        let rot = self.rot_of();
        let mut angle = rot as f32 * 90.0;
        if let Some((at, from, to)) = self.rot_anim {
            let p = crate::motion::since(at, crate::motion::Spring::Snappy, self.shooting);
            angle = from + (to - from) * p;
            if p >= 1.0 {
                self.rot_anim = None;
            } else {
                moving = true;
            }
        }
        let (bw, bh) = match self.base_dims() {
            Some(d) => d,
            None => (0.0, 0.0),
        };
        let dst = Rect::new(cx - bw * s / 2.0, cy - bh * s / 2.0, bw * s, bh * s);
        // the visible picture (turned) for hits: double-click zooms there, a drag moves it
        let (dw, dh) = if rot % 2 == 1 { (bh * s, bw * s) } else { (bw * s, bh * s) };
        let shown = Rect::new(cx - dw / 2.0, cy - dh / 2.0, dw, dh);
        // zoomed past the screen-size copy: the whole picture is decoded (shown when ready)
        if !self.shooting && self.pics.get(&path).is_some_and(|p| !p.full && p.err.is_none() && self.needs_full(p)) {
            let j = (path.clone(), true);
            let asked = {
                let (m, _) = &*self.dq;
                let q = m.lock().unwrap();
                q.want.contains(&j) || q.busy.contains(&j) || q.done.contains(&j)
            };
            if !asked {
                self.request();
            }
        }
        let pic = self.pics.get(&path);
        let err = pic.and_then(|p| p.err.clone());
        let bmp = if self.shooting { self.shot_bmp.clone() } else { pic.and_then(|p| p.bmp.clone()) };
        let ready_at = pic.map(|p| p.at);
        self.gfx.clip(st);
        if let Some(e) = err {
            self.gfx.unclip();
            self.paint_error(st, &e);
            return moving;
        }
        // fade in over the thumbnail when the picture came after it was asked for
        let fade = match ready_at {
            Some(at) if at > self.shown_at && bmp.is_some() => crate::motion::fade_in(crate::motion::lin(at, 160.0)),
            _ => 1.0,
        };
        let fade = if self.shooting { 1.0 } else { fade };
        if fade < 1.0 {
            moving = true;
        }
        if dst.w > 0.0 {
            if angle != 0.0 {
                self.gfx.push_rotate(cx, cy, angle);
            }
            if fade < 1.0 || bmp.is_none() {
                self.paint_thumb_in(&path, dst);
            }
            if let Some(b) = &bmp {
                // past 400 % the pixels show as sharp squares (the way pixel peeping should look)
                self.gfx.photo(b, dst, fade, s * self.k() < 4.0);
            }
            if angle != 0.0 {
                self.gfx.pop_xf();
            }
            self.hit(Rect::new(shown.x.max(st.x), shown.y.max(st.y), (shown.r().min(st.r()) - shown.x.max(st.x)).max(0.0), (shown.b().min(st.b()) - shown.y.max(st.y)).max(0.0)), VHit::Image);
        }
        self.gfx.unclip();
        if bmp.is_none() && !self.thumbs.contains_key(&path) && self.shown_at.elapsed() > Duration::from_millis(300) {
            self.gfx.text("Loading…", st, Font::Ui, 13.0, 400, Align::Center, INK2);
        }
        moving
    }

    /// The current picture's thumbnail stretched into `dst` (the first look while the real one decodes).
    fn paint_thumb_in(&mut self, path: &Path, dst: Rect) {
        if let Some(b) = self.thumb_bmp(path) {
            self.gfx.bitmap(&b, dst, 1.0);
        }
    }

    fn thumb_bmp(&mut self, path: &Path) -> Option<ID2D1Bitmap> {
        let tid = self.gfx.target_id();
        let th = self.thumbs.get_mut(path)?;
        if let Some((t, b)) = &th.bmp
            && *t == tid
        {
            return Some(b.clone());
        }
        let b = self.gfx.make_bitmap(th.w, th.h, &th.px)?;
        th.bmp = Some((tid, b.clone()));
        Some(b)
    }

    fn paint_error(&mut self, st: Rect, e: &str) {
        let cy = st.cy();
        self.gfx.glyph(0xE91B, Rect::new(st.cx() - 24.0, cy - 86.0, 48.0, 48.0), 36.0, INK2);
        self.gfx.text("Can't show this picture", Rect::new(st.x + 20.0, cy - 30.0, st.w - 40.0, 24.0), Font::Ui, 15.0, 600, Align::Center, INK);
        let msg = crate::app::humanize(e);
        self.gfx.para(&msg, Rect::new(st.cx() - 220.0, cy - 2.0, 440.0, 40.0), Font::Ui, 12.5, 400, Align::Center, INK2, 18.0, false, 2);
        let n = self.app_name();
        let label = if n.is_empty() { "Open in the default app".to_string() } else { format!("Open in {n}") };
        let bw = self.gfx.measure(&label, Font::Ui, 13.0, 400) + 48.0;
        let r = Rect::new(st.cx() - bw / 2.0, cy + 46.0, bw, 34.0);
        let hk = self.hk(VHit::OpenApp);
        self.gfx.fill_round(r, R_CTRL, C(1.0, 1.0, 1.0, 0.10 + 0.08 * hk));
        self.gfx.stroke_round(r, R_CTRL, C(1.0, 1.0, 1.0, 0.16), 1.0);
        self.gfx.glyph(0xE8A7, Rect::new(r.x + 14.0, r.y, 14.0, r.h), 13.0, INK);
        self.gfx.text(&label, Rect::new(r.x + 34.0, r.y, r.w - 40.0, r.h), Font::Ui, 13.0, 400, Align::Left, INK);
        self.hit(r, VHit::OpenApp);
    }

    /// Previous / next arrows at the sides of the picture area, shown while the pointer is over it.
    fn paint_arrows(&mut self, st: Rect) {
        let a = if self.shooting { if self.shot_hover { 1.0 } else { 0.0 } } else { self.area_f.get(0) };
        if a <= 0.01 || self.confirm || self.list.len() < 2 || st.w < 200.0 {
            return;
        }
        for (h, glyph, x, on) in [(VHit::ArrowL, 0xE76Bu32, st.x + 14.0, self.idx > 0), (VHit::ArrowR, 0xE76C, st.r() - 14.0 - 44.0, self.idx + 1 < self.list.len())] {
            if !on {
                continue;
            }
            let r = Rect::new(x, st.cy() - 22.0, 44.0, 44.0);
            let (hk, pk) = (self.hk(h), self.pk(h));
            self.gfx.ellipse(r.cx(), r.cy(), 22.0, C(0.06, 0.07, 0.09, (0.50 + 0.22 * hk - 0.08 * pk) * a), true, 1.0);
            self.gfx.ellipse(r.cx(), r.cy(), 22.0, C(1.0, 1.0, 1.0, 0.14 * a), false, 1.0);
            self.gfx.glyph(glyph, r, 16.0, INK.a(a));
            self.hit(r, h);
        }
    }

    fn icon_btn(&mut self, r: Rect, glyph: u32, enabled: bool, h: VHit, on: bool, t: &Theme) {
        let (hk, pk) = if enabled { (self.hk(h), self.pk(h)) } else { (0.0, 0.0) };
        if on {
            self.gfx.fill_round(r, R_CTRL, t.sel);
        }
        self.gfx.fill_round(r, R_CTRL, t.hover.a(t.hover.3 * hk));
        self.gfx.fill_round(r, R_CTRL, t.press.a(t.press.3 * pk * 0.8));
        let fg = if !enabled { t.text_dis } else if on { t.accent_text } else { t.text };
        if glyph != 0 {
            self.gfx.glyph(glyph, r, 16.0, fg);
        }
        if enabled {
            self.hit(r, h);
        }
    }

    fn paint_title(&mut self, t: &Theme) {
        self.gfx.fill(Rect::new(0.0, 0.0, self.w, TITLE_H), t.mica_a.a(self.pa));
        self.hit(Rect::new(0.0, 0.0, self.w, TITLE_H), VHit::Title);
        let caps_x = self.w - CAP_W * 3.0;
        crate::ficons::draw(&self.gfx, t, crate::ficons::Kind::Img, Rect::new(14.0, TITLE_H / 2.0 - 8.0, 16.0, 16.0));
        let name = self.cur().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let count = if self.list.len() > 1 { format!("{} / {}", self.idx + 1, self.list.len()) } else { String::new() };
        let cw = if count.is_empty() { 0.0 } else { self.gfx.measure(&count, Font::Ui, 12.0, 400) + 2.0 };
        let x = 40.0;
        let room = (caps_x - 16.0 - x - if cw > 0.0 { cw + 14.0 } else { 0.0 }).max(20.0);
        let nw = (self.gfx.measure(&name, Font::Ui, 12.5, 400) + 2.0).min(room);
        self.gfx.text_mid(&name, Rect::new(x, 0.0, nw, TITLE_H), Font::Ui, 12.5, 400, Align::Left, t.text);
        if cw > 0.0 {
            self.gfx.text_mid(&count, Rect::new(x + nw + 14.0, 0.0, cw, TITLE_H), Font::Ui, 12.0, 400, Align::Left, t.text2);
        }
        // caption buttons as the main window draws them (Windows 11 sizes and colours)
        let maxed = unsafe { IsZoomed(self.hwnd).as_bool() };
        let active = unsafe { GetForegroundWindow() } == self.hwnd || self.shooting || self.offscreen || self.test;
        let caps = [(VHit::Min, 0xE921u32), (VHit::Max, if maxed { 0xE923 } else { 0xE922 }), (VHit::Close, 0xE8BB)];
        for (i, (h, gl)) in caps.iter().enumerate() {
            let r = Rect::new(caps_x + i as f32 * CAP_W, 0.0, CAP_W, TITLE_H);
            let hk = self.hk(*h).max(if self.nc_hover == *h && !self.shooting { self.hov_f.get(*h) } else { 0.0 });
            let down = self.nc_down == *h && self.nc_hover == *h;
            let mut fg = if active { t.text } else { t.text.a(0.45) };
            if *h == VHit::Close {
                self.gfx.fill(r, C::hex(0xC42B1C).a(if down { 0.9 } else { 1.0 } * hk));
                fg = fg.mix(C::hex(0xFFFFFF), hk);
            } else {
                let (hov, prs) = if t.dark { (C(1.0, 1.0, 1.0, 0.0605), C(1.0, 1.0, 1.0, 0.0419)) } else { (C(0.0, 0.0, 0.0, 0.0373), C(0.0, 0.0, 0.0, 0.0241)) };
                self.gfx.fill(r, if down { prs } else { hov.a(hov.3 * hk) });
            }
            self.gfx.text(&char::from_u32(*gl).unwrap().to_string(), r, Font::Icon, 10.0, 400, Align::Center, fg);
            self.hit(r, *h);
        }
    }

    /// The command bar: icon buttons in the middle (each with a tooltip); the zoom value between the zoom buttons.
    fn paint_bar(&mut self, t: &Theme) {
        let bar = Rect::new(0.0, TITLE_H, self.w, BAR_H);
        self.gfx.fill(bar, t.bar.a(self.pa));
        self.gfx.fill(Rect::new(0.0, bar.b() - 1.0, self.w, 1.0), t.divider);
        let n = self.list.len();
        let st = self.stage_rect();
        let s = self.zs.target;
        let has = n > 0 && self.dims().is_some() && self.cur().and_then(|p| self.pics.get(p)).is_none_or(|p| p.err.is_none());
        let any = n > 0;
        let pct = format!("{}%", (s * self.k() * 100.0).round());
        #[derive(Clone, Copy)]
        enum It {
            B(VHit, u32, bool, bool),
            Sep,
            Pct,
        }
        let full_set = [
            It::B(VHit::Prev, 0xE76B, self.idx > 0, false),
            It::B(VHit::Next, 0xE76C, self.idx + 1 < n, false),
            It::Sep,
            It::B(VHit::ZoomOut, 0xE71F, has && s > self.min_scale(st) * 1.001, false),
            It::Pct,
            It::B(VHit::ZoomIn, 0xE8A3, has && s < self.max_scale() * 0.999, false),
            It::B(VHit::Fit, if self.fit { 0 } else { 0xE9A6 }, has, false),
            It::Sep,
            It::B(VHit::Rotate, 0xE7AD, has, false),
            It::B(VHit::Delete, 0xE74D, any, false),
            It::Sep,
            It::B(VHit::OpenApp, 0xE8A7, any, false),
            It::B(VHit::Reveal, 0xE838, any, false),
            It::Sep,
            It::B(VHit::Strip, 0, true, self.strip),
            It::B(VHit::Full, 0xE740, true, false),
        ];
        let width = |items: &[It]| -> f32 { items.iter().map(|i| match i { It::B(..) => 38.0, It::Sep => 13.0, It::Pct => 56.0 }).sum::<f32>() - 2.0 };
        let mut items: Vec<It> = full_set.to_vec();
        if width(&items) > self.w - 24.0 {
            // narrow: no separators and no zoom value
            items.retain(|i| matches!(i, It::B(..)));
        }
        if width(&items) > self.w - 24.0 {
            // narrower: previous / next stay as the arrows on the picture
            items.retain(|i| !matches!(i, It::B(VHit::Prev | VHit::Next, ..)));
        }
        let mut x = ((self.w - width(&items)) / 2.0).max(8.0);
        let y = bar.y + (BAR_H - 32.0) / 2.0;
        for it in items {
            match it {
                It::B(h, g, en, on) => {
                    let r = Rect::new(x, y, 36.0, 32.0);
                    self.icon_btn(r, g, en, h, on, t);
                    let fg = if !en { t.text_dis } else if on { t.accent_text } else { t.text };
                    if h == VHit::Fit && g == 0 {
                        // actual size: "1:1"
                        self.gfx.text("1:1", r, Font::Ui, 12.0, 600, Align::Center, fg);
                    }
                    if h == VHit::Strip {
                        // a filmstrip: the picture above, three frames below
                        let (cx, cy) = (r.cx(), r.cy());
                        self.gfx.stroke_round(Rect::new(cx - 8.0, cy - 7.5, 16.0, 9.0), 2.0, fg, 1.2);
                        for k in 0..3 {
                            self.gfx.fill_round(Rect::new(cx - 8.0 + k as f32 * 6.0, cy + 3.5, 4.0, 4.0), 1.0, fg);
                        }
                    }
                    x += 38.0;
                }
                It::Sep => {
                    self.gfx.fill(Rect::new(x + 5.0, bar.cy() - 10.0, 1.0, 20.0), t.divider.mix(t.text3, 0.25));
                    x += 13.0;
                }
                It::Pct => {
                    let label = if has { pct.clone() } else { String::new() };
                    self.gfx.text(&label, Rect::new(x, y, 54.0, 32.0), Font::Ui, 12.0, 400, Align::Center, t.text2);
                    x += 56.0;
                }
            }
        }
    }

    fn paint_strip(&mut self, t: &Theme) {
        let band = Rect::new(0.0, self.h - STRIP_H, self.w, STRIP_H);
        self.gfx.fill(band, t.bar.a(self.pa));
        self.gfx.fill(Rect::new(0.0, band.y, self.w, 1.0), t.divider);
        self.hit(band, VHit::StripBg);
        let n = self.list.len();
        if n == 0 {
            return;
        }
        let pitch = Self::strip_pitch();
        let total = n as f32 * pitch - GAP;
        let room = self.w - 32.0;
        let x0 = if total <= room { (self.w - total) / 2.0 } else { 16.0 - self.strip_x.value };
        let first = (((-x0) / pitch).floor().max(0.0)) as usize;
        let last = ((((self.w - x0) / pitch).ceil()) as usize).min(n);
        let y = band.y + (STRIP_H - THUMB) / 2.0;
        self.gfx.clip(Rect::new(0.0, band.y + 1.0, self.w, band.h - 1.0));
        for i in first..last {
            let r = Rect::new(x0 + i as f32 * pitch, y, THUMB, THUMB);
            let h = VHit::Thumb(i);
            let hk = self.hk(h);
            let cur = i == self.idx;
            let path = self.list[i].clone();
            match self.thumb_bmp(&path) {
                Some(b) => {
                    let th = &self.thumbs[&path];
                    // square crop from the middle (cover)
                    let side = th.w.min(th.h) as f32;
                    let src = Rect::new((th.w as f32 - side) / 2.0, (th.h as f32 - side) / 2.0, side, side);
                    self.gfx.bitmap_round(&b, r, src, R_SMALL);
                }
                None => {
                    self.gfx.fill_round(r, R_SMALL, t.layer2);
                    self.gfx.glyph(0xE91B, r, 16.0, t.text3);
                }
            }
            if !cur {
                // the others sit back a little and come forward on hover
                self.gfx.fill_round(r, R_SMALL, t.bar.a(if t.dark { 0.38 } else { 0.28 } * (1.0 - hk)));
                self.gfx.stroke_round(r, R_SMALL, t.stroke, 1.0);
            } else {
                self.gfx.stroke_round(r.inset(-3.0, -3.0), R_SMALL + 3.0, t.accent, 2.0);
            }
            self.hit(r, h);
        }
        self.gfx.unclip();
        // thumbnails for what is shown, the current one first
        for i in (first..last).rev() {
            if i != self.idx {
                self.ask_thumb(i);
            }
        }
        self.ask_thumb(self.idx);
    }

    /// Full screen: only the picture; the name, counter and the way out show for a moment when the pointer moves.
    fn paint_full_chrome(&mut self) {
        let k = if self.shooting {
            1.0
        } else {
            let idle = self.last_move.elapsed().as_secs_f32();
            if idle < 2.5 { 1.0 } else { (1.0 - (idle - 2.5) / 0.3).clamp(0.0, 1.0) }
        };
        if k > 0.0 && k < 1.0 {
            self.anim();
        }
        if k <= 0.0 {
            return;
        }
        let bar = Rect::new(0.0, 0.0, self.w, 52.0);
        self.gfx.opacity_layer(bar, k);
        self.gfx.fill(bar, C(0.0, 0.0, 0.0, 0.42));
        let name = self.cur().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let count = format!("{} / {}", self.idx + 1, self.list.len());
        let cw = self.gfx.measure(&count, Font::Ui, 12.0, 400) + 2.0;
        let nw = (self.gfx.measure(&name, Font::Ui, 13.0, 400) + 2.0).min(self.w - 120.0 - cw);
        self.gfx.text_mid(&name, Rect::new(18.0, 0.0, nw, 52.0), Font::Ui, 13.0, 400, Align::Left, INK);
        self.gfx.text_mid(&count, Rect::new(18.0 + nw + 14.0, 0.0, cw, 52.0), Font::Ui, 12.0, 400, Align::Left, INK2);
        let r = Rect::new(self.w - 54.0, 8.0, 40.0, 36.0);
        let hk = self.hk(VHit::Full);
        self.gfx.fill_round(r, R_CTRL, C(1.0, 1.0, 1.0, 0.08 + 0.10 * hk));
        self.gfx.glyph(0xE73F, r, 15.0, INK);
        self.hit(r, VHit::Full);
        self.gfx.pop_layer();
    }

    fn paint_confirm(&mut self, t: &Theme, st: Rect) {
        self.gfx.fill(Rect::new(0.0, 0.0, self.w, self.h), C(0.0, 0.0, 0.0, 0.40));
        self.hit(Rect::new(0.0, 0.0, self.w, self.h), VHit::Block);
        let w = 380.0f32.min(self.w - 32.0);
        let r = Rect::new(st.cx() - w / 2.0, st.cy() - 80.0, w, 160.0);
        self.gfx.shadow(r, R_CARD, 18.0, 30.0, if t.dark { 0.55 } else { 0.24 });
        self.gfx.fill_round(r, R_CARD, t.card);
        self.gfx.stroke_round(r, R_CARD, t.stroke, 1.0);
        self.gfx.text("Delete this picture?", Rect::new(r.x + 22.0, r.y + 18.0, r.w - 44.0, 24.0), Font::Ui, 15.0, 600, Align::Left, t.text);
        let name = self.cur().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.gfx.text(&name, Rect::new(r.x + 22.0, r.y + 46.0, r.w - 44.0, 20.0), Font::Ui, 12.5, 600, Align::Left, t.text2);
        self.gfx.text("It goes to the Recycle Bin.", Rect::new(r.x + 22.0, r.y + 66.0, r.w - 44.0, 20.0), Font::Ui, 12.5, 400, Align::Left, t.text2);
        let bw = (r.w - 44.0 - 8.0) / 2.0;
        for (i, (label, h)) in [("Delete", VHit::Yes), ("Cancel", VHit::No)].iter().enumerate() {
            let b = Rect::new(r.x + 22.0 + i as f32 * (bw + 8.0), r.b() - 52.0, bw, 34.0);
            let hk = self.hk(*h);
            if i == 0 {
                let red = if t.dark { C::hex(0xD13438) } else { C::hex(0xC42B1C) };
                self.gfx.fill_round(b, R_CTRL, red.mix(C::hex(0xFFFFFF), 0.10 * hk));
                self.gfx.text(label, b, Font::Ui, 13.0, 400, Align::Center, C::hex(0xFFFFFF));
            } else {
                self.gfx.fill_round(b, R_CTRL, t.field.mix(t.layer2, hk));
                self.gfx.stroke_round(b, R_CTRL, t.field_border, 1.0);
                self.gfx.text(label, b, Font::Ui, 13.0, 400, Align::Center, t.text);
            }
            self.hit(b, *h);
        }
    }

    fn paint_toast(&mut self, t: &Theme, st: Rect) -> bool {
        let Some(to) = self.toast.as_ref() else { return false };
        let (text, undo, err, at) = (to.text.clone(), to.undo.is_some(), to.err, to.at);
        let k = crate::motion::since(at, crate::motion::Spring::Bouncy, self.shooting);
        let tw = self.gfx.measure(&text, Font::Ui, 13.0, 400).min(self.w - 200.0).max(40.0);
        let uw = if undo { self.gfx.measure("Undo", Font::Ui, 13.0, 400) + 40.0 } else { 0.0 };
        let w = 16.0 + 16.0 + 10.0 + tw + 12.0 + uw + if undo { 8.0 } else { 0.0 } + 28.0 + 10.0;
        let r = Rect::new(st.cx() - w / 2.0, st.b() - 64.0 + (1.0 - k) * 12.0, w, 46.0);
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), k.clamp(0.05, 1.0));
        self.gfx.shadow(r, R_CARD, 8.0, 18.0, if t.dark { 0.45 } else { 0.18 });
        self.gfx.fill_round(r, R_CARD, t.card);
        self.gfx.stroke_round(r, R_CARD, t.stroke, 1.0);
        self.hit(r, VHit::Block);
        self.gfx.glyph(if err { 0xEA39 } else { 0xE73E }, Rect::new(r.x + 16.0, r.y, 16.0, r.h), 14.0, if err { t.danger } else { t.ok });
        self.gfx.text(&text, Rect::new(r.x + 42.0, r.y, tw + 2.0, r.h), Font::Ui, 13.0, 400, Align::Left, t.text);
        let mut x = r.x + 42.0 + tw + 12.0;
        if undo {
            let b = Rect::new(x, r.y + 7.0, uw, 32.0);
            let hk = self.hk(VHit::Undo);
            self.gfx.fill_round(b, R_CTRL, t.field.mix(t.layer2, hk));
            self.gfx.stroke_round(b, R_CTRL, t.field_border, 1.0);
            self.gfx.glyph(0xE7A7, Rect::new(b.x + 10.0, b.y, 14.0, b.h), 12.0, t.text);
            self.gfx.text("Undo", Rect::new(b.x + 28.0, b.y, b.w - 30.0, b.h), Font::Ui, 13.0, 400, Align::Left, t.text);
            self.hit(b, VHit::Undo);
            x += uw + 8.0;
        }
        let c = Rect::new(x, r.y + 9.0, 28.0, 28.0);
        let hk = self.hk(VHit::ToastClose);
        self.gfx.fill_round(c, R_SMALL, t.hover.a(t.hover.3 * hk));
        self.gfx.glyph(0xE711, c, 10.0, t.text2);
        self.hit(c, VHit::ToastClose);
        self.gfx.pop_layer();
        k != 1.0
    }

    fn paint_tip(&mut self, t: &Theme) {
        let Some((_, text, x, y)) = self.tip.clone() else { return };
        let tw = self.gfx.measure(&text, Font::Ui, 12.0, 400).min(self.w - 40.0) + 20.0;
        let mut r = Rect::new(x - 8.0, y + 22.0, tw, 28.0);
        if r.b() > self.h - 4.0 {
            r.y = y - 8.0 - r.h;
        }
        r.x = r.x.clamp(4.0, (self.w - r.w - 4.0).max(4.0));
        self.gfx.shadow(r, R_SMALL, 6.0, 12.0, if t.dark { 0.4 } else { 0.14 });
        self.gfx.fill_round(r, R_SMALL, t.card);
        self.gfx.stroke_round(r, R_SMALL, t.stroke, 1.0);
        self.gfx.text_mid(&text, r.inset(10.0, 0.0), Font::Ui, 12.0, 400, Align::Left, t.text);
    }

    // ------------------------------------------------------------------ tests (`wfm gui view-...`)
    /// Draw a frame into a PNG (offscreen software target; the picture is decoded again for it, no bigger than 4096).
    pub fn shot(&mut self, path: &str, hover: bool) -> Result<(), String> {
        self.take_bg();
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
            self.shot_bmp = None;
            if let Some(p) = self.cur().cloned()
                && self.pics.get(&p).is_some_and(|x| x.err.is_none())
                && let Ok(d) = fmcore::picture::decode(&p, 4096, MAX_PX, true)
            {
                self.shot_bmp = self.gfx.make_bitmap(d.width, d.height, &d.bgra);
            }
            self.shooting = true;
            self.shot_hover = hover;
            self.paint();
            self.shooting = false;
            self.shot_hover = false;
            self.shot_bmp = None;
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
            fmcore::thumb::save_png(&fmcore::thumb::Image { width: w as u32, height: h as u32, bgra: px }, Path::new(path))
        }
    }

    pub fn state(&self) -> serde_json::Value {
        let cur = self.cur();
        let pic = cur.and_then(|p| self.pics.get(p));
        let mut title = [0u16; 512];
        let n = unsafe { GetWindowTextW(self.hwnd, &mut title) } as usize;
        let pending = {
            let (m, _) = &*self.dq;
            let q = m.lock().unwrap();
            q.want.len() + q.busy.len()
        };
        json!({
            "open": true,
            "hwnd": self.hwnd.0 as isize,
            "title": String::from_utf16_lossy(&title[..n]),
            "file": cur.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
            "path": cur.map(|p| p.display().to_string()),
            "index": self.idx + 1,
            "count": self.list.len(),
            "list": self.list.iter().take(40).map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>(),
            "zoom_pct": (self.zs.target * self.k() * 1000.0).round() / 10.0,
            "fit": self.fit,
            "offset": [self.zx.target.round(), self.zy.target.round()],
            "rotation": self.rot_of() as u32 * 90,
            "fullscreen": self.full,
            "filmstrip": self.strip,
            "loaded": pic.is_some_and(|p| p.bmp.is_some()),
            "approx": pic.is_some_and(|p| p.approx),
            "error": pic.and_then(|p| p.err.clone()),
            "image": pic.map(|p| [p.w, p.h]),
            "bitmap_width": pic.map(|p| p.bw),
            "full_resolution": pic.map(|p| p.full),
            "decode_ms": pic.map(|p| (p.ms * 10.0).round() / 10.0),
            "first_picture_ms": self.first_ms.map(|m| m.round()),
            "ready": self.pics.iter().filter(|(_, p)| p.bmp.is_some()).filter_map(|(k, _)| k.file_name().map(|n| n.to_string_lossy().into_owned())).collect::<Vec<_>>(),
            "pending_decodes": pending,
            "thumbs": self.thumbs.len(),
            "strip_scroll": self.strip_x.target.round(),
            "hover": format!("{:?}", self.hover),
            "tip": self.tip.as_ref().map(|t| t.1.clone()),
            "toast": self.toast.as_ref().map(|t| t.text.clone()),
            "confirm": self.confirm,
            "theme": if self.t.dark { "dark" } else { "light" },
            "mica": self.mica,
            "renderer": self.gfx.renderer,
            "size": [self.w.round(), self.h.round()],
            "dpi": self.gfx.dpi,
            "last_error": self.last_error,
        })
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        crate::app::trace(&format!("viewer dropped: {} pictures, {} thumbnails", self.pics.len(), self.thumbs.len()));
    }
}

/// Viewer state for `wfm gui state`.
pub fn state() -> serde_json::Value {
    match get() {
        Some(v) => v.state(),
        None => json!({"open": false}),
    }
}

/// `wfm gui view-...` commands.
pub fn ipc(a: &mut App, cmd: &str, arg: &str) -> String {
    let err = |e: &str| json!({"ok": false, "error": e}).to_string();
    if cmd == "view-image" {
        let p = PathBuf::from(arg.trim().trim_matches('"'));
        if !p.is_file() {
            return err("not found");
        }
        let p = fmcore::util::abs(&p);
        let (list, idx) = a.picture_list(&p);
        open(a, list, idx);
        return match get() {
            Some(v) => json!({"ok": true, "index": v.idx + 1, "count": v.list.len()}).to_string(),
            None => err(a.last_error.as_deref().unwrap_or("the viewer did not open")),
        };
    }
    let Some(v) = get() else { return err("the viewer is not open (gui view-image <path>)") };
    // results of finished background work first, so the command sees the latest state
    v.take_bg();
    v.synthetic = true;
    let r = ipc_view(v, cmd, arg);
    if let Some(v) = get() {
        v.synthetic = false;
        v.invalidate();
    }
    r
}

fn ipc_view(v: &mut Viewer, cmd: &str, arg: &str) -> String {
    let err = |e: &str| json!({"ok": false, "error": e}).to_string();
    let num = |s: &str| s.parse::<f32>().ok();
    let mut it = arg.split_whitespace();
    let r = match cmd {
        "view-state" => return v.state().to_string(),
        "view-key" => {
            let Some((vk, c, s, al)) = crate::input::parse_keys(arg) else { return err("view-key: e.g. right, left, home, end, ctrl+plus, f11, esc, delete") };
            v.key(vk, c, s, al);
            json!({"ok": true})
        }
        // real key messages through the window procedure (they run after this reply)
        "view-post" => {
            let Some((vk, _, _, _)) = crate::input::parse_keys(arg) else { return err("view-post KEY") };
            unsafe {
                let _ = PostMessageW(Some(v.hwnd), WM_KEYDOWN, WPARAM(vk as usize), LPARAM(crate::app::TEST_KEY));
                let _ = PostMessageW(Some(v.hwnd), WM_KEYUP, WPARAM(vk as usize), LPARAM(crate::app::TEST_KEY));
            }
            json!({"ok": true})
        }
        "view-click" | "view-dblclick" | "view-hover" => {
            let (Some(x), Some(y)) = (it.next().and_then(num), it.next().and_then(num)) else { return err("view-click X Y (DIPs)") };
            v.mouse_move(x, y);
            if cmd != "view-hover" {
                v.mouse_down(x, y, false);
                v.mouse_up(x, y);
            }
            if cmd == "view-dblclick" {
                v.mouse_down(x, y, true);
                v.mouse_up(x, y);
            }
            json!({"ok": true, "hover": format!("{:?}", v.hover)})
        }
        "view-wheel" => {
            let nums: Vec<f32> = arg.split_whitespace().filter_map(num).collect();
            if nums.len() < 3 {
                return err("view-wheel X Y NOTCHES [ctrl] (negative = down)");
            }
            v.mouse_move(nums[0], nums[1]);
            for _ in 0..(nums[2].abs() as i32).max(1) {
                v.wheel(nums[0], nums[1], 120.0 * nums[2].signum(), arg.contains("ctrl"));
            }
            json!({"ok": true})
        }
        "view-drag" => {
            let nums: Vec<f32> = arg.split_whitespace().filter_map(num).collect();
            if nums.len() < 4 {
                return err("view-drag X0 Y0 X1 Y1");
            }
            v.mouse_move(nums[0], nums[1]);
            v.mouse_down(nums[0], nums[1], false);
            for k in 1..=12 {
                let f = k as f32 / 12.0;
                v.mouse_move(nums[0] + (nums[2] - nums[0]) * f, nums[1] + (nums[3] - nums[1]) * f);
            }
            v.mouse_up(nums[2], nums[3]);
            json!({"ok": true, "offset": [v.zx.value.round(), v.zy.value.round()]})
        }
        "view-hits" => {
            let h: Vec<_> = v.hits.iter().map(|(r, h)| (format!("{h:?}"), r)).filter(|(n, _)| arg.is_empty() || n.contains(arg)).map(|(n, r)| json!({"hit": n, "x": r.x, "y": r.y, "w": r.w, "h": r.h})).collect();
            json!({"ok": true, "hits": h})
        }
        "view-size" => {
            let (Some(w), Some(h)) = (it.next().and_then(num), it.next().and_then(num)) else { return err("view-size W H (DIPs)") };
            let s = v.k();
            unsafe {
                let _ = SetWindowPos(v.hwnd, None, 0, 0, (w * s) as i32, (h * s) as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            json!({"ok": true})
        }
        "view-paint" => {
            v.paint();
            json!({"ok": true})
        }
        "view-shot" => {
            let (file, hover) = match arg.strip_suffix(" --hover") {
                Some(f) => (f.trim(), true),
                None => (arg.trim(), false),
            };
            if file.is_empty() {
                return err("view-shot <file.png> [--hover]");
            }
            match v.shot(file, hover) {
                Ok(()) => json!({"ok": true, "file": file}),
                Err(e) => return err(&e),
            }
        }
        "view-close" => {
            unsafe {
                let _ = DestroyWindow(v.hwnd);
            }
            json!({"ok": true})
        }
        _ => return err("view-image <path> | view-state | view-key K | view-post K | view-click X Y | view-dblclick X Y | view-hover X Y | view-wheel X Y N [ctrl] | view-drag X0 Y0 X1 Y1 | view-hits [f] | view-size W H | view-paint | view-shot FILE [--hover] | view-close"),
    };
    r.to_string()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Viewer;
        if ptr.is_null() {
            if msg == WM_NCCALCSIZE && wp.0 != 0 {
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let v = &mut *ptr;
        let s = v.k();
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
                if v.full {
                    return LRESULT(HTCLIENT as isize);
                }
                if !IsZoomed(hwnd).as_bool() {
                    let b = 6.0;
                    let (l, r, t, bt) = (x < b, x > v.w - b, y < b, y > v.h - b);
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
                    return LRESULT(match v.hit_at(x, y) {
                        VHit::Min => HTMINBUTTON,
                        VHit::Max => HTMAXBUTTON,
                        VHit::Close => HTCLOSE,
                        VHit::Block => HTCLIENT,
                        _ if v.confirm => HTCLIENT,
                        _ => HTCAPTION,
                    } as isize);
                }
                LRESULT(HTCLIENT as isize)
            }
            WM_NCMOUSEMOVE => {
                let h = match wp.0 as u32 {
                    HTMINBUTTON => VHit::Min,
                    HTMAXBUTTON => VHit::Max,
                    HTCLOSE => VHit::Close,
                    _ => VHit::None,
                };
                if h != v.nc_hover {
                    v.nc_hover = h;
                    v.set_hover(VHit::None);
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            crate::app::WM_NCMOUSELEAVE => {
                v.nc_hover = VHit::None;
                v.set_hover(VHit::None);
                LRESULT(0)
            }
            WM_NCLBUTTONDOWN => match wp.0 as u32 {
                HTMINBUTTON | HTMAXBUTTON | HTCLOSE => {
                    v.nc_down = v.nc_hover;
                    v.invalidate();
                    LRESULT(0)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            },
            WM_NCLBUTTONUP => {
                let d = std::mem::replace(&mut v.nc_down, VHit::None);
                match (wp.0 as u32, d) {
                    (HTMINBUTTON, VHit::Min) => v.click(VHit::Min),
                    (HTMAXBUTTON, VHit::Max) => v.click(VHit::Max),
                    (HTCLOSE, VHit::Close) => v.click(VHit::Close),
                    _ => return DefWindowProcW(hwnd, msg, wp, lp),
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_xy(lp);
                v.mouse_move(x as f32 / s, y as f32 / s);
                LRESULT(0)
            }
            WM_MOUSELEAVE_ => {
                v.mouse_leave();
                LRESULT(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = lparam_xy(lp);
                SetCapture(hwnd);
                v.mouse_down(x as f32 / s, y as f32 / s, msg == WM_LBUTTONDBLCLK);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = lparam_xy(lp);
                v.mouse_up(x as f32 / s, y as f32 / s);
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                if v.drag.is_some() {
                    v.drag = None;
                    v.dragged = false;
                    v.pressed = VHit::None;
                }
                LRESULT(0)
            }
            WM_XBUTTONUP => {
                v.step(if (wp.0 >> 16) & 0xFFFF == 1 { -1 } else { 1 });
                LRESULT(1)
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
                let mut pt = POINT { x: lparam_xy(lp).0, y: lparam_xy(lp).1 };
                let _ = ScreenToClient(hwnd, &mut pt);
                v.wheel(pt.x as f32 / s, pt.y as f32 / s, d, (wp.0 & 0x0008) != 0);
                LRESULT(0)
            }
            WM_SYSKEYDOWN if wp.0 == 0x73 => {
                v.close();
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let (c, sh, alt) = mods();
                if v.key(wp.0 as u16, c, sh, alt || msg == WM_SYSKEYDOWN) {
                    return LRESULT(0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CHAR | WM_SYSCHAR => LRESULT(0),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                v.paint();
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                v.tip = None;
                if v.mica && v.gfx.composed() {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let dc = GetDC(Some(hwnd));
                    FillRect(dc, &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                    ReleaseDC(Some(hwnd), dc);
                }
                v.invalidate();
                LRESULT(0)
            }
            WM_TIMER => {
                v.on_timer(wp.0);
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
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xFFFF) as u32 == HTCLIENT {
                    let cur = match v.hover {
                        _ if v.drag.is_some() && v.dragged => IDC_SIZEALL,
                        VHit::Image if v.pannable() => IDC_SIZEALL,
                        VHit::Image | VHit::Stage | VHit::StripBg | VHit::Title | VHit::None | VHit::Block => IDC_ARROW,
                        _ => IDC_HAND,
                    };
                    SetCursor(LoadCursorW(None, cur).ok());
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            m if m == WM_VBG => {
                v.take_bg();
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                v.stop_workers();
                // the pictures' bitmaps go now, and the device gives their memory back
                v.pics.clear();
                v.thumbs.clear();
                v.shot_bmp = None;
                v.gfx.release_memory(true);
                if !v.offscreen && !v.test {
                    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                    let wp = match v.saved {
                        Some(s) => Some(s),
                        None => GetWindowPlacement(hwnd, &mut wp).is_ok().then_some(wp),
                    };
                    if let Some(wp) = wp {
                        let r = wp.rcNormalPosition;
                        let max = (wp.showCmd == SW_SHOWMAXIMIZED.0 as u32 || (v.saved.is_none() && IsZoomed(hwnd).as_bool())) as i32;
                        save_setting("window", json!([r.left, r.top, r.right, r.bottom, max]));
                    }
                }
                if VIEWER.load(Ordering::Relaxed) == hwnd.0 as isize {
                    VIEWER.store(0, Ordering::Relaxed);
                }
                // the main window takes the keys again
                if !v.test && !v.offscreen && IsWindow(Some(v.main)).as_bool() && GetForegroundWindow() == hwnd {
                    let _ = SetForegroundWindow(v.main);
                }
                LRESULT(0)
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

const WM_MOUSELEAVE_: u32 = 0x02A3;
