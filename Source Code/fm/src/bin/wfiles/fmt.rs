//! Formatting: sizes with three significant digits, dates by age (24-hour), a one-line text field.
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

pub fn size(n: u64) -> String {
    if n < 1024 {
        return if n == 1 { "1 byte".into() } else { format!("{n} bytes") };
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

/// One-line editable text that behaves like the Windows EDIT control (the box Explorer renames a file in): UTF-16
/// caret positions, a selection anchor, EDIT's own word rules, its one-step undo and its sideways scrolling.
/// Every rule here was measured on a real EDIT control (Windows 11 26200), see the tests at the end of this file.
#[derive(Clone, Default, Debug)]
pub struct Field {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    /// sideways scroll in DIPs, kept between frames: it moves only when the caret leaves the box (`edit_scroll`)
    pub scroll: f32,
    /// the caret is drawn at the near edge of the character after it, not at the far edge of the one before
    /// (set by a click on a character's near half; the two differ only where the text changes direction)
    pub lead: bool,
    /// the word a double-click selected: a drag with the button still down then selects whole words
    pub word: Option<(usize, usize)>,
    /// the selection was set by the program (EM_SETSEL): the text does not scroll to the caret until the caret
    /// is moved by hand. The rename box opens showing the start of a long name, as in Explorer.
    pub hold: bool,
    /// EDIT's undo buffer: the text Undo puts back at `undo_pos`, and how many units typed there it removes
    undo_text: String,
    undo_pos: usize,
    undo_ins: usize,
}

/// Per UTF-16 unit: (a line may break before it, it is white space). This is Uniscribe's ScriptBreak, the same
/// source the EDIT control takes its words from: a "word" runs from one break to the next, with its spaces
/// ("my-" | "file_name " | "v2.final.tar.gz").
pub fn word_attrs(w: &[u16]) -> Vec<(bool, bool)> {
    use windows::Win32::Globalization::{SCRIPT_ANALYSIS, SCRIPT_LOGATTR, ScriptBreak};
    let n = w.len();
    if n == 0 {
        return vec![];
    }
    let mut la = vec![SCRIPT_LOGATTR::default(); n];
    let sa = SCRIPT_ANALYSIS::default();
    // (a pointer and a length: the text needs no terminating zero)
    let ok = unsafe { ScriptBreak(windows::core::PCWSTR(w.as_ptr()), n as i32, &sa, la.as_mut_ptr()).is_ok() };
    if !ok {
        // no Uniscribe: words end after their spaces
        return (0..n).map(|i| (i > 0 && w[i - 1] == 32 && w[i] != 32, w[i] == 32)).collect();
    }
    la.iter().enumerate().map(|(i, a)| (i > 0 && a._bitfield & 1 != 0, a._bitfield & 2 != 0)).collect()
}

/// The word (with its trailing spaces) that holds unit `pos`.
fn chunk_of(at: &[(bool, bool)], pos: usize) -> (usize, usize) {
    let n = at.len();
    if n == 0 {
        return (0, 0);
    }
    let pos = pos.min(n - 1);
    let mut a = pos;
    while a > 0 && !at[a].0 {
        a -= 1;
    }
    let mut b = pos + 1;
    while b < n && !at[b].0 {
        b += 1;
    }
    (a, b)
}

/// EDIT's "word to the left of i": the word that holds the unit before i.
fn word_before(at: &[(bool, bool)], i: usize) -> (usize, usize) {
    if i == 0 { (0, 0) } else { chunk_of(at, i - 1) }
}

/// EDIT's "word to the right of i": the word that holds unit i; from a space, the NEXT word (this is why
/// Ctrl+Right at the end of a word jumps over the whole next word in Explorer).
fn word_after(at: &[(bool, bool)], i: usize) -> (usize, usize) {
    let n = at.len();
    if i >= n {
        return (n, n);
    }
    if !at[i].1 {
        return chunk_of(at, i);
    }
    let mut j = i + 1;
    while j < n && !at[j].0 {
        j += 1;
    }
    if j >= n { (n, n) } else { chunk_of(at, j) }
}

/// EDIT's sideways scrolling: the text stays put while the caret is inside the box; a caret past the right edge
/// lands a quarter of the box before that edge (never past the end of the text), one past the left edge a quarter
/// after it. `caret_x` and `text_w` are measured from the start of the text.
pub fn edit_scroll(scroll: f32, caret_x: f32, text_w: f32, view_w: f32) -> f32 {
    if view_w <= 1.0 {
        return 0.0;
    }
    if caret_x < scroll {
        (caret_x - view_w / 4.0).max(0.0)
    } else if caret_x > scroll + view_w {
        (caret_x - view_w * 0.75).min((text_w - view_w).max(0.0)).max(0.0)
    } else {
        scroll.max(0.0)
    }
}

impl Field {
    pub fn new(s: &str) -> Field {
        let n = s.encode_utf16().count();
        Field { text: s.to_string(), caret: n, anchor: n, ..Default::default() }
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
    fn attrs(&self) -> Vec<(bool, bool)> {
        let w: Vec<u16> = self.text.encode_utf16().collect();
        word_attrs(&w)
    }
    pub fn sel_range(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
    pub fn has_sel(&self) -> bool {
        self.caret != self.anchor
    }
    pub fn all_selected(&self) -> bool {
        let n = self.len16();
        n > 0 && self.sel_range() == (0, n)
    }
    /// Move the caret (and the anchor with it unless `keep_anchor`): every caret move goes through here.
    fn set_caret(&mut self, to: usize, keep_anchor: bool) {
        self.caret = to.min(self.len16());
        if !keep_anchor {
            self.anchor = self.caret;
        }
        self.lead = false;
        self.hold = false;
    }
    /// Set the selection as EM_SETSEL does: nothing scrolls.
    pub fn select(&mut self, anchor: usize, caret: usize) {
        let n = self.len16();
        self.anchor = anchor.min(n);
        self.caret = caret.min(n);
        self.lead = false;
        self.word = None;
        self.hold = true;
    }
    /// Where the text is scrolled to once the caret is at `caret_x` (see `edit_scroll`).
    pub fn scroll_for(&self, caret_x: f32, text_w: f32, view_w: f32) -> f32 {
        if self.hold { self.scroll.max(0.0) } else { edit_scroll(self.scroll, caret_x, text_w, view_w) }
    }
    pub fn select_all(&mut self) {
        self.select(0, usize::MAX);
    }
    pub fn selected(&self) -> String {
        let (a, b) = self.sel_range();
        self.text[self.byte_at(a)..self.byte_at(b)].to_string()
    }

    // ---- editing: everything that changes the text goes through `replace_sel`, which keeps EDIT's undo buffer
    /// Replace the selection (or insert at the caret) the way EM_REPLACESEL does. The undo buffer holds ONE step:
    /// a run of typing, a run of Backspace / Delete, or a replaced selection.
    pub fn replace_sel(&mut self, s: &str, can_undo: bool) {
        let n = self.len16();
        self.caret = self.caret.min(n);
        self.anchor = self.anchor.min(n);
        let (a, b) = self.sel_range();
        let ins = s.encode_utf16().count();
        if a == b && ins == 0 {
            return;
        }
        let (ba, bb) = (self.byte_at(a), self.byte_at(b));
        if !can_undo {
            self.clear_undo();
        } else {
            if b > a {
                let cut = &self.text[ba..bb];
                if ins == 0 && self.undo_ins == 0 && !self.undo_text.is_empty() && a == self.undo_pos {
                    // Delete pressed again (nothing typed in its place): the removed text grows to the right
                    self.undo_text.push_str(cut);
                } else if ins == 0 && self.undo_ins == 0 && !self.undo_text.is_empty() && b == self.undo_pos {
                    // Backspace pressed again: it grows to the left
                    self.undo_text.insert_str(0, cut);
                    self.undo_pos = a;
                } else {
                    self.undo_text = cut.to_string();
                    self.undo_pos = a;
                }
                // a deletion ends the run of typing that came before it
                self.undo_ins = 0;
            }
            if ins > 0 {
                if a == self.undo_pos || (self.undo_ins > 0 && a == self.undo_pos + self.undo_ins) {
                    // typed where text was just removed, or right after the last typed letter: the same step
                    self.undo_ins += ins;
                } else {
                    self.undo_pos = a;
                    self.undo_ins = ins;
                    self.undo_text.clear();
                }
            }
        }
        self.text.replace_range(ba..bb, s);
        self.caret = a + ins;
        self.anchor = self.caret;
        self.lead = false;
        self.word = None;
        self.hold = false;
    }
    pub fn can_undo(&self) -> bool {
        self.undo_ins > 0 || !self.undo_text.is_empty()
    }
    pub fn clear_undo(&mut self) {
        self.undo_ins = 0;
        self.undo_text.clear();
    }
    /// EDIT's Undo: takes the last step back and selects what came back; Undo again redoes it (there is no
    /// separate Redo and no second step).
    pub fn undo(&mut self) -> bool {
        if !self.can_undo() {
            return false;
        }
        let n = self.len16();
        let back = std::mem::take(&mut self.undo_text);
        let (a, b) = (self.undo_pos.min(n), (self.undo_pos + self.undo_ins).min(n));
        self.undo_pos = a;
        self.undo_ins = 0;
        self.anchor = a;
        self.caret = b;
        self.replace_sel(&back, true);
        self.anchor = self.undo_pos;
        self.caret = (self.undo_pos + self.undo_ins).min(self.len16());
        self.hold = false;
        true
    }
    pub fn delete_sel(&mut self) {
        self.replace_sel("", true);
    }
    pub fn insert(&mut self, s: &str) {
        let s: String = s.chars().filter(|c| *c != '\r' && *c != '\n' && *c != '\t').collect();
        self.replace_sel(&s, true);
    }

    // ---- moving
    fn prev(&self, i: usize) -> usize {
        // step over a surrogate pair
        let w: Vec<u16> = self.text.encode_utf16().collect();
        if i == 0 {
            return 0;
        }
        let i = i.min(w.len());
        if i >= 2 && (0xDC00..0xE000).contains(&w[i - 1]) { i - 2 } else { i - 1 }
    }
    fn next(&self, i: usize) -> usize {
        let w: Vec<u16> = self.text.encode_utf16().collect();
        if i >= w.len() {
            return w.len();
        }
        if (0xD800..0xDC00).contains(&w[i]) { (i + 2).min(w.len()) } else { i + 1 }
    }
    /// Ctrl+Left: the start of the word before the caret.
    pub fn word_left(&self, i: usize) -> usize {
        word_before(&self.attrs(), i.min(self.len16())).0
    }
    /// Ctrl+Right: the start of the word after the one at the caret.
    pub fn word_right(&self, i: usize) -> usize {
        word_after(&self.attrs(), i).1
    }
    /// The word a double-click selects when the click put the caret at `i`.
    pub fn word_at(&self, i: usize) -> (usize, usize) {
        let at = self.attrs();
        if at.is_empty() { (0, 0) } else { chunk_of(&at, i.max(1) - 1) }
    }

    // ---- mouse
    /// The button went down at unit `i`: the caret goes there, Shift keeps the anchor. `lead` = the pointer was on
    /// the near half of the character at `i`.
    pub fn click(&mut self, i: usize, shift: bool, lead: bool) {
        let n = self.len16();
        self.caret = i.min(n);
        if !shift {
            self.anchor = self.caret;
        }
        self.lead = lead && self.caret < n;
        self.word = None;
        self.hold = false;
    }
    /// The second press of a double-click (after `click` put the caret under the pointer): select that word with
    /// its spaces. With Shift the selection grows to the word's far end instead.
    pub fn double_click(&mut self, shift: bool) {
        let (a, b) = self.word_at(self.caret);
        if !shift {
            self.anchor = a;
            self.caret = b;
        } else if self.caret <= self.anchor {
            self.caret = a;
        } else {
            self.caret = b;
        }
        self.lead = false;
        self.hold = false;
        self.word = Some((a, b));
    }
    /// The pointer moved to unit `i` with the button down: the selection follows it; after a double-click it
    /// grows by whole words and never loses the first word.
    pub fn drag(&mut self, i: usize, lead: bool) {
        let n = self.len16();
        let i = i.min(n);
        self.hold = false;
        match self.word {
            Some((a, b)) => {
                let at = self.attrs();
                if i <= a {
                    self.anchor = b;
                    self.caret = word_after(&at, i).0.min(a);
                } else {
                    self.anchor = a;
                    self.caret = word_before(&at, i).1.max(b);
                }
                self.lead = false;
            }
            None => {
                self.caret = i;
                self.lead = lead && i < n;
            }
        }
    }
    /// One step of the scrolling drag (the pointer is outside the box, to the left when `left`): the selection
    /// grows by one character past `edge`, the unit at that end of the box.
    pub fn drag_out(&mut self, edge: usize, left: bool) {
        let to = if left { self.prev(edge) } else { self.next(edge) };
        self.drag(to, false);
    }
    pub fn end_drag(&mut self) {
        self.word = None;
    }

    /// Editing keys. Returns true when the key was used. Clipboard keys are handled by the caller.
    pub fn key(&mut self, vk: u16, ctrl: bool, shift: bool) -> bool {
        let n = self.len16();
        self.caret = self.caret.min(n);
        self.anchor = self.anchor.min(n);
        match vk {
            // Left, and Up (a one-line EDIT moves the same way on both)
            0x25 | 0x26 => {
                if ctrl {
                    let to = self.word_left(self.caret);
                    self.set_caret(to, shift);
                } else if self.has_sel() && !shift {
                    // a selection collapses to its start
                    let a = self.sel_range().0;
                    self.set_caret(a, false);
                } else {
                    let to = self.prev(self.caret);
                    self.set_caret(to, shift);
                }
            }
            0x27 | 0x28 => {
                if ctrl {
                    let to = self.word_right(self.caret);
                    self.set_caret(to, shift);
                } else if self.has_sel() && !shift {
                    let b = self.sel_range().1;
                    self.set_caret(b, false);
                } else {
                    let to = self.next(self.caret);
                    self.set_caret(to, shift);
                }
            }
            0x24 => self.set_caret(0, shift),
            0x23 => self.set_caret(n, shift),
            0x08 => {
                if self.has_sel() {
                    self.delete_sel();
                } else if self.caret > 0 {
                    // Ctrl+Backspace removes back to the start of the word (EDIT itself types a box character
                    // here, an old bug nobody wants copied)
                    self.anchor = if ctrl { self.word_left(self.caret) } else { self.prev(self.caret) };
                    self.delete_sel();
                }
            }
            0x2E => {
                if self.has_sel() {
                    self.delete_sel();
                } else if self.caret < n && !(ctrl && shift) {
                    // Ctrl+Delete removes up to where Ctrl+Right would go
                    self.anchor = if ctrl { self.word_right(self.caret) } else { self.next(self.caret) };
                    self.delete_sel();
                }
            }
            0x41 if ctrl => self.select_all(),
            0x5A if ctrl => {
                self.undo();
            }
            _ => return false,
        }
        self.word = None;
        true
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;

    fn f(s: &str, anchor: usize, caret: usize) -> Field {
        let mut f = Field::new(s);
        f.anchor = anchor;
        f.caret = caret;
        f
    }
    fn breaks(s: &str) -> Vec<usize> {
        let w: Vec<u16> = s.encode_utf16().collect();
        word_attrs(&w).iter().enumerate().filter(|(_, a)| a.0).map(|(i, _)| i).collect()
    }
    const DOC: &str = "New Text Document.txt";

    // The expected numbers in these tests were read from a real EDIT control (comctl32, Windows 11 26200) with
    // EM_GETSEL after sending it the same clicks and keys.
    #[test]
    fn words_break_where_the_edit_control_breaks() {
        assert_eq!(breaks(DOC), [4, 9]);
        // a dash ends a word, a dot or an underscore does not
        assert_eq!(breaks("my-file_name v2.final.tar.gz"), [3, 13]);
        assert_eq!(breaks("a(b)c [d] {e}+f=g,h;i&j@k#l"), [1, 6, 10]);
        assert_eq!(breaks("one  two   three"), [5, 11]);
        assert_eq!(breaks("it's 50% off!~x"), [5, 9, 13]);
        // Persian: "گزارش نهایی.docx", and Persian inside Latin: "ab سلام cd.txt"
        assert_eq!(breaks("\u{6af}\u{632}\u{627}\u{631}\u{634} \u{646}\u{647}\u{627}\u{6cc}\u{6cc}.docx"), [6]);
        assert_eq!(breaks("ab \u{633}\u{644}\u{627}\u{645} cd.txt"), [3, 8]);
    }

    #[test]
    fn ctrl_arrows_move_like_the_edit_control() {
        let right = |s: &str| (0..=s.encode_utf16().count()).map(|i| Field::new(s).word_right(i)).collect::<Vec<_>>();
        let left = |s: &str| (0..=s.encode_utf16().count()).map(|i| Field::new(s).word_left(i)).collect::<Vec<_>>();
        // from the space after a word, Ctrl+Right jumps over the whole next word (3 -> 9, 8 -> 21)
        assert_eq!(right(DOC), [4, 4, 4, 9, 9, 9, 9, 9, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21]);
        assert_eq!(left(DOC), [0, 0, 0, 0, 0, 4, 4, 4, 4, 4, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]);
        assert_eq!(right("one  two   three"), [5, 5, 5, 11, 11, 11, 11, 11, 16, 16, 16, 16, 16, 16, 16, 16, 16]);
        assert_eq!(left("one  two   three"), [0, 0, 0, 0, 0, 0, 5, 5, 5, 5, 5, 5, 11, 11, 11, 11, 11]);
        assert_eq!(right("my-file_name v2.final.tar.gz")[..14], [3, 3, 3, 13, 13, 13, 13, 13, 13, 13, 13, 13, 28, 28]);
        assert_eq!(right("it's 50% off!~x"), [5, 5, 5, 5, 9, 9, 9, 9, 13, 13, 13, 13, 13, 15, 15, 15]);
        assert_eq!(right("ab \u{633}\u{644}\u{627}\u{645} cd.txt"), [3, 3, 8, 8, 8, 8, 8, 14, 14, 14, 14, 14, 14, 14, 14]);
    }

    #[test]
    fn keys_with_a_selection() {
        let key = |a: usize, c: usize, vk: u16, ctrl: bool, shift: bool| {
            let mut x = f(DOC, a, c);
            assert!(x.key(vk, ctrl, shift));
            x.sel_range()
        };
        // Left / Right fold the selection to its start / end; with Ctrl they move from the caret
        assert_eq!(key(0, 17, 0x25, false, false), (0, 0));
        assert_eq!(key(0, 17, 0x27, false, false), (17, 17));
        assert_eq!(key(4, 8, 0x25, false, false), (4, 4));
        assert_eq!(key(4, 8, 0x27, false, false), (8, 8));
        assert_eq!(key(4, 8, 0x25, true, false), (4, 4));
        assert_eq!(key(4, 8, 0x27, true, false), (21, 21));
        assert_eq!(key(4, 8, 0x24, false, false), (0, 0));
        assert_eq!(key(4, 8, 0x23, false, false), (21, 21));
        assert_eq!(key(6, 6, 0x24, false, true), (0, 6));
        assert_eq!(key(6, 6, 0x23, false, true), (6, 21));
        assert_eq!(key(6, 6, 0x25, true, true), (4, 6));
        assert_eq!(key(6, 6, 0x27, true, true), (6, 9));
        // Up / Down are Left / Right in a one-line box
        assert_eq!(key(6, 6, 0x26, false, false), (5, 5));
        assert_eq!(key(6, 6, 0x28, false, false), (7, 7));
        assert_eq!(key(6, 6, 0x26, false, true), (5, 6));
        assert_eq!(key(6, 6, 0x41, true, false), (0, 21));
        // F2 inside the box does nothing
        assert!(!f(DOC, 6, 6).key(0x71, false, false));
    }

    #[test]
    fn word_delete() {
        let del = |c: usize, vk: u16| {
            let mut x = f(DOC, c, c);
            x.key(vk, true, false);
            (x.text.clone(), x.caret)
        };
        // Ctrl+Delete removes up to where Ctrl+Right goes (EDIT: 0 -> "Text Document.txt", 3 -> "NewDocument.txt")
        assert_eq!(del(0, 0x2E), ("Text Document.txt".into(), 0));
        assert_eq!(del(3, 0x2E), ("NewDocument.txt".into(), 3));
        assert_eq!(del(8, 0x2E), ("New Text".into(), 8));
        assert_eq!(del(21, 0x2E), (DOC.into(), 21));
        // Ctrl+Backspace removes back to the start of the word
        assert_eq!(del(9, 0x08), ("New Document.txt".into(), 4));
        assert_eq!(del(21, 0x08), ("New Text ".into(), 9));
        assert_eq!(del(0, 0x08), (DOC.into(), 0));
        // with a selection both remove just the selection
        let mut x = f(DOC, 4, 8);
        x.key(0x2E, true, false);
        assert_eq!((x.text.as_str(), x.caret), ("New  Document.txt", 4));
    }

    #[test]
    fn clicks_and_double_clicks() {
        let mut x = f(DOC, 0, 17);
        x.click(6, false, false);
        assert_eq!((x.anchor, x.caret), (6, 6));
        // Shift+click keeps the anchor
        let mut x = f(DOC, 4, 8);
        x.click(12, true, false);
        assert_eq!((x.anchor, x.caret), (4, 12));
        x.click(1, true, false);
        assert_eq!((x.anchor, x.caret), (4, 1));
        // a double-click takes the word with its spaces; the caret ends at the far end. A caret right at the start
        // of a word (4, 9) belongs to the word before it, as in the EDIT control.
        for (c, want) in [(0, (0, 4)), (3, (0, 4)), (4, (0, 4)), (6, (4, 9)), (9, (4, 9)), (12, (9, 21)), (17, (9, 21)), (21, (9, 21))] {
            let mut x = f(DOC, 0, 0);
            x.click(c, false, false);
            x.double_click(false);
            assert_eq!((x.anchor, x.caret), want, "double-click at {c}");
        }
        // Shift+double-click: the selection grows to the far end of the word under the pointer
        let mut x = f(DOC, 1, 1);
        x.click(12, true, false);
        x.double_click(true);
        assert_eq!((x.anchor, x.caret), (1, 21));
        let mut x = f(DOC, 20, 20);
        x.click(6, true, false);
        x.double_click(true);
        assert_eq!((x.anchor, x.caret), (20, 4));
    }

    #[test]
    fn dragging() {
        // a plain drag selects by character, either way
        let mut x = f(DOC, 0, 0);
        x.click(6, false, false);
        x.drag(12, false);
        assert_eq!(x.sel_range(), (6, 12));
        x.drag(2, false);
        assert_eq!((x.anchor, x.caret), (6, 2));
        // after a double-click the drag selects whole words and keeps the first word
        let dd = |start: usize, to: &[usize]| {
            let mut x = f(DOC, 0, 0);
            x.click(start, false, false);
            x.double_click(false);
            for t in to {
                x.drag(*t, false);
            }
            (x.anchor, x.caret)
        };
        assert_eq!(dd(6, &[12]), (4, 21));
        assert_eq!(dd(12, &[1]), (21, 0));
        assert_eq!(dd(6, &[7]), (4, 9));
        assert_eq!(dd(6, &[9]), (4, 9));
        assert_eq!(dd(6, &[3]), (9, 4));
        assert_eq!(dd(6, &[4]), (9, 4));
        assert_eq!(dd(12, &[1, 6]), (21, 4));
        assert_eq!(dd(1, &[12]), (0, 21));
        // the word mode ends with the button
        let mut x = f(DOC, 0, 0);
        x.click(6, false, false);
        x.double_click(false);
        x.end_drag();
        x.click(6, false, false);
        x.drag(7, false);
        assert_eq!(x.sel_range(), (6, 7));
    }

    #[test]
    fn undo_is_the_edit_controls_one_step() {
        let ty = |x: &mut Field, s: &str| s.chars().for_each(|c| x.insert(&c.to_string()));
        // a run of typing is one step; Undo again redoes it and selects what came back
        let mut x = f("abc def", 7, 7);
        ty(&mut x, "XYZ");
        assert!(x.undo());
        assert_eq!((x.text.as_str(), x.sel_range()), ("abc def", (7, 7)));
        assert!(x.undo());
        assert_eq!((x.text.as_str(), x.sel_range()), ("abc defXYZ", (7, 10)));
        // typing somewhere else starts a new step: only the last run comes back
        let mut x = f("abc def", 7, 7);
        ty(&mut x, "XY");
        x.key(0x24, false, false);
        ty(&mut x, "Q");
        x.undo();
        assert_eq!((x.text.as_str(), x.sel_range()), ("abc defXY", (0, 0)));
        x.undo();
        assert_eq!((x.text.as_str(), x.sel_range()), ("Qabc defXY", (0, 1)));
        // a typed-over selection comes back whole (the rename box: type over the name, Ctrl+Z)
        let mut x = f(DOC, 0, 17);
        ty(&mut x, "abc");
        assert_eq!(x.text, "abc.txt");
        x.undo();
        assert_eq!((x.text.as_str(), x.sel_range()), (DOC, (0, 17)));
        // typed over, then Backspace: only the Backspace is undone
        let mut x = f("abc def", 0, 3);
        ty(&mut x, "Q");
        x.key(0x08, false, false);
        assert_eq!(x.text, " def");
        x.undo();
        assert_eq!((x.text.as_str(), x.sel_range()), ("Q def", (0, 1)));
        x.undo();
        assert_eq!((x.text.as_str(), x.sel_range()), (" def", (0, 0)));
        // Backspace twice then typing there: one step
        let mut x = f("abc def", 3, 3);
        x.key(0x08, false, false);
        x.key(0x08, false, false);
        ty(&mut x, "ZZ");
        assert_eq!(x.text, "aZZ def");
        x.undo();
        assert_eq!((x.text.as_str(), x.sel_range()), ("abc def", (1, 3)));
        // Delete twice
        let mut x = f("abc def", 1, 1);
        x.key(0x2E, false, false);
        x.key(0x2E, false, false);
        assert_eq!(x.text, "a def");
        x.key(0x5A, true, false);
        assert_eq!((x.text.as_str(), x.sel_range()), ("abc def", (1, 3)));
        // nothing to undo in a fresh box
        assert!(!Field::new("abc").undo());
    }

    #[test]
    fn scrolling_like_the_edit_control() {
        // inside the box: nothing moves
        assert_eq!(edit_scroll(0.0, 50.0, 400.0, 94.0), 0.0);
        assert_eq!(edit_scroll(30.0, 124.0, 400.0, 94.0), 30.0);
        // past the right edge: the caret lands a quarter of the box before the edge (EDIT: x 97 -> 70 of 94)
        assert_eq!(edit_scroll(0.0, 97.0, 400.0, 94.0), 97.0 - 70.5);
        // but never past the end of the text
        assert_eq!(edit_scroll(280.0, 380.0, 400.0, 94.0), 306.0);
        // past the left edge: a quarter after it (EDIT: x -6 -> 23 of 94), never before the start
        assert_eq!(edit_scroll(100.0, 94.0, 400.0, 94.0), 94.0 - 23.5);
        assert_eq!(edit_scroll(20.0, 10.0, 400.0, 94.0), 0.0);
        // a name that fits never scrolls
        assert_eq!(edit_scroll(0.0, 60.0, 60.0, 94.0), 0.0);
        // a selection set by the program (the rename box opening) leaves a long name at its start; the first
        // key that moves the caret brings the caret into view
        let mut x = Field::new("a long name that does not fit.txt");
        x.select(0, 29);
        assert_eq!(x.scroll_for(300.0, 330.0, 94.0), 0.0);
        x.key(0x23, false, true);
        assert_eq!(x.scroll_for(330.0, 330.0, 94.0), 236.0);
        x.select_all();
        x.scroll = 236.0;
        assert_eq!(x.scroll_for(330.0, 330.0, 94.0), 236.0);
    }

    #[test]
    fn surrogate_pairs_and_filters() {
        let mut x = Field::new("a\u{1F600}b");
        assert_eq!(x.len16(), 4);
        x.key(0x25, false, false);
        x.key(0x25, false, false);
        assert_eq!(x.caret, 1);
        x.key(0x2E, false, false);
        assert_eq!(x.text, "ab");
        x.insert("x\r\ny\tz");
        assert_eq!(x.text, "axyzb");
        // stale positions never panic
        let mut x = f("abc", 9, 12);
        x.key(0x23, false, true);
        x.insert("d");
        assert_eq!(x.text, "abcd");
    }
}
