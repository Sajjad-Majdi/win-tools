//! Motion: every curve, timing and spring the window uses, in one place.
//! Colour fades use short eased tweens; things that move (pane, flyouts, menus, the selection highlight,
//! pills, toggle knobs, press-scale, scrolling) use springs, so they can be re-targeted mid-flight and keep
//! their speed. Nothing here runs a timer by itself: callers ask the window for frames only while a value moves.
use std::time::Instant;

/// Hover and press colour fades.
pub const FAST: f32 = 83.0;
/// Fades of menus, flyouts and toasts; leaving things.
pub const NORMAL: f32 = 167.0;
/// Number and bar transitions after a new sample.
pub const VALUE: f32 = 260.0;
/// Rows that change place.
pub const MOVE: f32 = 250.0;
/// Press depth -> scale (1.0 at rest, 0.97 fully pressed).
pub const PRESS_SCALE: f32 = 0.03;

/// cubic-bezier(x1, y1, x2, y2) at time `t` (0..1).
pub fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let bx = |u: f32| 3.0 * (1.0 - u) * (1.0 - u) * u * x1 + 3.0 * (1.0 - u) * u * u * x2 + u * u * u;
    let by = |u: f32| 3.0 * (1.0 - u) * (1.0 - u) * u * y1 + 3.0 * (1.0 - u) * u * u * y2 + u * u * u;
    // bisection: x(u) is monotonic for 0 <= x1, x2 <= 1
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if bx(mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    by((lo + hi) * 0.5)
}

/// Things coming in: fast start, long soft landing.
pub fn decel(t: f32) -> f32 {
    bezier(0.0, 0.0, 0.0, 1.0, t)
}

/// Things leaving: slow start, quick exit.
pub fn accel(t: f32) -> f32 {
    bezier(0.7, 0.0, 1.0, 0.5, t)
}

/// Moves from one place to another.
pub fn standard(t: f32) -> f32 {
    bezier(0.8, 0.0, 0.2, 1.0, t)
}

/// Spring settings, iOS style: `response` = period in seconds, `damping` = damping ratio (1 = no bounce).
#[derive(Clone, Copy, Debug)]
pub struct Cfg {
    pub response: f32,
    pub damping: f32,
}

/// Side pane in / out.
pub const PANE: Cfg = Cfg { response: 0.40, damping: 0.86 };
/// Flyouts, menus, toasts and the dialog.
pub const POP: Cfg = Cfg { response: 0.30, damping: 0.78 };
/// Selection highlight, tab and segment pills, toggle knobs.
pub const GLIDE: Cfg = Cfg { response: 0.32, damping: 0.84 };
/// Press-scale: quick in, a small bounce on release.
pub const PRESS: Cfg = Cfg { response: 0.20, damping: 0.58 };
/// Momentum scrolling: critically damped, so it glides and stops softly without bouncing.
pub const SCROLL: Cfg = Cfg { response: 0.34, damping: 1.0 };

/// Step response of a spring at rest at 0 that is pulled to 1, `t` seconds after it starts (may overshoot 1).
pub fn spring(t: f32, c: Cfg) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    let w = std::f32::consts::TAU / c.response;
    let z = c.damping.min(0.999);
    let wd = w * (1.0 - z * z).sqrt();
    let env = (-z * w * t).exp();
    1.0 - env * ((wd * t).cos() + (z * w / wd) * (wd * t).sin())
}

/// Time (seconds) after which `spring()` stays within 0.2% of 1.
pub fn settle(c: Cfg) -> f32 {
    let w = std::f32::consts::TAU / c.response;
    (500.0f32).ln() / (c.damping.min(0.999) * w)
}

/// A spring that can be re-targeted while it moves and keeps its speed (selection, pills, scrolling, press).
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
    pub target: f32,
    at: Instant,
    /// distance below which it counts as stopped (units of x)
    eps: f32,
}

impl Spring {
    pub fn new(x: f32, eps: f32) -> Spring {
        Spring { x, v: 0.0, target: x, at: Instant::now(), eps }
    }

    /// Advance to `now` toward `target`; returns the new position.
    pub fn step(&mut self, target: f32, c: Cfg, now: Instant) -> f32 {
        if self.v == 0.0 && (self.x - self.target).abs() <= self.eps {
            // at rest: the time it rested does not count; start with one frame of motion
            self.at = now - std::time::Duration::from_millis(8);
        }
        self.target = target;
        let mut dt = now.saturating_duration_since(self.at).as_secs_f32().min(0.1);
        self.at = now;
        let w = std::f32::consts::TAU / c.response;
        let (k, d) = (w * w, 2.0 * c.damping * w);
        // semi-implicit Euler in 2 ms steps (stable for these stiffnesses)
        while dt > 0.0 {
            let h = dt.min(0.002);
            let a = -k * (self.x - target) - d * self.v;
            self.v += a * h;
            self.x += self.v * h;
            dt -= h;
        }
        if !self.moving() {
            self.x = target;
            self.v = 0.0;
        }
        self.x
    }

    pub fn moving(&self) -> bool {
        (self.x - self.target).abs() > self.eps || self.v.abs() > self.eps * 8.0
    }

    /// Jump to `x` and stop.
    pub fn snap(&mut self, x: f32) {
        self.x = x;
        self.target = x;
        self.v = 0.0;
        self.at = Instant::now();
    }
}

/// A timed fade from one value to another along a curve (hover fills, number and bar transitions).
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub at: Instant,
    pub ms: f32,
    pub used: Instant,
}

impl Tween {
    pub fn fixed(v: f32) -> Tween {
        let now = Instant::now();
        Tween { from: v, to: v, at: now, ms: 0.0, used: now }
    }

    pub fn progress(&self, now: Instant) -> f32 {
        if self.ms <= 0.0 { 1.0 } else { (now.saturating_duration_since(self.at).as_secs_f32() * 1000.0 / self.ms).clamp(0.0, 1.0) }
    }

    pub fn value(&self, now: Instant, curve: fn(f32) -> f32) -> f32 {
        let p = self.progress(now);
        self.from + (self.to - self.from) * curve(p)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    /// Head for `to` from wherever the value is now.
    pub fn retarget(&mut self, to: f32, ms: f32, now: Instant, curve: fn(f32) -> f32) {
        self.from = self.value(now, curve);
        self.to = to;
        self.at = now;
        self.ms = ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_hit_ends() {
        for f in [decel, accel, standard] {
            assert!(f(0.0).abs() < 1e-4 && (f(1.0) - 1.0).abs() < 1e-4);
        }
        // decel is ahead early, accel is behind
        assert!(decel(0.3) > 0.6 && accel(0.3) < 0.1);
    }

    #[test]
    fn spring_settles_and_overshoots_a_little() {
        let peak = (1..200).map(|i| spring(i as f32 * 0.005, POP)).fold(0.0f32, f32::max);
        assert!(peak > 1.0 && peak < 1.08, "peak {peak}");
        assert!((spring(settle(POP), POP) - 1.0).abs() < 0.01);
        assert!((spring(2.0, SCROLL) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn stateful_spring_reaches_target() {
        let mut s = Spring::new(0.0, 0.05);
        let t0 = Instant::now();
        let mut x = 0.0;
        for i in 1..=120 {
            x = s.step(100.0, GLIDE, t0 + std::time::Duration::from_millis(i * 8));
        }
        assert!((x - 100.0).abs() < 0.01 && !s.moving());
    }
}
