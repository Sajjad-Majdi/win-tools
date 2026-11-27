//! The update card (same flow as Rust File Explorer's): a few seconds after start (then every 12 hours) the app asks
//! the update server for a newer version (crate::update). A newer one shows a card at the bottom right with what
//! changed, Update and Later. Update downloads the setup with a progress bar, runs it silently and closes; the setup
//! starts the app again. The checks and the download run on their own threads and share `UpdState` with the window.
use super::app::*;
use super::gfx::{Align, Font, Rect};
use super::motion;
use super::paint::Btn;
use crate::update::{self, Offer};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

const KEY: &str = "tm";
const W: f32 = 360.0;
const MAX_NOTES: usize = 4;

#[derive(Clone)]
pub enum Phase {
    Offer,
    /// downloaded share 0..=1
    Down(f32),
    Err(String),
}

#[derive(Clone)]
pub struct Upd {
    pub offer: Offer,
    pub phase: Phase,
    pub at: Instant,
}

pub type UpdState = Arc<Mutex<Option<Upd>>>;

/// Settings > About while a check runs.
pub const CHECKING: &str = "Checking...";

fn redraw(hw: isize) {
    unsafe {
        let _ = InvalidateRect(Some(HWND(hw as *mut _)), None, false);
    }
}

impl App {
    /// Start the background checks (not in a local build: crate::update::ENABLED; in a test window only with
    /// WT_UPDATE_AS set).
    pub fn upd_schedule(&self) {
        if !update::ENABLED || (self.test && std::env::var_os("WT_UPDATE_AS").is_none()) {
            return;
        }
        let (st, hw) = (self.upd.clone(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(6));
            loop {
                // a failed check (offline, filtered) stays silent; the next one runs later
                if let Ok(Some(o)) = update::check(KEY, env!("CARGO_PKG_VERSION"), false) {
                    let mut g = st.lock().unwrap();
                    if !matches!(g.as_ref().map(|u| &u.phase), Some(Phase::Down(_))) {
                        *g = Some(Upd { offer: o, phase: Phase::Offer, at: Instant::now() });
                    }
                    drop(g);
                    redraw(hw);
                }
                std::thread::sleep(Duration::from_secs(12 * 3600));
            }
        });
    }

    /// Settings > About > Check for updates: asks now (ignores "Later") and shows the card again when there is one.
    fn upd_check_now(&mut self) {
        if !update::ENABLED {
            // a personal build made from the source: an update would replace it with the published one
            *self.upd_note.lock().unwrap() = Some("Local build, updates are off".into());
            self.invalidate();
            return;
        }
        *self.upd_note.lock().unwrap() = Some(CHECKING.into());
        let (st, note, hw) = (self.upd.clone(), self.upd_note.clone(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            let n = match update::check(KEY, env!("CARGO_PKG_VERSION"), true) {
                Ok(Some(o)) => {
                    let v = o.version.clone();
                    let mut g = st.lock().unwrap();
                    if !matches!(g.as_ref().map(|u| &u.phase), Some(Phase::Down(_))) {
                        *g = Some(Upd { offer: o, phase: Phase::Offer, at: Instant::now() });
                    }
                    format!("{v} is available")
                }
                Ok(None) => "Up to date".into(),
                Err(e) => e,
            };
            *note.lock().unwrap() = Some(n);
            redraw(hw);
        });
        self.invalidate();
    }

    /// The card's buttons: 0 Update (or Try again), 1 Later; 2 is Settings > About > Check for updates.
    pub fn upd_click(&mut self, k: u8) {
        if k == 2 {
            return self.upd_check_now();
        }
        let mut g = self.upd.lock().unwrap();
        let Some(u) = g.as_mut() else { return };
        if k == 1 {
            update::snooze(KEY, &u.offer.version);
            *g = None;
        } else if !matches!(u.phase, Phase::Down(_)) {
            u.phase = Phase::Down(0.0);
            let o = u.offer.clone();
            let (st, hw) = (self.upd.clone(), self.hwnd.0 as isize);
            std::thread::spawn(move || {
                let last = std::sync::atomic::AtomicU32::new(u32::MAX);
                let set = |p: Phase| {
                    if let Some(u) = st.lock().unwrap().as_mut() {
                        u.phase = p;
                    }
                    redraw(hw);
                };
                let r = update::download(&o, &|d, t| {
                    let pct = if t > 0 { (d * 100 / t) as u32 } else { 0 };
                    if last.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                        set(Phase::Down(pct as f32 / 100.0));
                    }
                    true
                });
                let r = r.and_then(|setup| update::install(&setup));
                match r {
                    Ok(()) => unsafe {
                        let _ = PostMessageW(Some(HWND(hw as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
                    },
                    Err(e) => set(Phase::Err(e)),
                }
            });
        }
        drop(g);
        self.invalidate();
    }

    pub fn paint_update(&mut self) {
        let Some(u) = self.upd.lock().unwrap().clone() else { return };
        let t = self.t;
        let notes: Vec<String> = u.offer.notes.iter().take(MAX_NOTES).cloned().collect();
        let alpha = self.tween_in(u.at, motion::NORMAL, motion::decel);
        let off = (1.0 - self.spring_in(u.at, motion::POP)) * 12.0;
        let old = self.gfx.push_alpha(alpha);
        let w = W.min(self.w - 24.0);
        let h = 16.0 + 22.0 + 20.0 + 8.0 + notes.len() as f32 * 22.0 + 10.0 + 32.0 + 16.0;
        let r = Rect::new(self.w - w - 16.0, self.h - h - 16.0 + off, w, h);
        self.fly_card(r);
        self.hit(r, Hit::Block);
        let (x0, iw) = (r.x + 16.0, r.w - 32.0);
        let mut y = r.y + 16.0;
        let g = &self.gfx;
        g.glyph(0xE896, Rect::new(x0, y, 20.0, 22.0), 16.0, t.acc_t);
        g.text("Update available", Rect::new(x0 + 28.0, y, iw - 28.0, 22.0), Font::Ui, 14.0, 600, Align::Left, t.t1);
        y += 22.0;
        g.text(&format!("Rust Task Manager {}", u.offer.version), Rect::new(x0 + 28.0, y, iw - 28.0, 20.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
        y += 20.0 + 8.0;
        for n in &notes {
            g.fill_round(Rect::new(x0 + 9.0, y + 9.0, 4.0, 4.0), 2.0, t.t3);
            g.text_at(n, x0 + 28.0, y, 22.0, iw - 28.0, Font::Ui, 12.5, 400, t.t1);
            y += 22.0;
        }
        y += 10.0;
        let row = Rect::new(x0, y, iw, 32.0);
        match &u.phase {
            Phase::Down(f) => {
                g.text(&format!("Downloading {}%", (f * 100.0).round() as u32), Rect::new(row.x, row.y, row.w, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
                let bar = Rect::new(row.x, row.y + 22.0, row.w, 4.0);
                g.fill_round(bar, 2.0, t.line);
                g.fill_round(Rect::new(bar.x, bar.y, bar.w * f, bar.h), 2.0, t.acc);
            }
            p => {
                let err = if let Phase::Err(e) = p { Some(e.clone()) } else { None };
                let main = if err.is_some() { "Try again" } else { "Update" };
                let lw = g.measure("Later", Font::Ui, 13.0, 400) + 32.0;
                let mw = g.measure(main, Font::Ui, 13.0, 400) + 32.0;
                if let Some(e) = &err {
                    g.text_at(e, row.x, row.y, 32.0, row.w - mw - lw - 16.0, Font::Ui, 12.0, 400, t.warn);
                }
                self.button(Rect::new(row.r() - lw, row.y, lw, 32.0), "Later", 0, Btn::Std, false, true, Hit::Upd(1));
                self.button(Rect::new(row.r() - lw - 8.0 - mw, row.y, mw, 32.0), main, 0, Btn::Accent, false, true, Hit::Upd(0));
            }
        }
        self.gfx.pop_alpha(old);
        if alpha < 1.0 || off.abs() > 0.1 {
            self.animate(30);
        }
    }
}
