//! Motion: every curve and timing of the window in one place (no other file invents its own).
//! Self-contained (std only), so the Task Manager can copy it as is and both apps move the same way.
//!
//! - Springs (closed form, no state): `spring(t, kind)` gives 0 -> 1 for things that move or appear.
//! - Fades: `fade_in` / `fade_out` for colour changes (hover, press) - fast and plain.
//! - `Follow`: a critically damped spring that chases a target (smooth / momentum scrolling, slides).
//! - `Fades`: hover / press amounts per key, eased in and out.
//! Something animates only while a value is not settled; the window runs its frame timer only then.
#![allow(dead_code)] // shared with the Task Manager: not every helper is used by both apps
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Instant;

/// Durations in ms.
pub const FAST: f32 = 90.0; // hover and press colour
pub const NORMAL: f32 = 170.0; // fades, small slides
pub const SLOW: f32 = 260.0; // larger moves

/// Distance a menu or flyout travels while it opens (DIPs).
pub const POP_SLIDE: f32 = 8.0;
/// Scale of a pressed button or pill (springs back on release).
pub const PRESS_SCALE: f32 = 0.97;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Spring {
    /// Critically damped, quick: selection, small moves. Settles in ~220 ms.
    Snappy,
    /// A little bouncy (about 4 % overshoot): menus, flyouts, preview, panels.
    Bouncy,
    /// Critically damped, soft: views and large panels. Settles in ~360 ms.
    Gentle,
}

impl Spring {
    /// (angular frequency in rad/s, damping ratio)
    fn params(self) -> (f32, f32) {
        match self {
            Spring::Snappy => (30.0, 1.0),
            Spring::Bouncy => (24.0, 0.72),
            Spring::Gentle => (18.0, 1.0),
        }
    }
    /// Time (ms) after which the spring is treated as settled.
    pub fn settle_ms(self) -> f32 {
        match self {
            Spring::Snappy => 260.0,
            Spring::Bouncy => 380.0,
            Spring::Gentle => 420.0,
        }
    }
}

/// Step response of a damped spring at `ms` milliseconds (0 -> 1, may overshoot a little when bouncy).
pub fn spring(ms: f32, kind: Spring) -> f32 {
    if ms <= 0.0 {
        return 0.0;
    }
    if ms >= kind.settle_ms() {
        return 1.0;
    }
    let (w, z) = kind.params();
    let t = ms / 1000.0;
    if z >= 1.0 {
        1.0 - (-w * t).exp() * (1.0 + w * t)
    } else {
        let wd = w * (1.0 - z * z).sqrt();
        1.0 - (-z * w * t).exp() * ((wd * t).cos() + z * w / wd * (wd * t).sin())
    }
}

/// Spring progress since `at` (1.0 at once when `instant`, e.g. screenshots or reduced motion).
pub fn since(at: Instant, kind: Spring, instant: bool) -> f32 {
    if instant { 1.0 } else { spring(at.elapsed().as_secs_f32() * 1000.0, kind) }
}

/// True while a spring started at `at` is still moving.
pub fn moving(at: Instant, kind: Spring) -> bool {
    at.elapsed().as_secs_f32() * 1000.0 < kind.settle_ms()
}

/// Decelerating curve for colours coming in (cubic ease-out).
pub fn fade_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Accelerating curve for things leaving (quadratic ease-in).
pub fn fade_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t
}

/// Linear progress of `ms` since `at`, 0..1.
pub fn lin(at: Instant, ms: f32) -> f32 {
    (at.elapsed().as_secs_f32() * 1000.0 / ms).clamp(0.0, 1.0)
}

/// A value that follows its target with a critically damped spring and keeps its speed between targets: adding to
/// the target again while moving builds momentum (wheel scrolling); it slows down softly at the end.
#[derive(Clone, Copy, Debug)]
pub struct Follow {
    pub value: f32,
    pub target: f32,
    pub vel: f32,
    last: Option<Instant>,
}

impl Default for Follow {
    fn default() -> Self {
        Follow { value: 0.0, target: 0.0, vel: 0.0, last: None }
    }
}

impl Follow {
    /// Angular frequency (rad/s): higher = quicker.
    const W: f32 = 16.0;

    pub fn set(&mut self, v: f32) {
        self.value = v;
        self.target = v;
        self.vel = 0.0;
        self.last = None;
    }

    pub fn to(&mut self, target: f32) {
        if self.last.is_none() {
            self.last = Some(Instant::now());
        }
        self.target = target;
    }

    pub fn settled(&self) -> bool {
        self.last.is_none()
    }

    /// Advance to now; returns true while still moving.
    pub fn step(&mut self) -> bool {
        let Some(last) = self.last else { return false };
        let now = Instant::now();
        let mut dt = (now - last).as_secs_f32().min(0.05);
        self.last = Some(now);
        // a few sub-steps keep the integration stable on slow frames
        while dt > 0.0 {
            let h = dt.min(0.008);
            let w = Self::W;
            let acc = w * w * (self.target - self.value) - 2.0 * w * self.vel;
            self.vel += acc * h;
            self.value += self.vel * h;
            dt -= h;
        }
        if (self.target - self.value).abs() < 0.3 && self.vel.abs() < 8.0 {
            self.value = self.target;
            self.vel = 0.0;
            self.last = None;
            return false;
        }
        true
    }
}

/// Hover / press amounts per key: each eases towards 1 when on and towards 0 when off.
pub struct Fades<K: Eq + Hash + Copy> {
    map: HashMap<K, (f32, f32, Instant, bool)>,
    ms_in: f32,
    ms_out: f32,
}

impl<K: Eq + Hash + Copy> Fades<K> {
    pub fn new(ms_in: f32, ms_out: f32) -> Self {
        Fades { map: HashMap::new(), ms_in, ms_out }
    }

    /// Turn `key` on and every other key off.
    pub fn only(&mut self, key: Option<K>) {
        let keys: Vec<K> = self.map.keys().copied().collect();
        for k in keys {
            if Some(k) != key {
                self.set(k, false);
            }
        }
        if let Some(k) = key {
            self.set(k, true);
        }
    }

    pub fn set(&mut self, key: K, on: bool) {
        let now = self.get(key);
        match self.map.get_mut(&key) {
            Some(e) if e.3 == on => {}
            Some(e) => *e = (now, if on { 1.0 } else { 0.0 }, Instant::now(), on),
            None if on => {
                self.map.insert(key, (0.0, 1.0, Instant::now(), true));
            }
            None => {}
        }
    }

    /// Current amount 0..1.
    pub fn get(&self, key: K) -> f32 {
        let Some(&(from, to, at, on)) = self.map.get(&key) else { return 0.0 };
        let t = lin(at, if on { self.ms_in } else { self.ms_out });
        let k = if on { fade_in(t) } else { 1.0 - fade_out(1.0 - t) };
        from + (to - from) * k
    }

    /// Drop finished "off" entries; true while anything still changes.
    pub fn busy(&mut self) -> bool {
        let (ms_in, ms_out) = (self.ms_in, self.ms_out);
        self.map.retain(|_, e| e.3 || lin(e.2, ms_out) < 1.0);
        self.map.values().any(|e| lin(e.2, if e.3 { ms_in } else { ms_out }) < 1.0)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn springs_start_at_0_and_end_at_1() {
        for k in [Spring::Snappy, Spring::Bouncy, Spring::Gentle] {
            assert_eq!(spring(0.0, k), 0.0);
            assert_eq!(spring(k.settle_ms(), k), 1.0);
            assert!((spring(k.settle_ms() - 1.0, k) - 1.0).abs() < 0.03, "{k:?} jumps at the end");
        }
        // bouncy overshoots a little, never much
        let peak = (0..380).map(|ms| spring(ms as f32, Spring::Bouncy)).fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.08, "peak {peak}");
    }

    #[test]
    fn follow_reaches_target() {
        let mut f = Follow::default();
        f.set(0.0);
        f.to(100.0);
        let t0 = Instant::now();
        while f.step() {
            std::thread::sleep(std::time::Duration::from_millis(4));
            assert!(t0.elapsed().as_secs() < 2);
        }
        assert_eq!(f.value, 100.0);
    }
}
