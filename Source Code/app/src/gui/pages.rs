//! Startup apps page and Settings page.
use super::app::*;
use super::gfx::{Align, C, Font, Rect};
use super::model;
use super::paint::{Btn, exe_of, keep_from};
use std::cell::RefCell;
use std::collections::HashMap;

/// How long a switched startup row stays in place (its switch animation runs first).
pub const STARTUP_HOLD: std::time::Duration = std::time::Duration::from_millis(700);

/// Windows-style impact words from measured use: private memory and average CPU (% of one core).
pub fn impact_level(mem: u64, avg_cpu: f64) -> &'static str {
    if mem >= 300 << 20 || avg_cpu >= 2.0 {
        "High"
    } else if mem >= 80 << 20 || avg_cpu >= 0.3 {
        "Medium"
    } else {
        "Low"
    }
}

/// What the details pane of a startup entry says (read once when it opens).
#[derive(Clone, Debug)]
pub struct StInfo {
    pub what: String,
    pub maker: String,
    pub off: String,
    pub service: bool,
    pub path: String,
}

/// Facts about a startup entry without AI: Windows' own service description, the built-in list of known programs,
/// else the file's version info.
pub fn startup_info(e: &crate::startup::Entry) -> StInfo {
    let path = exe_of(&e.command);
    let file = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (fdesc, company) = if path.is_empty() { (String::new(), String::new()) } else { crate::procs::version_strings(&path) };
    let known = crate::explain::lookup(&file);
    let maker = known.as_ref().map(|k| k.0.clone()).filter(|m| !m.is_empty()).unwrap_or(company);
    if e.source == "Service" {
        let svc = crate::services::list().into_iter().find(|s| s.display == e.name);
        let desc = svc.as_ref().and_then(|s| crate::services::details(&s.name)).map(|d| d.2).unwrap_or_default();
        let what = if !desc.is_empty() {
            desc
        } else if let Some(k) = &known {
            k.2.clone()
        } else if !fdesc.is_empty() {
            fdesc
        } else {
            "No description from Windows or from the file.".into()
        };
        let key = svc.map(|s| format!(" (service name: {})", s.name)).unwrap_or_default();
        let off = format!("A background service{key} that Windows starts by itself. If it did not start, its work (updates, sync, licence checks, device helpers) would wait until a program starts it.");
        return StInfo { what, maker, off, service: true, path };
    }
    let what = known.as_ref().map(|k| k.2.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| if fdesc.is_empty() { "No description in the file.".into() } else { fdesc });
    let base = if e.source == "Scheduled task" { "Task Scheduler no longer runs it when you sign in. You can still open it yourself." } else { "It no longer opens by itself when you sign in. You can still open it yourself." };
    let off = match crate::insight::stop_effect(&file) {
        Some(x) if !x.is_empty() => format!("{base} While it is not running, {}.", x.trim_start_matches("Ending it is safe, but ").trim_end_matches('.')),
        _ => base.to_string(),
    };
    StInfo { what, maker, off, service: false, path }
}

pub const STARTUP_SORT: [&str; 3] = ["Impact", "Name", "Starts from"];

/// Rows a section shows before "Show N more".
pub const ST_TOP: usize = 5;

/// One drawn row of the startup page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StRow {
    Head,
    Item(usize),
    /// "Show all (N)" (N = the section size)
    More(usize),
    Less,
}

/// Startup page sections, in this order; each has on items first, then off items.
pub const ST_SECTIONS: [&str; 3] = ["Apps", "Scheduled tasks", "Services"];
pub const ST_HEAD_H: f32 = 36.0;
pub const ST_ROW_H: f32 = 48.0;

/// 0 = Apps (registry and Startup folders), 1 = scheduled tasks, 2 = services.
pub fn startup_section(e: &crate::startup::Entry) -> u8 {
    match e.source {
        "Scheduled task" => 1,
        "Service" => 2,
        _ => 0,
    }
}

thread_local! {
    /// startup entry name -> (generation, what it runs now)
    static NOW_CACHE: RefCell<HashMap<String, (usize, Vec<String>)>> = RefCell::new(HashMap::new());
}

/// File names a startup entry starts: the exe itself, or what a script / shortcut refers to.
fn referenced(command: &str) -> Vec<String> {
    let exe = exe_of(command);
    let low = exe.to_lowercase();
    let file = low.rsplit(['\\', '/']).next().unwrap_or("").to_string();
    let script = [".vbs", ".lnk", ".cmd", ".bat", ".ps1", ".js"].iter().any(|e| low.ends_with(e));
    if !script {
        return vec![file];
    }
    let Ok(bytes) = std::fs::read(&exe) else { return vec![] };
    if bytes.len() > 256 * 1024 {
        return vec![];
    }
    let mut text = String::from_utf8_lossy(&bytes).to_lowercase();
    let u16s: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    text.push(' ');
    text.push_str(&String::from_utf16_lossy(&u16s).to_lowercase());
    let mut out: Vec<String> = vec![];
    for tok in text.split(|c: char| !(c.is_alphanumeric() || "._-\\/:".contains(c))) {
        let name = tok.rsplit(['\\', '/']).next().unwrap_or("");
        let good = [".exe", ".py", ".pyw", ".js", ".ps1", ".mjs"].iter().any(|e| name.ends_with(e) && name.len() > e.len());
        let shell = ["wscript.exe", "cscript.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "conhost.exe"].contains(&name);
        if good && !shell && !out.iter().any(|o| o == name) {
            out.push(name.to_string());
        }
    }
    // scripts and modules first: they tell processes of the same exe apart
    out.sort_by_key(|n| n.ends_with(".exe"));
    out
}

impl App {
    /// The running processes of a startup entry: every copy of its own exe ("direct"), else the first process that
    /// runs the program or script its command (or shortcut) names.
    pub fn startup_procs(&self, e: &crate::startup::Entry) -> (Vec<&crate::procs::Proc>, bool) {
        let refs = NOW_CACHE.with(|c| {
            let gn = self.startup.len();
            if let Some((g, v)) = c.borrow().get(&e.name)
                && *g == gn
            {
                return v.clone();
            }
            let v = referenced(&e.command);
            c.borrow_mut().insert(e.name.clone(), (gn, v.clone()));
            v
        });
        let direct = !exe_of(&e.command).to_lowercase().ends_with(".lnk") && refs.len() == 1 && refs[0].ends_with(".exe") && exe_of(&e.command).to_lowercase().ends_with(&refs[0]);
        for r in &refs {
            let hits: Vec<&crate::procs::Proc> = if r.ends_with(".exe") {
                self.procs.iter().filter(|p| p.name.to_lowercase() == *r).collect()
            } else {
                self.procs.iter().filter(|p| self.cmdlines.get(&p.pid).map(|c| c.to_lowercase().contains(r.as_str())).unwrap_or(false)).collect()
            };
            if !hits.is_empty() {
                return (if direct { hits } else { hits.into_iter().take(1).collect() }, direct);
            }
        }
        (vec![], direct)
    }

    /// "Running · 347 MB", "Running · python.exe · 81 MB" or "Not running".
    pub fn startup_now(&self, e: &crate::startup::Entry) -> (bool, String) {
        let (hits, direct) = self.startup_procs(e);
        let Some(p) = hits.first() else { return (false, "Not running".into()) };
        let mem: u64 = hits.iter().map(|p| p.mem).sum();
        (true, if direct { format!("Running · {}", model::fmt_mem(mem)) } else { format!("Running · {} · {}", p.name, model::fmt_mem(mem)) })
    }

    /// Startup impact from what the program really uses now: High / Medium / Low, or "Not measured" when it is not
    /// running. Memory (private working set) and its average CPU since it started; the text says the numbers.
    pub fn startup_impact(&self, e: &crate::startup::Entry) -> (&'static str, String) {
        let (hits, _) = self.startup_procs(e);
        if hits.is_empty() {
            return ("Not measured", "Not running now, so there is nothing to measure.".into());
        }
        let now = unsafe {
            let ft = windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime();
            ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
        };
        let mem: u64 = hits.iter().map(|p| p.mem).sum();
        let cpu_s: f64 = hits.iter().map(|p| p.cpu_time as f64 / 1e7).sum();
        let age_s = hits.iter().map(|p| ((now - p.create_time) as f64 / 1e7).max(1.0)).fold(1.0, f64::max);
        let avg = cpu_s / age_s * 100.0;
        let io_mb = hits.iter().map(|p| p.io_bytes).sum::<u64>() as f64 / 1_048_576.0;
        (impact_level(mem, avg), format!("Uses {} of memory and {:.1}% of one CPU core on average since it started {} ago; {:.0} MB read and written.", model::fmt_mem(mem), avg, model::fmt_dur(age_s as u64), io_mb))
    }

    pub fn startup_order(&self) -> Vec<usize> {
        // the search box filters this page too (name and command)
        let toks = model::tokens(&self.query);
        let mut v: Vec<usize> = (0..self.startup.len()).filter(|&i| toks.is_empty() || model::startup_matches(&self.startup[i], &toks)).collect();
        let s = &self.startup;
        // a just-switched row keeps its place until its switch has moved
        let held = |i: usize| match self.startup_hold {
            Some((h, was, at)) if h == i && at.elapsed() < STARTUP_HOLD => was,
            _ => s[i].enabled,
        };
        // section, then on before off, then the order picked in the Sort menu
        match self.startup_sort {
            1 => v.sort_by_key(|&i| (startup_section(&s[i]), !held(i), s[i].name.to_lowercase())),
            2 => v.sort_by_key(|&i| (startup_section(&s[i]), !held(i), source_label(s[i].source), s[i].name.to_lowercase())),
            // impact: High, Medium, Low, Not measured; the bigger memory first inside a level (8 MB steps, so rows do
            // not swap on small changes), then the name
            _ => {
                let rank: std::collections::HashMap<usize, (u8, std::cmp::Reverse<u64>)> = v.iter().map(|&i| (i, self.startup_rank(&s[i]))).collect();
                v.sort_by_key(|&i| (startup_section(&s[i]), !held(i), rank[&i], s[i].name.to_lowercase()))
            }
        }
        v
    }

    /// Sort key by impact: (level 0 High .. 3 Not measured, memory in 8 MB steps, larger first).
    pub fn startup_rank(&self, e: &crate::startup::Entry) -> (u8, std::cmp::Reverse<u64>) {
        let (hits, _) = self.startup_procs(e);
        if hits.is_empty() {
            return (3, std::cmp::Reverse(0));
        }
        let (level, _) = self.startup_impact(e);
        let r = match level {
            "High" => 0,
            "Medium" => 1,
            _ => 2,
        };
        (r, std::cmp::Reverse(hits.iter().map(|p| p.mem).sum::<u64>() >> 23))
    }

    /// The startup list as drawn: (y from the top, row, section). A folded section shows only its header; a section
    /// shows its top ST_TOP rows and "Show all (N)" unless it is open in full ("Show fewer"). A search shows all matches.
    pub fn startup_rows(&self) -> Vec<(f32, StRow, u8)> {
        let searching = !self.query.trim().is_empty();
        let order = self.startup_order();
        let mut out = vec![];
        let mut y = 0.0;
        for sec in 0..3u8 {
            let items: Vec<usize> = order.iter().copied().filter(|&i| startup_section(&self.startup[i]) == sec).collect();
            if items.is_empty() {
                continue;
            }
            out.push((y, StRow::Head, sec));
            y += ST_HEAD_H;
            if self.prefs.st_fold[sec as usize] && !searching {
                continue;
            }
            let all = searching || self.prefs.st_all[sec as usize];
            let shown = if all { items.len() } else { items.len().min(ST_TOP) };
            for &i in &items[..shown] {
                out.push((y, StRow::Item(i), sec));
                y += ST_ROW_H;
            }
            if !searching && items.len() > ST_TOP {
                out.push((y, if all { StRow::Less } else { StRow::More(items.len()) }, sec));
                y += ST_HEAD_H;
            }
        }
        out
    }

    pub fn open_st_pane(&mut self, i: usize) {
        if let Some(e) = self.startup.get(i) {
            self.st_pane = Some((i, startup_info(e)));
        }
        self.invalidate();
    }

    /// The details pane of the selected startup entry: what it is, who made it, its measured impact, what turning
    /// it off changes (and why a service has no switch).
    fn paint_st_pane(&mut self, r: Rect) {
        let t = self.t;
        let Some((i, info)) = self.st_pane.clone() else { return };
        let Some(e) = self.startup.get(i).cloned() else {
            self.st_pane = None;
            return;
        };
        self.gfx.card(r, 8.0, t.layer, t.layer_s);
        self.hit(r, Hit::Block);
        self.gfx.clip(r.inset(1.0, 1.0));
        let (x, w) = (r.x + 20.0, r.w - 40.0);
        let mut y = r.y + 18.0;
        let cr = Rect::new(r.r() - 16.0 - 32.0 + 6.0, y - 2.0, 32.0, 32.0);
        self.hp(cr, 4.0, Hit::StPaneClose);
        self.gfx.glyph(0xE711, cr, 11.0, t.t2);
        self.hit(cr, Hit::StPaneClose);
        self.tip(cr, "Close (Esc)".to_string());
        let glyph = if e.source == "Service" { 0xE713 } else if e.source == "Scheduled task" { 0xE916 } else { 0xE8A5 };
        self.draw_icon(&info.path, Rect::new(x, y + 2.0, 24.0, 24.0), glyph);
        let shown = if e.source == "Scheduled task" { e.name.rsplit('\\').next().unwrap_or(&e.name).to_string() } else { e.name.clone() };
        let th = self.gfx.para(&shown, Rect::new(x + 36.0, y, w - 36.0 - 30.0, 80.0), Font::Ui, 15.0, 600, Align::Left, t.t1, 20.0);
        y += th.max(20.0) + 2.0;
        self.gfx.text(source_label(e.source), Rect::new(x + 36.0, y, w - 36.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
        y += 30.0;
        let (level, detail) = self.startup_impact(&e);
        let maker = if info.maker.is_empty() { "Unknown (the file names no company)".to_string() } else { info.maker.clone() };
        let rows = [("What it is", info.what.clone()), ("Made by", maker), ("Startup impact", format!("{level}. {detail}")), ("If it is off", info.off.clone()), ("Command", e.command.clone())];
        for (label, body) in rows {
            self.gfx.text(label, Rect::new(x, y, w, 18.0), Font::Ui, 12.0, 600, Align::Left, t.t2);
            y += 20.0;
            let h = self.gfx.para(&body, Rect::new(x, y, w, 400.0), Font::Ui, 13.0, 400, Align::Left, t.t1, 19.0);
            y += h + 14.0;
        }
        if info.service {
            self.gfx.para("Switching it off sets it to Manual: Windows no longer starts it at sign-in. It keeps running now; the change applies from its next start. Windows asks once for administrator approval.", Rect::new(x, y, w, 60.0), Font::Ui, 12.5, 400, Align::Left, t.t2, 18.0);
        }
        self.gfx.unclip();
    }

    pub fn paint_startup(&mut self) {
        let t = self.t;
        let pane_w = if self.st_pane.is_some() { PANE_W } else { 0.0 };
        let gap = if pane_w > 0.0 { pane_w + 12.0 } else { 0.0 };
        let layer = Rect::new(self.cx0(), TITLE_H, self.w - self.cx0() - 12.0 - gap, self.h - TITLE_H - 12.0);
        if pane_w > 0.0 {
            self.paint_st_pane(Rect::new(layer.r() + 12.0, layer.y, pane_w, layer.h));
        }
        self.gfx.card(layer, 8.0, t.layer, t.layer_s);
        self.gfx.clip(layer.inset(1.0, 1.0));
        let on = self.startup.iter().filter(|e| e.enabled).count();
        let off = self.startup.len() - on;
        let tw = self.gfx.text_at("Startup apps", layer.x + 20.0, layer.y + 16.0, 32.0, 300.0, Font::Display, 20.0, 600, t.t1);
        self.gfx.text_at(&format!("{on} on · {off} off"), layer.x + 20.0 + tw + 12.0, layer.y + 19.0, 32.0, 200.0, Font::Ui, 13.0, 400, t.t2);
        let label = format!("Sort: {}", STARTUP_SORT[self.startup_sort as usize]);
        let bw = (12.0 + 13.0 + 8.0 + self.gfx.measure(&label, Font::Ui, 13.0, 400) + 8.0 + 9.0 + 12.0).ceil();
        let br = Rect::new(layer.r() - 20.0 - bw, layer.y + 16.0, bw, 32.0);
        self.button(br, "", 0, Btn::Std, false, true, Hit::StSort);
        let any_open = (0..3).any(|k| !self.prefs.st_fold[k]);
        let fl = if any_open { "Collapse all" } else { "Expand all" };
        let fw = (12.0 + self.gfx.measure(fl, Font::Ui, 13.0, 400) + 12.0).ceil();
        self.button(Rect::new(br.x - 8.0 - fw, br.y, fw, 32.0), fl, 0, Btn::Std, false, true, Hit::StFoldAll);
        self.gfx.glyph(0xE8CB, Rect::new(br.x + 12.0, br.y, 13.0, 32.0), 13.0, t.t1);
        self.gfx.text_at(&label, br.x + 33.0, br.y, 32.0, bw, Font::Ui, 13.0, 400, t.t1);
        self.gfx.glyph(0xE70D, Rect::new(br.r() - 12.0 - 9.0, br.y + 1.0, 9.0, 32.0), 9.0, t.t2);
        // header
        let hy = layer.y + 60.0;
        self.gfx.hline(layer.x, layer.r(), hy, t.line);
        self.gfx.hline(layer.x, layer.r(), hy + 31.0, t.line);
        let (x0, x1) = (layer.x + 4.0 + 8.0, layer.r() - 4.0 - 12.0);
        let tog_x = x1 - 96.0;
        // a narrow window drops "Now", then "Starts from", so the name keeps its room
        let (show_imp, show_now, show_src) = (layer.w >= 560.0, layer.w >= 960.0, layer.w >= 740.0);
        let imp_x = if show_imp { tog_x - 104.0 } else { tog_x };
        let now_x = if show_now { imp_x - 220.0 } else { imp_x };
        let src_x = if show_src { now_x - 196.0 } else { now_x };
        for (s, x, on) in [("Name", x0 + 36.0, true), ("Starts from", src_x, show_src), ("Now", now_x, show_now), ("Impact", imp_x, show_imp)] {
            if on {
                self.gfx.text_at(s, x, hy, 32.0, 200.0, Font::Ui, 12.0, 400, t.t2);
            }
        }
        self.gfx.text("At startup", Rect::new(tog_x, hy, 96.0, 32.0), Font::Ui, 12.0, 400, Align::Right, t.t2);
        // rows
        let view = Rect::new(layer.x, hy + 32.0, layer.w, layer.b() - hy - 32.0);
        let order = self.startup_order();
        let rows = self.startup_rows();
        self.content_h[1] = rows.last().map(|r| r.0 + if matches!(r.1, StRow::Item(_)) { ST_ROW_H } else { ST_HEAD_H }).unwrap_or(0.0) + 8.0;
        self.view_h[1] = view.h;
        self.clamp_scroll();
        self.gfx.clip(view);
        if let Some(&(sy, StRow::Item(i), _)) = rows.iter().find(|r| matches!(r.1, StRow::Item(i) if self.sel == Some(Sel::Startup(i)))) {
            self.sel_highlight(Rect::new(layer.x + 4.0, view.y + 4.0 - self.scroll[1], layer.w - 8.0, 48.0), (2 << 56) | i as u64, sy, 0x6200);
        }
        for &(ry, item, sec) in &rows {
            let y = view.y + 4.0 + ry - self.scroll[1];
            if y + 48.0 < view.y || y > view.b() {
                continue;
            }
            let i = match item {
                StRow::Item(i) => i,
                StRow::Head => {
                    // section header: chevron, name (count), how many are on; a click folds or opens the section
                    let (total, on) = order.iter().map(|&k| &self.startup[k]).filter(|e| startup_section(e) == sec).fold((0usize, 0usize), |a, e| (a.0 + 1, a.1 + e.enabled as usize));
                    let folded = self.prefs.st_fold[sec as usize] && self.query.trim().is_empty();
                    let hr = Rect::new(layer.x + 4.0, y, layer.w - 8.0, ST_HEAD_H);
                    let h = Hit::StSecHead(sec);
                    self.hp(hr, 4.0, h);
                    self.hit(hr, h);
                    let hx = layer.x + 16.0;
                    self.gfx.glyph(if folded { 0xE76C } else { 0xE70D }, Rect::new(hx, y + 6.0, 12.0, ST_HEAD_H - 6.0), 10.0, t.t2);
                    let w = self.gfx.text_at(ST_SECTIONS[sec as usize], hx + 20.0, y + 6.0, ST_HEAD_H - 6.0, 300.0, Font::Ui, 13.0, 600, t.t1);
                    self.gfx.text_at(&format!("({total})"), hx + 20.0 + w + 6.0, y + 6.0, ST_HEAD_H - 6.0, 80.0, Font::Ui, 13.0, 400, t.t2);
                    self.gfx.text(&format!("{on} on · {} off", total - on), Rect::new(layer.x, y + 6.0, layer.w - 24.0, ST_HEAD_H - 6.0), Font::Ui, 12.5, 400, Align::Right, t.t2);
                    self.gfx.hline(layer.x + 12.0, layer.r() - 12.0, y + ST_HEAD_H - 1.0, t.line);
                    continue;
                }
                StRow::More(_) | StRow::Less => {
                    // the same row as on the Processes page: "Show all (N)" ▾ / "Show fewer" ▴
                    let (label, down) = match item {
                        StRow::More(n) => (format!("Show all ({n})"), true),
                        _ => ("Show fewer".to_string(), false),
                    };
                    let h = Hit::StMore(sec);
                    let x = x0 + 36.0;
                    let lw = self.gfx.measure(&label, Font::Ui, 12.5, 600);
                    let br = Rect::new(x - 8.0, y + 3.0, lw + 16.0 + 16.0, ST_HEAD_H - 6.0);
                    self.hp(br, 4.0, h);
                    let pressed = self.press_begin(br, h);
                    self.gfx.text(&label, Rect::new(x, y, lw + 2.0, ST_HEAD_H), Font::Ui, 12.5, 600, Align::Left, t.acc_t);
                    self.gfx.glyph(if down { 0xE70D } else { 0xE70E }, Rect::new(x + lw + 6.0, y + 1.0, 9.0, ST_HEAD_H), 9.0, t.acc_t);
                    self.press_end(pressed);
                    self.hit(br, h);
                    continue;
                }
            };
            let e = self.startup[i].clone();
            let r = Rect::new(layer.x + 4.0, y, layer.w - 8.0, 48.0);
            let h = Hit::StRow(i);
            if self.sel != Some(Sel::Startup(i)) {
                self.hp(r, 4.0, h);
            }
            self.hit(r, h);
            let exe = exe_of(&e.command);
            let glyph = if e.source == "Service" { 0xE713 } else if e.source == "Scheduled task" { 0xE916 } else { 0xE8A5 };
            self.draw_icon(&exe, Rect::new(x0, r.cy() - 12.0, 24.0, 24.0), glyph);
            let tx = x0 + 36.0;
            let tw = src_x - tx - 12.0;
            let shown = if e.source == "Scheduled task" { e.name.rsplit('\\').next().unwrap_or(&e.name).to_string() } else { e.name.clone() };
            self.gfx.text(&shown, Rect::new(tx, r.y + 6.0, tw, 18.0), Font::Ui, 13.0, 400, Align::Left, if e.enabled { t.t1 } else { t.t2 });
            let keep = keep_from(&e.command);
            let cmd = self.gfx.elide_mid(&e.command, keep, tw, Font::Ui, 12.0, 400);
            if cmd != e.command || self.gfx.measure(&shown, Font::Ui, 13.0, 400) > tw {
                self.tip(Rect::new(tx, r.y, tw, r.h), format!("{}
{}", e.name, e.command));
            }
            self.gfx.text(&cmd, Rect::new(tx, r.y + 25.0, tw, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t3);
            if show_src {
                self.gfx.text(source_label(e.source), Rect::new(src_x, r.y, 196.0 - 12.0, 48.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
            }
            let (running, now) = self.startup_now(&e);
            if !show_now {
            } else if running {
                let rw = self.gfx.text_at("Running", now_x, r.y, 48.0, 200.0, Font::Ui, 12.5, 400, t.t1);
                self.gfx.text_at(&now["Running".len()..], now_x + rw, r.y, 48.0, 220.0 - 12.0 - rw, Font::Ui, 12.5, 400, t.t2);
            } else {
                self.gfx.text_at(&now, now_x, r.y, 48.0, 208.0, Font::Ui, 12.5, 400, t.t2);
            }
            if show_imp {
                let (level, detail) = self.startup_impact(&e);
                let c = match level {
                    "High" => t.warn,
                    "Not measured" => t.t3,
                    _ => t.t1,
                };
                self.gfx.text_at(level, imp_x, r.y, 48.0, 100.0, Font::Ui, 12.5, if level == "High" { 600 } else { 400 }, c);
                self.tip(Rect::new(imp_x, r.y, 96.0, r.h), detail);
            }
            self.toggle(x1, r.cy(), e.enabled, Hit::StToggle(i), true);
        }
        if self.startup.is_empty() {
            self.gfx.text("Reading startup entries…", Rect::new(view.x, view.y + 30.0, view.w, 24.0), Font::Ui, 13.0, 400, Align::Center, t.t2);
        } else if order.is_empty() {
            self.gfx.text(&format!("No startup apps match \"{}\"", self.query.trim()), Rect::new(view.x, view.y + 30.0, view.w, 24.0), Font::Ui, 13.0, 400, Align::Center, t.t2);
        }
        self.gfx.unclip();
        self.scrollbar(view, 1);
        self.gfx.unclip();
    }

    // ------------------------------------------------------------------ Settings
    pub fn paint_settings(&mut self) {
        let t = self.t;
        let layer = Rect::new(self.cx0(), TITLE_H, self.w - self.cx0() - 12.0, self.h - TITLE_H - 12.0);
        self.gfx.card(layer, 8.0, t.layer, t.layer_s);
        self.gfx.clip(layer.inset(1.0, 1.0));
        let one_col = layer.w < 900.0;
        let pad_x = 24.0;
        let lx = layer.x + pad_x;
        let lw = if one_col { layer.w - pad_x * 2.0 } else { 500.0 };
        let top = layer.y + 20.0 - self.scroll[2];
        let mut y = top;
        self.gfx.text_at("Settings", lx, y, 28.0, 300.0, Font::Display, 20.0, 600, t.t1);
        y += 28.0 + 12.0;
        // default Task Manager
        // the two buttons always try to share one row: full labels under the title, then full labels from the card
        // edge, then short labels; only a very narrow card stacks them (both full width)
        let edge = lx + lw - 16.0;
        let mut pick = None;
        for (set_l, res_l) in [("Set as default Task Manager", "Restore Windows Task Manager"), ("Set as default", "Restore Windows")] {
            let (sw, rw) = (self.btn_w(set_l, 0xEA18, false), self.btn_w(res_l, 0, false));
            for bx in [lx + 48.0, lx + 16.0] {
                if pick.is_none() && bx + sw + 14.0 + rw <= edge {
                    pick = Some((set_l, res_l, sw, rw, bx));
                }
            }
        }
        let one_row = pick.is_some();
        let (set_l, res_l, sw, rw, bx) = pick.unwrap_or(("Set as default", "Restore Windows", lw - 32.0, lw - 32.0, lx + 16.0));
        let card = Rect::new(lx, y, lw, 64.0 + 52.0 + if one_row { 0.0 } else { 42.0 });
        self.gfx.card(card, 8.0, t.card, t.card_s);
        self.gfx.glyph(0xE7EF, Rect::new(lx + 16.0, y, 18.0, 64.0), 16.0, t.t1);
        self.gfx.text("Default Task Manager", Rect::new(lx + 48.0, y + 13.0, lw - 64.0, 18.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
        let now = if self.is_default { "Now: Rust Task Manager" } else { "Now: Windows Task Manager" };
        self.gfx.text(now, Rect::new(lx + 48.0, y + 32.0, lw - 64.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
        self.gfx.hline(lx + 1.0, lx + lw - 1.0, y + 64.0, t.line);
        let by = y + 64.0 + 10.0;
        self.button(Rect::new(bx, by, sw, 32.0), set_l, 0xEA18, Btn::Accent, false, !self.is_default, Hit::SetDefault);
        let (rx0, ry0) = if one_row { (bx + sw + 14.0, by) } else { (bx, by + 42.0) };
        self.button(Rect::new(rx0, ry0, rw, 32.0), res_l, 0, Btn::Std, false, self.is_default, Hit::RestoreDefault);
        y = card.b();
        // Personalize
        y += 18.0;
        self.gfx.text_at("Personalize", lx, y, 20.0, 200.0, Font::Ui, 14.0, 600, t.t1);
        y += 28.0;
        let n = 9 + self.custom_open as usize;
        let card = Rect::new(lx, y, lw, 56.0 * n as f32);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let right = lx + lw - 16.0;
        let mut ry = y;
        self.set_row(lx, lw, ry, false, 0xE790, "Theme", "");
        self.segmented(right, ry + 28.0, &["System", "Light", "Dark"], self.prefs.theme as usize, 0);
        ry += 56.0;
        self.set_row(lx, lw, ry, true, 0xE771, "Background", "");
        self.segmented(right, ry + 28.0, &["Mica", "Mica Alt", "Acrylic", "Solid"], self.prefs.backdrop as usize, 3);
        ry += 56.0;
        // the accent colours the whole window; this says how much
        self.set_row(lx, lw, ry, true, 0xE790, "Colour strength", "");
        self.segmented(right, ry + 28.0, &["Subtle", "Medium", "Strong"], self.prefs.strength.min(2) as usize, 9);
        ry += 56.0;
        if self.backdrop_on() {
            self.set_row(lx, lw, ry, true, 0xE81E, "Transparency", "");
            self.segmented(right, ry + 28.0, &["Off", "Low", "Medium", "High"], self.prefs.transp as usize, 4);
        } else {
            // Solid has no backdrop to show: the control is disabled (drawn faded, no hits)
            self.set_row(lx, lw, ry, true, 0xE81E, "Transparency", "Needs Mica or Acrylic");
            let (n0, old) = (self.hits.len(), self.gfx.push_alpha(0.45));
            self.segmented(right, ry + 28.0, &["Off", "Low", "Medium", "High"], self.prefs.transp as usize, 4);
            self.gfx.pop_alpha(old);
            self.hits.truncate(n0);
        }
        ry += 56.0;
        let acc_name = match self.prefs.accent {
            _ if self.prefs.desktop => "Follows the desktop".to_string(),
            None => "Windows".to_string(),
            Some(rgb) => super::theme::PRESETS.iter().find(|p| p.1 == rgb).map(|p| p.0.to_string()).unwrap_or_else(|| format!("Custom · #{rgb:06X}")),
        };
        self.set_row(lx, lw, ry, true, 0xE8D3, "Accent color", &acc_name);
        self.swatches(right, ry + 28.0);
        ry += 56.0;
        if self.custom_open {
            self.custom_row(lx + 48.0, ry, right, lx, lw);
            ry += 56.0;
        }
        self.set_row(lx, lw, ry, true, 0xE8D2, "Text size", "");
        self.segmented(right, ry + 28.0, &["Small", "Medium", "Large"], self.prefs.font as usize, 6);
        ry += 56.0;
        self.set_row(lx, lw, ry, true, 0xE8A9, "Density", "");
        self.segmented(right, ry + 28.0, &["Compact", "Comfortable"], self.prefs.density as usize, 5);
        ry += 56.0;
        self.set_row(lx, lw, ry, true, 0xE739, "Corners", "");
        self.segmented(right, ry + 28.0, &["Rounded", "Square"], self.prefs.corners as usize, 7);
        ry += 56.0;
        self.set_row(lx, lw, ry, true, 0xE89F, "Details pane width", "");
        self.segmented(right, ry + 28.0, &["Narrow", "Medium", "Wide"], self.prefs.pane as usize, 8);
        y = card.b();
        // Processes
        y += 18.0;
        self.gfx.text_at("Processes", lx, y, 20.0, 200.0, Font::Ui, 14.0, 600, t.t1);
        y += 28.0;
        let card = Rect::new(lx, y, lw, 56.0 * 7.0);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let mut ry = y;
        let speed_sel = [500u64, 1000, 2000, 5000].iter().position(|v| *v == self.prefs.interval_ms).unwrap_or(1);
        self.set_row(lx, lw, ry, false, 0xE895, "Refresh rate", "");
        self.segmented(right, ry + 28.0, &["0.5 s", "1 s", "2 s", "5 s"], speed_sel, 1);
        ry += 56.0;
        self.set_row(lx, lw, ry, true, 0xE8CB, "Default sort", "");
        let label = model::SortKey::parse(&self.prefs.sort).map(|s| s.label()).unwrap_or("Impact");
        let bw = (self.gfx.measure(label, Font::Ui, 13.0, 400) + 24.0 + 9.0 + 8.0).ceil();
        let br = Rect::new(right - bw, ry + 12.0, bw, 32.0);
        self.button(br, "", 0, Btn::Std, false, true, Hit::DefSort);
        self.gfx.text_at(label, br.x + 12.0, br.y, 32.0, bw, Font::Ui, 13.0, 400, t.t1);
        self.gfx.glyph(0xE70D, Rect::new(br.r() - 12.0 - 9.0, br.y + 1.0, 9.0, 32.0), 9.0, t.t2);
        ry += 56.0;
        self.set_row(lx, lw, ry, true, 0xE71D, "Columns", "");
        // Network per app has no counter yet, so it is not offered (ColPill 3 stays reserved for it)
        let cols = [(0u8, "Type", self.prefs.col_type), (1, "PID", self.prefs.show_pid), (2, "I/O", self.prefs.col_disk), (4, "GPU", self.prefs.col_gpu)];
        let ws: Vec<f32> = cols.iter().map(|c| (self.gfx.measure(c.1, Font::Ui, 12.5, 400) + 22.0).ceil()).collect();
        let mut cx = right - ws.iter().sum::<f32>() - 6.0 * (cols.len() as f32 - 1.0);
        for (i, (id, name, on)) in cols.iter().enumerate() {
            let r = Rect::new(cx, ry + 15.0, ws[i], 26.0);
            let h = Hit::ColPill(*id);
            let k = self.fade(hid(1, h), *on);
            let pressed = self.press_begin(r, h);
            self.gfx.fill_round(r, 13.0, t.inp);
            self.gfx.stroke_round(r, 13.0, t.ctl_s.fade(1.0 - k), 1.0);
            self.gfx.fill_round(r, 13.0, t.acc.fade(k));
            self.hp(r, 13.0, h);
            self.gfx.text(name, r, Font::Ui, 12.5, 400, Align::Center, t.t1.mix(t.on_acc, k));
            self.press_end(pressed);
            self.hit(r, h);
            cx += ws[i] + 6.0;
        }
        ry += 56.0;
        let toggles = [(0xE8FDu32, "Group processes", self.prefs.group, 6u8), (0xE8EC, "Show attention chips", self.prefs.chips, 5), (0xE8FD, "Merge processes with the same name", self.prefs.merge, 0), (0xE7BA, "Ask before ending an app that may lose work", self.prefs.ask_end, 1)];
        for (g, label, on, id) in toggles {
            self.set_row(lx, lw, ry, true, g, label, "");
            self.toggle(right, ry + 28.0, on, Hit::Toggle(id), true);
            ry += 56.0;
        }
        y = card.b() + 20.0;
        // right column (below on narrow windows)
        let (rx, rw2, mut ry) = if one_col { (lx, lw, y + 8.0) } else { (lx + lw + 24.0, layer.r() - pad_x - (lx + lw + 24.0), top + 40.0) };
        // AI features: off by default; Google Gemini with the user's own key (stored encrypted, never shown again)
        self.gfx.text_at("AI features", rx, ry, 20.0, 200.0, Font::Ui, 14.0, 600, t.t1);
        ry += 28.0;
        let note = crate::ai::SENT_NOTE;
        let note_w = rw2 - 48.0 - 100.0;
        let note_h = self.gfx.para_h(note, note_w, Font::Ui, 12.0, 400, 16.0);
        let r1 = 13.0 + 18.0 + 2.0 + note_h + 13.0;
        let r2 = 12.0 + 32.0 + 8.0 + 16.0 + 12.0;
        let r3 = 56.0;
        let card = Rect::new(rx, ry, rw2, r1 + r2 + r3);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        // row 1: the switch and what it sends
        self.gfx.sparkle(rx + 16.0, ry + 14.0, 16.0, t.t1);
        self.gfx.text("Explain processes with AI", Rect::new(rx + 48.0, ry + 13.0, note_w, 18.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
        self.gfx.para(note, Rect::new(rx + 48.0, ry + 33.0, note_w, 200.0), Font::Ui, 12.0, 400, Align::Left, t.t2, 16.0);
        if self.key_saved || self.ai_on {
            self.toggle(rx + rw2 - 16.0, ry + 22.0, self.ai_on, Hit::Toggle(3), true);
        } else {
            // no key yet: the switch cannot be turned on
            let tr = Rect::new(rx + rw2 - 16.0 - 40.0, ry + 12.0, 40.0, 20.0);
            self.gfx.stroke_round(tr, 10.0, t.t3.fade(0.6), 1.0);
            self.gfx.ellipse(tr.x + 9.0, tr.cy(), 5.0, t.t3.fade(0.6), true, 0.0);
            self.gfx.text("Off", Rect::new(tr.x - 36.0, tr.y, 24.0, 20.0), Font::Ui, 13.0, 400, Align::Right, t.t3);
        }
        // row 2: the key
        let ky = ry + r1;
        self.gfx.hline(rx + 1.0, rx + rw2 - 1.0, ky, t.line);
        self.gfx.glyph(0xE8D7, Rect::new(rx + 16.0, ky + 12.0, 18.0, 32.0), 16.0, t.t1);
        let tw_ = self.btn_w("Test key", 0, false);
        let sw_ = self.btn_w("Save", 0, false);
        let fx = rx + 48.0;
        let fw = rw2 - 48.0 - 16.0 - sw_ - 8.0 - tw_ - 8.0;
        let fr = Rect::new(fx, ky + 12.0, fw, 32.0);
        let focus = self.key_focus;
        self.gfx.fill_round(fr, 4.0, t.inp.mix(t.ctl_h, if focus { 0.0 } else { self.hov(Hit::KeyField) }));
        self.input_ring(fr, Hit::KeyField, focus);
        let (shown, color) = if !self.key_buf.is_empty() {
            ("•".repeat(self.key_buf.chars().count().min(48)), t.t1)
        } else if self.key_saved {
            ("Saved ••••".to_string(), t.t2)
        } else {
            ("Gemini API key".to_string(), t.t3)
        };
        let tw0 = self.gfx.text_at(&shown, fr.x + 12.0, fr.y, fr.h, fr.w - 24.0, Font::Ui, 13.0, 400, color);
        if focus {
            let cx = if self.key_buf.is_empty() { fr.x + 12.0 } else { fr.x + 12.0 + tw0 + 1.0 };
            self.gfx.fill(Rect::new(cx, fr.y + 8.0, 1.0, 16.0), t.t1);
        }
        self.hit(fr, Hit::KeyField);
        let can_save = !self.key_buf.trim().is_empty();
        self.button(Rect::new(fr.r() + 8.0, fr.y, sw_, 32.0), "Save", 0, Btn::Accent, false, can_save, Hit::KeySave);
        self.button(Rect::new(fr.r() + 8.0 + sw_ + 8.0, fr.y, tw_, 32.0), "Test key", 0, Btn::Std, false, !self.key_testing && (can_save || self.key_saved), Hit::KeyTest);
        // status line + "Get a free key"
        let sy = fr.b() + 8.0;
        let mut sx = fx;
        if self.key_testing {
            self.gfx.text_at("Testing…", sx, sy, 16.0, 200.0, Font::Ui, 12.0, 400, t.t2);
        } else if let Some(r) = &self.key_result {
            let (s, c) = match r {
                Ok(()) => ("Key works".to_string(), t.ok),
                Err(e) => (e.clone(), t.crit),
            };
            let gl = if r.is_ok() { 0xE930 } else { 0xE7BA };
            self.gfx.glyph(gl, Rect::new(sx, sy, 12.0, 16.0), 12.0, c);
            sx += self.gfx.text_at(&s, sx + 18.0, sy, 16.0, fw - 140.0, Font::Ui, 12.0, 400, c) + 18.0 + 16.0;
        } else if self.key_saved {
            sx += self.gfx.text_at("Key saved", sx, sy, 16.0, 200.0, Font::Ui, 12.0, 400, t.t2) + 12.0;
        }
        if self.key_saved && !self.key_testing {
            let w = self.gfx.measure("Remove", Font::Ui, 12.0, 600) + 8.0;
            let rr = Rect::new(sx - 4.0, sy - 3.0, w, 22.0);
            self.gfx.fill_round(rr, 4.0, t.acc_soft.fade(self.hov(Hit::KeyRemove)));
            self.gfx.text("Remove", rr, Font::Ui, 12.0, 600, Align::Center, t.acc_t);
            self.hit(rr, Hit::KeyRemove);
        }
        let lw = self.gfx.measure("Get a free key", Font::Ui, 12.0, 600) + 8.0;
        let lr = Rect::new(fr.r() - lw + 4.0, sy - 3.0, lw, 22.0);
        self.gfx.fill_round(lr, 4.0, t.acc_soft.fade(self.hov(Hit::KeyLink)));
        self.gfx.text("Get a free key", lr, Font::Ui, 12.0, 600, Align::Center, t.acc_t);
        self.hit(lr, Hit::KeyLink);
        // row 3: answer language
        let ly = ky + r2;
        self.gfx.hline(rx + 1.0, rx + rw2 - 1.0, ly, t.line);
        self.gfx.glyph(0xE8C1, Rect::new(rx + 16.0, ly, 18.0, r3), 16.0, t.t1);
        self.gfx.text_at("Answer language", rx + 48.0, ly, r3, 200.0, Font::Ui, 13.0, 400, t.t1);
        let fa = self.lang_is_fa();
        self.segmented(rx + rw2 - 16.0, ly + r3 / 2.0, &["English", "فارسی"], if fa { 1 } else { 0 }, 2);
        ry = card.b() + 20.0;
        self.content_h[2] = ry.max(y) + self.scroll[2] - layer.y;
        let view = Rect::new(layer.x, layer.y, layer.w, layer.h);
        self.view_h[2] = view.h;
        self.clamp_scroll();
        self.gfx.unclip();
        self.scrollbar(view.inset(0.0, 4.0), 2);
    }

    /// One settings row: divider, glyph, label (and a second line in t2).
    fn set_row(&mut self, lx: f32, lw: f32, ry: f32, divider: bool, glyph: u32, label: &str, sub: &str) {
        let t = self.t;
        if divider {
            self.gfx.hline(lx + 1.0, lx + lw - 1.0, ry, t.line);
        }
        self.gfx.glyph(glyph, Rect::new(lx + 16.0, ry, 18.0, 56.0), 16.0, t.t1);
        if sub.is_empty() {
            self.gfx.text_at(label, lx + 48.0, ry, 56.0, lw - 160.0, Font::Ui, 13.0, 400, t.t1);
        } else {
            self.gfx.text_at(label, lx + 48.0, ry + 10.0, 18.0, 160.0, Font::Ui, 13.0, 400, t.t1);
            self.gfx.text_at(sub, lx + 48.0, ry + 29.0, 16.0, 160.0, Font::Ui, 12.0, 400, t.t2);
        }
    }

    /// Accent swatches, right edge at `right`: Windows accent, 8 presets, custom.
    fn swatches(&mut self, right: f32, cy: f32) {
        let t = self.t;
        let dark = t.dark;
        let custom = self.prefs.accent.filter(|a| !super::theme::PRESETS.iter().any(|p| p.1 == *a));
        let desk = self.prefs.desktop;
        // Windows accent, 8 presets, Custom, Follow desktop (the last one is a choice, never the default)
        for i in 0..11u8 {
            let cx = right - 12.0 - (10 - i) as f32 * 30.0;
            let col = match i {
                0 => super::theme::system_accent(dark),
                1..=8 => {
                    let (l, d) = super::theme::accent_pair(super::theme::PRESETS[i as usize - 1].1);
                    if dark { d } else { l }
                }
                9 => custom.map(|c| { let (l, d) = super::theme::accent_pair(c); if dark { d } else { l } }).unwrap_or(t.acc),
                _ => t.acc,
            };
            let sel = match i {
                _ if desk => i == 10,
                0 => self.prefs.accent.is_none(),
                1..=8 => self.prefs.accent == Some(super::theme::PRESETS[i as usize - 1].1),
                9 => custom.is_some() || self.custom_open,
                _ => false,
            };
            let h = Hit::Accent(i);
            let r = Rect::new(cx - 14.0, cy - 14.0, 28.0, 28.0);
            let hv = self.hov(h);
            let k = self.fade(hid(1, h), sel);
            let pressed = self.press_begin(r, h);
            if k > 0.0 {
                self.gfx.ellipse(cx, cy, 13.0, t.t1.fade(k), false, 2.0);
            }
            let rad = 10.0 - 1.5 * k + 1.0 * hv * (1.0 - k);
            if i == 10 {
                // Follow desktop: a clean circle in the desktop colour with a tiny white monitor drawn with shapes
                let dc = self.desktop_rgb().map(|c| {
                    let (l, d) = super::theme::accent_pair(c);
                    if dark { d } else { l }
                });
                self.gfx.ellipse(cx, cy, rad, dc.unwrap_or(t.track), true, 0.0);
                let ink = if dc.is_some() { C(1.0, 1.0, 1.0, 0.92) } else { t.t2 };
                self.gfx.stroke_round(Rect::new(cx - 4.5, cy - 4.0, 9.0, 6.5), 0.0, ink, 1.2);
                self.gfx.fill(Rect::new(cx - 2.5, cy + 3.5, 5.0, 1.2), ink);
                self.tip(r, "Follow desktop (the wallpaper colour)");
            } else if i == 9 && custom.is_none() {
                self.gfx.ellipse(cx, cy, rad, t.track, true, 0.0);
                self.gfx.glyph(0xE710, Rect::new(cx - 10.0, cy - 10.0, 20.0, 20.0), 10.0, t.t2);
            } else {
                self.gfx.ellipse(cx, cy, rad, col, true, 0.0);
                if i == 0 {
                    // the Windows accent: a thin four-pane Windows mark (8 px) in white
                    let ink = C(1.0, 1.0, 1.0, 0.92);
                    for (dx, dy) in [(-4.0f32, -4.0f32), (0.5, -4.0), (-4.0, 0.5), (0.5, 0.5)] {
                        self.gfx.fill(Rect::new(cx + dx, cy + dy, 3.5, 3.5), ink);
                    }
                    self.tip(r, "Windows accent (follows Windows)");
                } else if i == 9 {
                    self.gfx.glyph(0xE70F, Rect::new(cx - 10.0, cy - 10.0, 20.0, 20.0), 9.0, t.on_acc);
                }
            }
            self.press_end(pressed);
            self.hit(r, h);
        }
    }

    /// Settings > Accent > Custom: hue strip and hex field.
    fn custom_row(&mut self, x: f32, ry: f32, right: f32, lx: f32, lw: f32) {
        let t = self.t;
        self.gfx.hline(lx + 1.0, lx + lw - 1.0, ry, t.line);
        let fw = 104.0;
        let hr = Rect::new(x, ry + 21.0, (right - fw - 16.0 - x).max(60.0), 14.0);
        self.hue_rect = hr;
        let n = 36;
        for i in 0..n {
            let seg = Rect::new(hr.x + hr.w * i as f32 / n as f32, hr.y, hr.w / n as f32 + 0.6, hr.h);
            let c = super::theme::from_hsl(i as f32 * 360.0 / n as f32, 0.78, if t.dark { 0.62 } else { 0.45 });
            self.gfx.fill(seg, c);
        }
        self.gfx.stroke_round(hr, 0.0, t.card_s, 1.0);
        let hue = self.prefs.accent.map(super::theme::hue_of).unwrap_or(206.0);
        let kx = hr.x + hr.w * hue / 359.0;
        let (gx, _) = self.glide(0x6300, kx, super::motion::GLIDE);
        self.gfx.fill_round(Rect::new(gx - 3.0, hr.y - 4.0, 6.0, hr.h + 8.0), 3.0, t.card);
        self.gfx.stroke_round(Rect::new(gx - 3.0, hr.y - 4.0, 6.0, hr.h + 8.0), 3.0, t.t1, 1.5);
        self.hit(hr.inset(0.0, -6.0), Hit::Hue);
        // hex field
        let fr = Rect::new(right - fw, ry + 12.0, fw, 32.0);
        let focus = self.hex_focus;
        self.gfx.fill_round(fr, 4.0, t.inp.mix(t.ctl_h, if focus { 0.0 } else { self.hov(Hit::HexField) }));
        self.input_ring(fr, Hit::HexField, focus);
        let shown = if self.hex_buf.is_empty() { self.prefs.accent.map(|a| format!("#{a:06X}")).unwrap_or_default() } else { self.hex_buf.clone() };
        let tw = self.gfx.text_at(&shown, fr.x + 10.0, fr.y, 32.0, fw - 20.0, Font::Mono, 12.5, 400, t.t1);
        if focus {
            self.gfx.fill(Rect::new(fr.x + 10.0 + tw + 1.0, fr.y + 8.0, 1.0, 16.0), t.t1);
        }
        self.hit(fr, Hit::HexField);
    }
}

#[cfg(test)]
mod tests {
    use super::impact_level;

    #[test]
    fn impact_from_measured_use() {
        assert_eq!(impact_level(20 << 20, 0.05), "Low");
        assert_eq!(impact_level(120 << 20, 0.05), "Medium");
        assert_eq!(impact_level(20 << 20, 0.5), "Medium");
        assert_eq!(impact_level(400 << 20, 0.0), "High");
        assert_eq!(impact_level(10 << 20, 3.0), "High");
    }
}
