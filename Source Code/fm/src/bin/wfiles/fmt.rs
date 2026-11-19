//! Formatting: sizes with three significant digits, dates by age (24-hour), a one-line text field.
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

pub fn size(n: u64) -> String {
    if n < 1024 {
        return format!("{n} bytes");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1000.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 {
        format!("{v:.2} {}", units[u])
    } else if v < 100.0 {
        format!("{v:.1} {}", units[u])
    } else {
        format!("{v:.0} {}", units[u])
    }
}

/// Short size for drive free space ("135 GB", "4.07 GB").
pub fn size_short(n: u64) -> String {
    size(n)
}

pub fn local(ft: u64) -> Option<SYSTEMTIME> {
    if ft == 0 {
        return None;
    }
    let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut loc = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&f, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut loc).ok()?;
    }
    Some(loc)
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Dates by age: this week "Mon 17:46", this year "16 Sep 18:34", other years "16 Sep 2025".
/// `short` drops the time (narrow columns): "Mon", "16 Sep", "16 Sep 2025".
pub fn date(ft: u64, short: bool) -> String {
    let Some(t) = local(ft) else { return String::new() };
    let now_ft = fmcore::util::now_ft();
    let Some(now) = local(now_ft) else { return String::new() };
    let age_s = (now_ft as i64 - ft as i64) / 10_000_000;
    let hm = format!("{:02}:{:02}", t.wHour, t.wMinute);
    let m = MONTHS[(t.wMonth.clamp(1, 12) - 1) as usize];
    if t.wYear == now.wYear && t.wMonth == now.wMonth && t.wDay == now.wDay {
        return if short { hm } else { format!("Today {hm}") };
    }
    if (0..6 * 86400).contains(&age_s) {
        let d = DAYS[t.wDayOfWeek as usize % 7];
        return if short { d.to_string() } else { format!("{d} {hm}") };
    }
    if t.wYear == now.wYear {
        return if short { format!("{} {m}", t.wDay) } else { format!("{} {m} {hm}", t.wDay) };
    }
    format!("{} {m} {}", t.wDay, t.wYear)
}

pub fn date_long(ft: u64) -> String {
    let Some(t) = local(ft) else { return String::new() };
    let m = MONTHS[(t.wMonth.clamp(1, 12) - 1) as usize];
    format!("{} {m} {} {:02}:{:02}", t.wDay, t.wYear, t.wHour, t.wMinute)
}

pub fn date_ymd(ft: u64) -> String {
    let Some(t) = local(ft) else { return String::new() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

pub fn thousands(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn duration(s: u64) -> String {
    if s < 60 {
        format!("{s} s left")
    } else if s < 3600 {
        format!("{} min left", s.div_ceil(60))
    } else {
        format!("{} h {} min left", s / 3600, (s % 3600) / 60)
    }
}

/// Persian digits for plain numbers inside Persian text. Versions and Latin tokens (1.98.0, x86_64, v2) keep
/// their Latin digits, as the design asks.
pub fn fa_digits(s: &str) -> String {
    let ch: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() * 2);
    let mut i = 0;
    while i < ch.len() {
        if !ch[i].is_ascii_digit() {
            out.push(ch[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < ch.len() && (ch[i].is_ascii_digit() || (ch[i] == '.' && i + 1 < ch.len() && ch[i + 1].is_ascii_digit())) {
            i += 1;
        }
        let tok: String = ch[start..i].iter().collect();
        let latin = |c: Option<&char>| c.map(|c| c.is_ascii_alphabetic() || *c == '_').unwrap_or(false);
        let version = tok.matches('.').count() >= 2 || latin(ch.get(i)) || (start > 0 && latin(ch.get(start - 1)));
        if version {
            out.push_str(&tok);
        } else {
            out.extend(tok.chars().map(|c| if c.is_ascii_digit() { char::from_u32(0x06F0 + c as u32 - '0' as u32).unwrap() } else { '\u{066B}' }));
        }
    }
    out
}

/// One-line editable text (UTF-16 caret positions, selection anchor).
#[derive(Clone, Default, Debug)]
pub struct Field {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
}

impl Field {
    pub fn new(s: &str) -> Field {
        let n = s.encode_utf16().count();
        Field { text: s.to_string(), caret: n, anchor: n }
    }
    pub fn len16(&self) -> usize {
        self.text.encode_utf16().count()
    }
    fn byte_at(&self, i16: usize) -> usize {
        let mut n = 0;
        for (b, ch) in self.text.char_indices() {
            if n >= i16 {
                return b;
            }
            n += ch.len_utf16();
        }
        self.text.len()
    }
    pub fn sel_range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
    pub fn has_sel(&self) -> bool {
        self.caret != self.anchor
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.len16();
    }
    pub fn selected(&self) -> String {
        let (a, b) = self.sel_range();
        self.text[self.byte_at(a)..self.byte_at(b)].to_string()
    }
    fn delete_sel(&mut self) {
        let (a, b) = self.sel_range();
        let (ba, bb) = (self.byte_at(a), self.byte_at(b));
        self.text.replace_range(ba..bb, "");
        self.caret = a;
        self.anchor = a;
    }
    pub fn insert(&mut self, s: &str) {
        if self.has_sel() {
            self.delete_sel();
        }
        let s: String = s.chars().filter(|c| *c != '\r' && *c != '\n' && *c != '\t').collect();
        let b = self.byte_at(self.caret);
        self.text.insert_str(b, &s);
        self.caret += s.encode_utf16().count();
        self.anchor = self.caret;
    }
    fn prev(&self, i: usize) -> usize {
        // step over a surrogate pair
        let w: Vec<u16> = self.text.encode_utf16().collect();
        if i == 0 {
            return 0;
        }
        if i >= 2 && (0xDC00..0xE000).contains(&w[i - 1]) { i - 2 } else { i - 1 }
    }
    fn next(&self, i: usize) -> usize {
        let w: Vec<u16> = self.text.encode_utf16().collect();
        if i >= w.len() {
            return w.len();
        }
        if (0xD800..0xDC00).contains(&w[i]) { (i + 2).min(w.len()) } else { i + 1 }
    }
    fn word_left(&self, i: usize) -> usize {
        let w: Vec<u16> = self.text.encode_utf16().collect();
        let mut j = i;
        while j > 0 && w[j - 1] == b' ' as u16 {
            j -= 1;
        }
        while j > 0 && !matches!(w[j - 1], 32 | 92 | 46 | 45 | 95) {
            j -= 1;
        }
        j
    }
    fn word_right(&self, i: usize) -> usize {
        let w: Vec<u16> = self.text.encode_utf16().collect();
        let mut j = i;
        while j < w.len() && !matches!(w[j], 32 | 92 | 46 | 45 | 95) {
            j += 1;
        }
        while j < w.len() && matches!(w[j], 32 | 92 | 46 | 45 | 95) {
            j += 1;
        }
        j
    }
    /// Editing keys. Returns true when the key was used. Clipboard keys are handled by the caller.
    pub fn key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let n = self.len16();
        let mv = |f: &mut Field, to: usize| {
            f.caret = to.min(n);
            if !shift {
                f.anchor = f.caret;
            }
        };
        match vk {
            0x25 => {
                // left
                if self.has_sel() && !shift {
                    let a = self.sel_range().0;
                    self.caret = a;
                    self.anchor = a;
                } else {
                    let to = if ctrl { self.word_left(self.caret) } else { self.prev(self.caret) };
                    mv(self, to);
                }
            }
            0x27 => {
                if self.has_sel() && !shift {
                    let b = self.sel_range().1;
                    self.caret = b;
                    self.anchor = b;
                } else {
                    let to = if ctrl { self.word_right(self.caret) } else { self.next(self.caret) };
                    mv(self, to);
                }
            }
            0x24 => mv(self, 0),
            0x23 => mv(self, n),
            0x08 => {
                if self.has_sel() {
                    self.delete_sel();
                } else if self.caret > 0 {
                    let from = if ctrl { self.word_left(self.caret) } else { self.prev(self.caret) };
                    self.anchor = from;
                    self.delete_sel();
                }
            }
            0x2E => {
                if self.has_sel() {
                    self.delete_sel();
                } else if self.caret < n {
                    self.anchor = if ctrl { self.word_right(self.caret) } else { self.next(self.caret) };
                    self.delete_sel();
                }
            }
            0x41 if ctrl => self.select_all(),
            _ => return false,
        }
        true
    }
}
