//! Startup apps page and Settings page.
use super::app::*;
use super::gfx::{Align, Font, Rect};
use super::model;
use super::paint::{Btn, exe_of, keep_from};
use std::cell::RefCell;
use std::collections::HashMap;

/// Settings > Command line: (command, meaning, new)
pub const CLI: [(&str, &str, bool); 12] = [
    ("wtm list --sort impact", "All processes, in this window\u{2019}s order", false),
    ("wtm find <words>", "Search name, path, command line", false),
    ("wtm info <pid|name>", "Path, command line, parent, services", false),
    ("wtm why <pid|name>", "Why it is running", true),
    ("wtm explain <pid|name>", "What is it, is it safe to end", false),
    ("wtm kill <pid|name> [--tree]", "End a process, or its whole tree", false),
    ("wtm restart <pid>", "Restart, same command line", true),
    ("wtm free [--end]", "What can be ended to free memory", true),
    ("wtm startup [enable|disable <name>]", "List or change startup apps", false),
    ("wtm services [words]", "List services", false),
    ("wtm perf", "CPU, memory, disk, network, GPU", false),
    ("wtm default [set|restore]", "Default Task Manager (admin)", true),
];

pub const STARTUP_SORT: [&str; 3] = ["On first", "Name", "Starts from"];

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
    /// "Running · 347 MB", "Running · python.exe · 81 MB" or "Not running".
    pub fn startup_now(&self, e: &crate::startup::Entry) -> (bool, String) {
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
        if refs.is_empty() {
            return (false, "Not running".into());
        }
        let direct = !exe_of(&e.command).to_lowercase().ends_with(".lnk") && refs.len() == 1 && refs[0].ends_with(".exe") && exe_of(&e.command).to_lowercase().ends_with(&refs[0]);
        for r in &refs {
            let hits: Vec<&crate::procs::Proc> = if r.ends_with(".exe") {
                self.procs.iter().filter(|p| p.name.to_lowercase() == *r).collect()
            } else {
                self.procs.iter().filter(|p| self.cmdlines.get(&p.pid).map(|c| c.to_lowercase().contains(r.as_str())).unwrap_or(false)).collect()
            };
            if let Some(p) = hits.first() {
                let mem: u64 = if direct { hits.iter().map(|p| p.mem).sum() } else { p.mem };
                return (true, if direct { format!("Running · {}", model::fmt_mem(mem)) } else { format!("Running · {} · {}", p.name, model::fmt_mem(mem)) });
            }
        }
        (false, "Not running".into())
    }

    pub fn startup_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.startup.len()).collect();
        let s = &self.startup;
        match self.startup_sort {
            1 => v.sort_by_key(|&i| s[i].name.to_lowercase()),
            2 => v.sort_by_key(|&i| (source_label(s[i].source), s[i].name.to_lowercase())),
            _ => v.sort_by_key(|&i| (!s[i].enabled, s[i].source == "Service", s[i].source == "Scheduled task")),
        }
        v
    }

    pub fn paint_startup(&mut self) {
        let t = self.t;
        let layer = Rect::new(12.0, TITLE_H, self.w - 24.0, self.h - TITLE_H - 12.0);
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
        self.gfx.glyph(0xE8CB, Rect::new(br.x + 12.0, br.y, 13.0, 32.0), 13.0, t.t1);
        self.gfx.text_at(&label, br.x + 33.0, br.y, 32.0, bw, Font::Ui, 13.0, 400, t.t1);
        self.gfx.glyph(0xE70D, Rect::new(br.r() - 12.0 - 9.0, br.y + 1.0, 9.0, 32.0), 9.0, t.t2);
        // header
        let hy = layer.y + 60.0;
        self.gfx.hline(layer.x, layer.r(), hy, t.line);
        self.gfx.hline(layer.x, layer.r(), hy + 31.0, t.line);
        let (x0, x1) = (layer.x + 4.0 + 8.0, layer.r() - 4.0 - 12.0);
        let tog_x = x1 - 96.0;
        let now_x = tog_x - 220.0;
        let src_x = now_x - 196.0;
        for (s, x) in [("Name", x0 + 36.0), ("Starts from", src_x), ("Now", now_x)] {
            self.gfx.text_at(s, x, hy, 32.0, 200.0, Font::Ui, 12.0, 400, t.t2);
        }
        self.gfx.text("At startup", Rect::new(tog_x, hy, 96.0, 32.0), Font::Ui, 12.0, 400, Align::Right, t.t2);
        // rows
        let view = Rect::new(layer.x, hy + 32.0, layer.w, layer.b() - hy - 32.0);
        let order = self.startup_order();
        self.content_h[1] = order.len() as f32 * 48.0 + 8.0;
        self.view_h[1] = view.h;
        self.clamp_scroll();
        self.gfx.clip(view);
        for (n, &i) in order.iter().enumerate() {
            let y = view.y + 4.0 + n as f32 * 48.0 - self.scroll[1];
            if y + 48.0 < view.y || y > view.b() {
                continue;
            }
            let e = self.startup[i].clone();
            let r = Rect::new(layer.x + 4.0, y, layer.w - 8.0, 48.0);
            let h = Hit::StRow(i);
            if self.sel == Some(Sel::Startup(i)) {
                self.gfx.fill_round(r, 4.0, t.sel);
            } else {
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
            self.gfx.text(&cmd, Rect::new(tx, r.y + 25.0, tw, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t3);
            self.gfx.text(source_label(e.source), Rect::new(src_x, r.y, 196.0 - 12.0, 48.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
            let (running, now) = self.startup_now(&e);
            if running {
                let rw = self.gfx.text_at("Running", now_x, r.y, 48.0, 200.0, Font::Ui, 12.5, 400, t.t1);
                self.gfx.text_at(&now["Running".len()..], now_x + rw, r.y, 48.0, 220.0 - 12.0 - rw, Font::Ui, 12.5, 400, t.t2);
            } else {
                self.gfx.text_at(&now, now_x, r.y, 48.0, 208.0, Font::Ui, 12.5, 400, t.t2);
            }
            if e.source == "Service" {
                self.gfx.text("Service", Rect::new(tog_x, r.y, 96.0, 48.0), Font::Ui, 12.5, 400, Align::Right, t.t3);
            } else {
                self.toggle(x1, r.cy(), e.enabled, Hit::StToggle(i), true);
            }
        }
        if self.startup.is_empty() {
            self.gfx.text("Reading startup entries…", Rect::new(view.x, view.y + 30.0, view.w, 24.0), Font::Ui, 13.0, 400, Align::Center, t.t2);
        }
        self.gfx.unclip();
        self.scrollbar(view, 1);
        self.gfx.unclip();
    }

    // ------------------------------------------------------------------ Settings
    pub fn paint_settings(&mut self) {
        let t = self.t;
        let layer = Rect::new(12.0, TITLE_H, self.w - 24.0, self.h - TITLE_H - 12.0);
        self.gfx.card(layer, 8.0, t.layer, t.layer_s);
        self.gfx.clip(layer.inset(1.0, 1.0));
        let one_col = self.w < 1000.0;
        let pad_x = 24.0;
        let lx = layer.x + pad_x;
        let lw = if one_col { layer.w - pad_x * 2.0 } else { 500.0 };
        let top = layer.y + 20.0 - self.scroll[2];
        let mut y = top;
        self.gfx.text_at("Settings", lx, y, 28.0, 300.0, Font::Display, 20.0, 600, t.t1);
        y += 28.0 + 12.0;
        // default Task Manager
        let card = Rect::new(lx, y, lw, 64.0 + 52.0);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        self.gfx.glyph(0xE7EF, Rect::new(lx + 16.0, y, 18.0, 64.0), 16.0, t.t1);
        self.gfx.text("Default Task Manager", Rect::new(lx + 48.0, y + 13.0, lw - 64.0, 18.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
        let now = if self.is_default { "Now: Win Tools Task Manager" } else { "Now: Windows Task Manager" };
        self.gfx.text(now, Rect::new(lx + 48.0, y + 32.0, lw - 64.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
        self.gfx.hline(lx + 1.0, lx + lw - 1.0, y + 64.0, t.line);
        let by = y + 64.0 + 10.0;
        let sw = self.btn_w("Set as default Task Manager", 0xEA18, false);
        self.button(Rect::new(lx + 48.0, by, sw, 32.0), "Set as default Task Manager", 0xEA18, Btn::Accent, false, !self.is_default, Hit::SetDefault);
        let rw = self.btn_w("Restore Windows Task Manager", 0, false);
        self.button(Rect::new(lx + 48.0 + sw + 14.0, by, rw, 32.0), "Restore Windows Task Manager", 0, Btn::Std, false, self.is_default, Hit::RestoreDefault);
        y = card.b();
        // Look
        y += 18.0;
        self.gfx.text_at("Look", lx, y, 20.0, 200.0, Font::Ui, 14.0, 600, t.t1);
        y += 20.0 + 8.0;
        let card = Rect::new(lx, y, lw, 56.0 * 2.0);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let theme_sel = self.prefs.theme as usize;
        let speed_sel = [500u64, 1000, 2000, 5000].iter().position(|v| *v == self.prefs.interval_ms).unwrap_or(1);
        for (i, (g, label)) in [(0xE790u32, "Theme"), (0xE895, "Update speed")].iter().enumerate() {
            let ry = y + i as f32 * 56.0;
            if i > 0 {
                self.gfx.hline(lx + 1.0, lx + lw - 1.0, ry, t.line);
            }
            self.gfx.glyph(*g, Rect::new(lx + 16.0, ry, 18.0, 56.0), 16.0, t.t1);
            self.gfx.text_at(label, lx + 48.0, ry, 56.0, 200.0, Font::Ui, 13.0, 400, t.t1);
            if i == 0 {
                self.segmented(lx + lw - 16.0, ry + 28.0, &["System", "Light", "Dark"], theme_sel, 0);
            } else {
                self.segmented(lx + lw - 16.0, ry + 28.0, &["0.5 s", "1 s", "2 s", "5 s"], speed_sel, 1);
            }
        }
        y = card.b();
        // Processes
        y += 18.0;
        self.gfx.text_at("Processes", lx, y, 20.0, 200.0, Font::Ui, 14.0, 600, t.t1);
        y += 28.0;
        let rows = [(0xE8FDu32, "Merge processes with the same name", self.prefs.merge), (0xE7BA, "Ask before ending an app that may lose work", self.prefs.ask_end), (0xE8EC, "Show PID column", self.prefs.show_pid)];
        let card = Rect::new(lx, y, lw, 56.0 * rows.len() as f32);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        for (i, (g, label, on)) in rows.iter().enumerate() {
            let ry = y + i as f32 * 56.0;
            if i > 0 {
                self.gfx.hline(lx + 1.0, lx + lw - 1.0, ry, t.line);
            }
            self.gfx.glyph(*g, Rect::new(lx + 16.0, ry, 18.0, 56.0), 16.0, t.t1);
            self.gfx.text_at(label, lx + 48.0, ry, 56.0, lw - 160.0, Font::Ui, 13.0, 400, t.t1);
            self.toggle(lx + lw - 16.0, ry + 28.0, *on, Hit::Toggle(i as u8), true);
        }
        y = card.b() + 20.0;
        // right column (below on narrow windows)
        let (rx, rw2, mut ry) = if one_col { (lx, lw, y + 8.0) } else { (lx + lw + 24.0, layer.r() - pad_x - (lx + lw + 24.0), top) };
        let hw = self.gfx.text_at("Command line", rx, ry, 28.0, 300.0, Font::Display, 20.0, 600, t.t1);
        self.gfx.text_at("wtm.exe", rx + hw + 10.0, ry + 2.0, 28.0, 100.0, Font::Ui, 13.0, 400, t.t2);
        ry += 28.0 + 12.0;
        let card = Rect::new(rx, ry, rw2, 34.0 * CLI.len() as f32);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let cmd_w = if rw2 < 520.0 { 230.0 } else { 276.0 };
        for (i, (cmd, desc, new)) in CLI.iter().enumerate() {
            let r = Rect::new(rx + 1.0, ry + i as f32 * 34.0, rw2 - 2.0, 34.0);
            if i > 0 {
                self.gfx.hline(r.x, r.r(), r.y, t.line);
            }
            let h = Hit::CliCopy(i as u8);
            let hov = self.hover == h;
            if hov {
                self.gfx.fill(Rect::new(r.x, r.y + if i > 0 { 1.0 } else { 0.0 }, r.w, r.h - if i > 0 { 1.0 } else { 0.0 }), t.hov);
            }
            self.hit(r, h);
            let cw = self.gfx.text_at(cmd, r.x + 15.0, r.y, 34.0, cmd_w, Font::Mono, 12.0, 400, t.t1);
            if *new {
                self.gfx.text_at("new", r.x + 15.0 + cw + 6.0, r.y, 34.0, 40.0, Font::Ui, 10.5, 600, t.acc_t);
            }
            self.gfx.text(desc, Rect::new(r.x + 15.0 + cmd_w + 12.0, r.y, r.w - 15.0 - cmd_w - 12.0 - 44.0, 34.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
            self.gfx.glyph(0xE8C8, Rect::new(r.r() - 12.0 - 20.0, r.y, 20.0, 34.0), 12.0, if hov { t.t1 } else { t.t3 });
        }
        ry = card.b() + 10.0;
        self.gfx.glyph(0xE756, Rect::new(rx, ry, 14.0, 20.0), 12.0, t.t2);
        let w1 = self.gfx.text_at("Add ", rx + 22.0, ry, 20.0, 60.0, Font::Ui, 12.0, 400, t.t2);
        let w2 = self.gfx.text_at("--json", rx + 22.0 + w1, ry, 20.0, 80.0, Font::Mono, 12.0, 400, t.t1);
        self.gfx.text_at(" to any command for agents", rx + 22.0 + w1 + w2, ry, 20.0, 300.0, Font::Ui, 12.0, 400, t.t2);
        ry += 20.0 + 18.0;
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
        self.gfx.fill_round(fr, 4.0, if self.hover == Hit::KeyField && !focus { t.ctl_h } else { t.inp });
        self.gfx.stroke_round(fr, 4.0, t.inp_s, 1.0);
        if focus {
            self.gfx.fill_round(Rect::new(fr.x + 1.0, fr.b() - 2.0, fr.w - 2.0, 2.0), 1.0, t.acc);
        } else {
            self.gfx.fill(Rect::new(fr.x + 3.0, fr.b() - 1.0, fr.w - 6.0, 1.0), t.inp_b);
        }
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
            if self.hover == Hit::KeyRemove {
                self.gfx.fill_round(rr, 4.0, t.acc_soft);
            }
            self.gfx.text("Remove", rr, Font::Ui, 12.0, 600, Align::Center, t.acc_t);
            self.hit(rr, Hit::KeyRemove);
        }
        let lw = self.gfx.measure("Get a free key", Font::Ui, 12.0, 600) + 8.0;
        let lr = Rect::new(fr.r() - lw + 4.0, sy - 3.0, lw, 22.0);
        if self.hover == Hit::KeyLink {
            self.gfx.fill_round(lr, 4.0, t.acc_soft);
        }
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
}
