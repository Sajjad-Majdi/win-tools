//! The update card: a few seconds after start (then every 12 hours) the app asks the update server for a newer
//! version (fmcore::update). A newer one shows a card at the bottom right with what changed, Update and Later.
//! Update downloads the setup with a progress bar, runs it silently and closes; the setup starts the app again.
use crate::app::*;
use crate::gfx::{Align, Font, Rect};
use crate::paint::Btn;
use crate::theme::{R_MENU, Theme};
use fmcore::update::{self, Offer};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const KEY: &str = "fm";
const W: f32 = 360.0;
const MAX_NOTES: usize = 4;

/// Settings > About while a check runs.
pub const CHECKING: &str = "Checking...";

pub enum UpdMsg {
    Checked(Result<Option<Offer>, String>),
    /// the answer to Settings > About > Check for updates
    Manual(Result<Option<Offer>, String>),
    Progress(u64, u64),
    Ready(Result<PathBuf, String>),
}

pub enum Phase {
    Offer,
    /// downloaded share 0..=1
    Down(f32),
    Err(String),
}

pub struct Upd {
    pub offer: Offer,
    pub phase: Phase,
    pub at: Instant,
}

impl App {
    /// Start the background checks (a local build never checks: fmcore::update::ENABLED).
    pub fn upd_schedule(&self) {
        if !update::ENABLED {
            return;
        }
        let (bg, hw) = (self.bg.clone(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(6));
            loop {
                let r = update::check(KEY, env!("CARGO_PKG_VERSION"), false);
                // a failed check (offline, filtered) stays silent; the next one runs later
                if let Ok(Some(o)) = r {
                    post_bg(&bg, hw, Msg::Upd(UpdMsg::Checked(Ok(Some(o)))));
                }
                std::thread::sleep(Duration::from_secs(12 * 3600));
            }
        });
    }

    pub fn upd_apply(&mut self, m: UpdMsg) {
        match m {
            UpdMsg::Checked(Ok(Some(o))) => {
                // a card that is downloading stays as it is
                if !matches!(self.upd.as_ref().map(|u| &u.phase), Some(Phase::Down(_))) {
                    self.upd = Some(Upd { offer: o, phase: Phase::Offer, at: Instant::now() });
                    self.anim();
                }
            }
            UpdMsg::Checked(_) => {}
            UpdMsg::Manual(r) => {
                self.upd_note = Some(match r {
                    Ok(Some(o)) => {
                        let v = o.version.clone();
                        if !matches!(self.upd.as_ref().map(|u| &u.phase), Some(Phase::Down(_))) {
                            self.upd = Some(Upd { offer: o, phase: Phase::Offer, at: Instant::now() });
                            self.anim();
                        }
                        format!("{v} is available")
                    }
                    Ok(None) => "Up to date".into(),
                    Err(e) => e,
                });
            }
            UpdMsg::Progress(d, t) => {
                if let Some(u) = self.upd.as_mut() {
                    u.phase = Phase::Down(if t > 0 { (d as f32 / t as f32).min(1.0) } else { 0.0 });
                }
            }
            UpdMsg::Ready(Ok(setup)) => {
                match update::install(&setup) {
                    Ok(()) => unsafe {
                        use windows::Win32::Foundation::{LPARAM, WPARAM};
                        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                        let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    },
                    Err(e) => self.upd_err(e),
                }
            }
            UpdMsg::Ready(Err(e)) => self.upd_err(e),
        }
        self.invalidate();
    }

    fn upd_err(&mut self, e: String) {
        if let Some(u) = self.upd.as_mut() {
            u.phase = Phase::Err(e);
        }
    }

    /// Settings > About > Check for updates: asks now (ignores "Later") and shows the card again when there is one.
    pub fn upd_check_now(&mut self) {
        if !update::ENABLED {
            // a personal build made from the source: an update would replace it with the published one
            self.upd_note = Some("Local build, updates are off".into());
            self.invalidate();
            return;
        }
        self.upd_note = Some(CHECKING.into());
        let (bg, hw) = (self.bg.clone(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            let r = update::check(KEY, env!("CARGO_PKG_VERSION"), true);
            post_bg(&bg, hw, Msg::Upd(UpdMsg::Manual(r)));
        });
        self.invalidate();
    }

    /// The card's buttons: 0 Update (or Try again), 1 Later.
    pub fn upd_click(&mut self, k: u8) {
        let Some(u) = self.upd.as_mut() else { return };
        if k == 1 {
            update::snooze(KEY, &u.offer.version);
            self.upd = None;
        } else if !matches!(u.phase, Phase::Down(_)) {
            u.phase = Phase::Down(0.0);
            let o = u.offer.clone();
            let (bg, hw) = (self.bg.clone(), self.hwnd.0 as isize);
            std::thread::spawn(move || {
                let last = std::sync::atomic::AtomicU32::new(u32::MAX);
                let r = update::download(&o, &|d, t| {
                    let pct = if t > 0 { (d * 100 / t) as u32 } else { 0 };
                    if last.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                        post_bg(&bg, hw, Msg::Upd(UpdMsg::Progress(d, t)));
                    }
                    true
                });
                post_bg(&bg, hw, Msg::Upd(UpdMsg::Ready(r)));
            });
        }
        self.invalidate();
    }

    pub fn paint_update(&mut self, t: &Theme) {
        let Some(u) = self.upd.as_ref() else { return };
        let notes: Vec<String> = u.offer.notes.iter().take(MAX_NOTES).cloned().collect();
        let (ver, at) = (u.offer.version.clone(), u.at);
        let phase = match &u.phase {
            Phase::Offer => None,
            Phase::Down(f) => Some(Ok(*f)),
            Phase::Err(e) => Some(Err(e.clone())),
        };
        let k = crate::paint::prog(at, 220.0, self.shooting);
        let w = W.min(self.w - 16.0);
        let h = 16.0 + 22.0 + 20.0 + 8.0 + notes.len() as f32 * 22.0 + 10.0 + 32.0 + 16.0;
        let x = self.w - w - 16.0;
        let y = self.h - STATUS_H - 12.0 - h + (1.0 - k) * 12.0;
        let r = Rect::new(x, y, w, h);
        self.gfx.opacity_layer(r.inset(-30.0, -30.0), k.clamp(0.05, 1.0));
        self.card(r, R_MENU, t, true);
        self.hit(r, Hit::Block);
        let x0 = r.x + 16.0;
        let iw = r.w - 32.0;
        let mut yy = r.y + 16.0;
        self.gfx.glyph(0xE896, Rect::new(x0, yy, 20.0, 22.0), 16.0, t.accent);
        self.gfx.text("Update available", Rect::new(x0 + 28.0, yy, iw - 28.0, 22.0), Font::Ui, 14.0, 600, Align::Left, t.text);
        yy += 22.0;
        self.gfx.text(&format!("Rust File Explorer {ver}"), Rect::new(x0 + 28.0, yy, iw - 28.0, 20.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
        yy += 20.0 + 8.0;
        for n in &notes {
            self.gfx.fill_round(Rect::new(x0 + 9.0, yy + 9.0, 4.0, 4.0), 2.0, t.text3);
            self.gfx.text(n, Rect::new(x0 + 28.0, yy, iw - 28.0, 22.0), Font::Ui, 12.5, 400, Align::Left, t.text);
            yy += 22.0;
        }
        yy += 10.0;
        let row = Rect::new(x0, yy, iw, 32.0);
        match phase {
            Some(Ok(f)) => {
                let label = format!("Downloading {}%", (f * 100.0).round() as u32);
                self.gfx.text(&label, Rect::new(row.x, row.y, row.w, 16.0), Font::Ui, 12.0, 400, Align::Left, t.text2);
                let bar = Rect::new(row.x, row.y + 22.0, row.w, 4.0);
                self.gfx.fill_round(bar, 2.0, t.text.a(0.14));
                self.gfx.fill_round(Rect::new(bar.x, bar.y, bar.w * f, bar.h), 2.0, t.accent);
            }
            other => {
                let later_w = self.gfx.measure("Later", Font::Ui, 13.0, 400) + 32.0;
                let main = if other.is_some() { "Try again" } else { "Update" };
                let main_w = self.gfx.measure(main, Font::Ui, 13.0, 600) + 32.0;
                if let Some(Err(e)) = &other {
                    self.gfx.text(e, Rect::new(row.x, row.y, row.w - main_w - later_w - 16.0, row.h), Font::Ui, 12.0, 400, Align::Left, t.danger);
                }
                self.button(Rect::new(row.r() - later_w, row.y, later_w, 32.0), "Later", 0, Btn::Std, Hit::Upd(1), t);
                self.button(Rect::new(row.r() - later_w - 8.0 - main_w, row.y, main_w, 32.0), main, 0, Btn::Accent, Hit::Upd(0), t);
            }
        }
        self.gfx.pop_layer();
        if k != 1.0 {
            self.anim();
        }
    }
}
