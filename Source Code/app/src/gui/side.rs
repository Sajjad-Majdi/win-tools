//! The right pane of the Processes page: details ("What is this?", "Why is this running?", Details) and
//! "Free up memory".
use super::app::*;
use super::gfx::{Align, Font, Rect};
use super::model;
use super::motion;
use super::paint::Btn;
use crate::explain::Verdict;

const NUM: [&str; 11] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];

impl App {
    /// The right pane. Opening: it springs in from the right and fades in; `closed`: it slides out and fades.
    pub fn paint_pane(&mut self, pr: Rect, closed: Option<std::time::Instant>) {
        let t = self.t;
        self.pane_rect = pr;
        self.view_h[3] = pr.h;
        let Some(at) = self.pane.as_ref().map(|p| p.at) else { return };
        let (a, dx) = match closed {
            None => (self.tween_in(at, motion::NORMAL, motion::decel), (1.0 - self.spring_in(at, motion::PANE)) * 32.0),
            Some(c) => match self.leaving(c) {
                Some(q) => (1.0 - q, q * 40.0),
                None => {
                    self.mo.borrow_mut().ghost_pane = None;
                    return;
                }
            },
        };
        let old = self.gfx.push_alpha(a);
        self.gfx.fill(Rect::new(pr.x, pr.y, pr.w, pr.h), t.layer);
        self.gfx.vline(pr.x, pr.y, pr.b(), t.line);
        self.hit(pr, Hit::Block);
        self.gfx.clip(Rect::new(pr.x + 1.0, pr.y, pr.w - 1.0, pr.h));
        self.gfx.push_move(dx, 0.0);
        let kind = self.pane.as_ref().map(|p| p.kind).unwrap();
        let h = match kind {
            PaneKind::Details => self.paint_details(pr),
            PaneKind::Free => self.paint_free(pr),
        };
        self.gfx.pop_xf();
        self.gfx.unclip();
        self.content_h[3] = h;
        self.clamp_scroll();
        self.scrollbar(pr, 3);
        self.gfx.pop_alpha(old);
    }

    /// Close button at the pane's top right.
    fn pane_close(&mut self, pr: Rect, y: f32) {
        let t = self.t;
        let cr = Rect::new(pr.r() - 16.0 - 32.0 + 6.0, y - 2.0, 32.0, 32.0);
        self.hp(cr, 4.0, Hit::PaneClose);
        self.gfx.glyph(0xE711, cr, 11.0, t.t2);
        self.hit(cr, Hit::PaneClose);
    }

    /// Section card header (44 high). Returns the y below it.
    fn sec_head(&mut self, x: f32, y: f32, w: f32, idx: u8, title: &str, glyph: u32, sparkle: bool, verdict: Option<(String, u32, super::gfx::C, super::gfx::C)>) -> f32 {
        let t = self.t;
        let open = self.pane.as_ref().map(|p| p.open[idx as usize]).unwrap_or(false);
        let hr = Rect::new(x, y, w, 44.0);
        let h = Hit::PaneSec(idx);
        self.hp(hr.inset(1.0, 1.0), 7.0, h);
        self.hit(hr, h);
        if sparkle {
            self.gfx.sparkle(x + 14.0, y + 14.5, 15.0, t.acc);
        } else {
            self.gfx.glyph(glyph, Rect::new(x + 14.0, y, 15.0, 44.0), 14.0, t.t1);
        }
        self.gfx.text_at(title, x + 14.0 + 15.0 + 10.0, y, 44.0, w - 140.0, Font::Ui, 14.0, 600, t.t1);
        let chev = Rect::new(x + w - 12.0 - 10.0, y, 10.0, 44.0);
        self.gfx.glyph(if open { 0xE70E } else { 0xE70D }, chev, 10.0, t.t2);
        if let Some((label, gl, fg, bg)) = verdict {
            let pw = self.gfx.measure(&label, Font::Ui, 12.0, 600) + 20.0 + 16.0;
            self.pill(chev.x - 10.0 - pw, y + 22.0, &label, gl, fg, bg, 24.0, 12.0, 600);
        }
        y + 44.0
    }

    fn verdict_pill(&self, v: Verdict) -> (String, u32, super::gfx::C, super::gfx::C) {
        let t = self.t;
        match v {
            Verdict::Safe => ("Safe to end".into(), 0xE73E, t.ok, t.ok_bg),
            Verdict::Work => ("May lose work".into(), 0xE7BA, t.warn, t.warn_bg),
            Verdict::Critical => ("Windows needs it".into(), 0xE72E, t.t2, t.tag),
            Verdict::Suspicious => ("Suspicious".into(), 0xE7BA, t.crit, t.crit_bg),
            Verdict::Unknown => ("Unknown".into(), 0xE9CE, t.t2, t.tag),
        }
    }

    /// The explanation paragraphs: what it is, who made it, what happens if it is ended.
    fn explain_paras(&self, fa: bool) -> Vec<String> {
        let Some(p) = &self.pane else { return vec![] };
        let Some(a) = &p.info.answer else { return vec![] };
        let signed = p.info.signed;
        // who made it is a local fact (built-in list or the file's version info), never AI text
        let maker = p.info.local.as_ref().map(|l| l.maker.clone()).unwrap_or_default();
        let mut v = vec![];
        if fa && !a.what_fa.is_empty() {
            v.push(a.what_fa.clone());
            let mut who = if maker.is_empty() { String::new() } else { format!("سازنده‌ی آن {maker} است.") };
            match signed {
                Some(true) => who += " امضای دیجیتال معتبر دارد.",
                Some(false) => who += " امضای دیجیتال ندارد.",
                None => {}
            }
            if !who.trim().is_empty() {
                v.push(who.trim().to_string());
            }
            v.push(
                match a.verdict {
                    Verdict::Safe => "پایان دادن به آن بی‌خطر است و هر وقت لازم باشد دوباره اجرا می‌شود.",
                    Verdict::Work => "با پایان دادن، برنامه بسته می‌شود و کار ذخیره‌نشده از دست می‌رود.",
                    Verdict::Critical => "ویندوز به آن نیاز دارد. پایان دادن به آن ممکن است سیستم را از کار بیندازد یا ری‌استارت کند.",
                    Verdict::Suspicious => "نشانه‌های مشکوک دارد. آن را با آنتی‌ویروس بررسی کنید.",
                    Verdict::Unknown => "اطلاعات کافی درباره‌ی آن نیست. فقط اگر آن را می‌شناسید پایانش دهید.",
                }
                .to_string(),
            );
        } else {
            v.push(a.what_en.clone());
            let mut who = if maker.is_empty() { String::new() } else { format!("Made by {maker}.") };
            match signed {
                Some(true) => who += " It has a valid digital signature.",
                Some(false) => who += " It has no digital signature.",
                None => {}
            }
            if !who.trim().is_empty() {
                v.push(who.trim().to_string());
            }
            v.push(
                match a.verdict {
                    Verdict::Safe => "Ending it is safe; it starts again when needed.",
                    Verdict::Work if !a.ends_en.is_empty() => a.ends_en.as_str(),
                    Verdict::Work => "Ending it closes the app; work you have not saved is lost.",
                    Verdict::Critical => "Windows needs it. Ending it can freeze or restart the PC.",
                    Verdict::Suspicious => "It shows warning signs. Check it with your antivirus.",
                    Verdict::Unknown => "Little is known about it. End it only if you know what it is.",
                }
                .to_string(),
            );
        }
        v
    }

    fn paint_details(&mut self, pr: Rect) -> f32 {
        let t = self.t;
        let Some(pane) = self.pane.as_ref() else { return 0.0 };
        let (key, pid, open, loading, ai_loading) = (pane.key.clone(), pane.pid, pane.open, pane.loading, pane.ai_loading);
        let info = pane.info.clone();
        let Some(g) = self.group(&key).cloned() else { return 0.0 };
        let x = pr.x + 17.0;
        let w = pr.w - 33.0;
        let y0 = pr.y + 16.0 - self.scroll[3];
        let mut y = y0;
        // header
        self.draw_icon(&g.path.clone(), Rect::new(x, y, 32.0, 32.0), 0xE737);
        let title = if g.members.len() > 1 && g.root() != pid { g.members.iter().find(|m| m.pid == pid).map(|m| m.name.clone()).unwrap_or(g.label.clone()) } else { g.label.clone() };
        self.gfx.text(&title, Rect::new(x + 44.0, y - 2.0, w - 44.0 - 36.0, 28.0), Font::Display, 20.0, 600, Align::Left, t.t1);
        let sub = if g.members.len() > 1 { format!("{} processes · {} · {}", g.members.len(), g.type_label(), model::fmt_mem(g.mem)) } else { format!("{} · {} · {}", g.exe, g.type_label(), model::fmt_mem(g.mem)) };
        self.gfx.text(&sub, Rect::new(x + 44.0, y + 26.0, w - 44.0, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
        self.pane_close(pr, y);
        y += 44.0 + 12.0;
        // actions
        let ew = self.btn_w("End task", 0xE711, false);
        self.button(Rect::new(x, y, ew, 32.0), "End task", 0xE711, Btn::Std, false, true, Hit::PaneEnd);
        let rw = self.btn_w("Restart", 0xE72C, false);
        let can_restart = g.kind != crate::procs::Kind::Windows;
        self.button(Rect::new(x + ew + 8.0, y, rw, 32.0), "Restart", 0xE72C, Btn::Std, false, can_restart, Hit::PaneRestart);
        self.button(Rect::new(x + ew + rw + 16.0, y, 34.0, 32.0), "", 0xE712, Btn::Std, false, true, Hit::PaneMore);
        y += 32.0 + 12.0;

        // ---- What is this?
        let fa = self.lang_is_fa() && info.answer.as_ref().map(|a| !a.what_fa.is_empty()).unwrap_or(false);
        let paras = self.explain_paras(fa);
        let facts = self.facts(&g, pid, &info);
        let is_ai = info.answer.as_ref().map(|a| a.source == "ai").unwrap_or(false);
        let body_h = if open[0] {
            let mut hh = if is_ai { 24.0 } else { 0.0 };
            for p in &paras {
                hh += self.gfx.para_h(p, w - 28.0, if fa { Font::Fa } else { Font::Ui }, 14.0, 400, if fa { 26.0 } else { 22.0 }) + 6.0;
            }
            if paras.is_empty() {
                hh += 3.0 * 26.0;
            }
            hh - 6.0 + 12.0 + 1.0 + 12.0 + facts.len() as f32 * 26.0 - 9.0 + 14.0
        } else {
            0.0
        };
        let card = Rect::new(x, y, w, 44.0 + body_h);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let verdict = info.answer.as_ref().map(|a| self.verdict_pill(a.verdict));
        let use_sparkle = self.ai_on;
        let mut by = self.sec_head(x, y, w, 0, "What is this?", 0xE9CE, use_sparkle, verdict);
        if open[0] {
            let bx = x + 14.0;
            let bw = w - 28.0;
            if is_ai && !loading {
                // AI text is always labelled as such; the facts below stay local
                self.gfx.sparkle(bx, by + 4.0, 12.0, t.t3);
                self.gfx.text_at("AI explanation", bx + 18.0, by, 20.0, bw - 18.0, Font::Ui, 12.0, 400, t.t3);
                by += 24.0;
            }
            if paras.is_empty() || loading {
                for (i, f) in [0.92f32, 0.8, 0.6].iter().enumerate() {
                    self.gfx.fill_round(Rect::new(if fa { bx + bw * (1.0 - f) } else { bx }, by + 8.0 + i as f32 * 26.0, bw * f, 10.0), 5.0, t.line);
                }
                by += 3.0 * 26.0;
            } else {
                for p in &paras {
                    let hh = if fa { self.gfx.para(p, Rect::new(bx, by, bw, 400.0), Font::Fa, 14.0, 400, Align::Right, t.t1, 26.0) } else { self.gfx.para(p, Rect::new(bx, by, bw, 400.0), Font::Ui, 14.0, 400, Align::Left, t.t1, 22.0) };
                    by += hh + 6.0;
                }
                by -= 6.0;
            }
            if ai_loading && self.ai_on {
                self.gfx.sparkle(bx + bw - 14.0, by - 16.0, 12.0, t.acc.fade(0.6));
            }
            by += 12.0;
            self.gfx.hline(bx, bx + bw, by, t.line);
            by += 1.0 + 12.0;
            for (k, v) in &facts {
                self.gfx.text(k, Rect::new(bx, by, 92.0, 17.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
                let vs = if k == "Location" { self.gfx.elide_start(v, bw - 92.0, Font::Ui, 12.5, 400) } else { v.clone() };
                self.gfx.text(&vs, Rect::new(bx + 92.0, by, bw - 92.0, 17.0), Font::Ui, 12.5, 400, Align::Left, t.t1);
                by += 26.0;
            }
        }
        y = card.b() + 12.0;

        // ---- Why is this running?
        let items = self.why_items(&g, pid, &info);
        let why_h = if open[1] { items.iter().map(|it| self.why_item_h(it, w - 28.0)).sum::<f32>() + 10.0 } else { 0.0 };
        let card = Rect::new(x, y, w, 44.0 + why_h);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let mut by = self.sec_head(x, y, w, 1, "Why is this running?", 0xE8FD, false, None);
        if open[1] {
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    self.gfx.hline(x + 14.0, x + w - 14.0, by, t.line);
                }
                let hh = self.why_item_h(it, w - 28.0);
                self.paint_why_item(it, x + 14.0, by + if i == 0 { 2.0 } else { 10.0 }, w - 28.0);
                by += hh;
            }
        }
        y = card.b() + 12.0;

        // ---- Details
        let det: Vec<(String, String)> = {
            let mut v = vec![("PID".to_string(), if g.members.len() > 1 { g.pids().iter().take(8).map(|p| p.to_string()).collect::<Vec<_>>().join(", ") } else { pid.to_string() })];
            let ct: u64 = g.cpu_time / 10_000_000;
            v.push(("CPU time".into(), model::fmt_dur(ct).replace(" min", " min").to_string()));
            v.push(("Memory".into(), model::fmt_mem(g.mem)));
            v
        };
        let cmd = info.cmdline.clone();
        let cmd_h = if cmd.is_empty() { 0.0 } else { self.gfx.para_h(&cmd, w - 28.0 - 16.0, Font::Mono, 11.5, 400, 16.0) + 10.0 + 8.0 };
        let det_h = if open[2] { det.len() as f32 * 26.0 + cmd_h + 10.0 } else { 0.0 };
        let card = Rect::new(x, y, w, 44.0 + det_h);
        self.gfx.card(card, 8.0, t.card, t.card_s);
        let mut by = self.sec_head(x, y, w, 2, "Details", 0xE946, false, None);
        if open[2] {
            for (k, v) in &det {
                self.gfx.text(k, Rect::new(x + 14.0, by, 92.0, 17.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
                self.gfx.text(v, Rect::new(x + 14.0 + 92.0, by, w - 28.0 - 92.0, 17.0), Font::Ui, 12.5, 400, Align::Left, t.t1);
                by += 26.0;
            }
            if !cmd.is_empty() {
                let cr = Rect::new(x + 14.0, by, w - 28.0, cmd_h - 8.0);
                self.gfx.fill_round(cr, 4.0, t.code);
                self.gfx.para(&cmd, Rect::new(cr.x + 8.0, cr.y + 5.0, cr.w - 16.0, 400.0), Font::Mono, 11.5, 400, Align::Left, t.t1, 16.0);
            }
        }
        y = card.b() + 16.0;
        y - y0
    }

    fn facts(&self, g: &model::Group, pid: u32, info: &PaneInfo) -> Vec<(String, String)> {
        let p = self.proc(pid);
        let publisher = match (info.signed, g.company.is_empty()) {
            (Some(false), _) => "Not signed".to_string(),
            (_, false) => g.company.clone(),
            _ => "Unknown".into(),
        };
        let mut v = vec![("Publisher".to_string(), publisher)];
        if let Some((_, exe, _)) = &info.parent {
            v.push(("Started by".into(), exe.clone()));
        }
        let path = p.map(|p| p.meta.path.clone()).filter(|s| !s.is_empty()).unwrap_or(g.path.clone());
        v.push(("Location".into(), if path.is_empty() { "No access (protected)".into() } else { path }));
        if let Some(p) = p {
            let secs = ((App::now_ft() - p.create_time) / 10_000_000).max(0) as u64;
            v.push(("Running for".into(), model::fmt_dur(secs)));
        }
        v
    }

    fn why_items(&self, g: &model::Group, pid: u32, info: &PaneInfo) -> Vec<WhyItem> {
        let mut v = vec![];
        for (i, s) in info.startup.iter().enumerate() {
            v.push(WhyItem { glyph: 0xE7E8, title: if s.enabled { "Runs at startup".into() } else { "Startup entry (off)".into() }, value: format!("{} · {}", s.name, s.source), code: s.command.clone(), button: Some((if s.enabled { "Disable at startup" } else { "Enable at startup" }.into(), Hit::PaneDisable(i))), muted: false, crit: None });
        }
        if let Some((label, exe, ct)) = &info.parent {
            let secs = ((App::now_ft() - ct) / 10_000_000).max(0) as u64;
            let child_ct = self.proc(pid).map(|p| p.create_time).unwrap_or(0);
            let at_signin = exe.eq_ignore_ascii_case("explorer.exe") && (child_ct - ct) < 3 * 60 * 10_000_000;
            let when = if at_signin { format!("at sign-in, {} ago", model::fmt_dur(secs)) } else { format!("{} ago", model::fmt_dur(((App::now_ft() - child_ct) / 10_000_000).max(0) as u64)) };
            v.push(WhyItem { glyph: 0xE7C4, title: format!("Started by {label}"), value: format!("{exe} · {when}"), code: String::new(), button: None, muted: false, crit: None });
        }
        for s in &info.services {
            v.push(WhyItem { glyph: 0xE90F, title: "Runs as a service".into(), value: s.clone(), code: String::new(), button: None, muted: false, crit: None });
        }
        for s in &info.tasks {
            v.push(WhyItem { glyph: 0xE916, title: "Scheduled task".into(), value: s.clone(), code: String::new(), button: None, muted: false, crit: None });
        }
        if info.services.is_empty() && info.tasks.is_empty() {
            v.push(WhyItem { glyph: 0xE90F, title: "No service and no scheduled task".into(), value: String::new(), code: String::new(), button: None, muted: true, crit: None });
        }
        let path = self.proc(pid).map(|p| p.meta.path.to_lowercase()).unwrap_or_default();
        let place = if path.is_empty() {
            "Protected Windows process".to_string()
        } else if path.starts_with(r"c:\windows\") {
            "Runs from Windows".into()
        } else if path.contains(r"\program files") {
            "Runs from Program Files".into()
        } else if path.contains(r"\downloads\") {
            "Runs from Downloads".into()
        } else if path.contains(r"\temp\") {
            "Runs from a Temp folder".into()
        } else if path.contains(r"\appdata\") {
            "Runs from AppData".into()
        } else {
            format!("Runs from {}", path.rsplit_once('\\').map(|x| x.0).unwrap_or(""))
        };
        let title = match info.signed {
            Some(true) if !g.company.is_empty() => format!("Signed by {}", g.company),
            Some(true) => "Signed".into(),
            Some(false) => "Not signed".into(),
            None => "Signature not checked".into(),
        };
        let sus = info.answer.as_ref().map(|a| a.verdict == Verdict::Suspicious).unwrap_or(false);
        let crit = if sus { Some(info.flags.join(" · ")) } else { None };
        v.push(WhyItem { glyph: 0xE83D, title, value: place, code: String::new(), button: None, muted: false, crit });
        v
    }

    fn why_item_h(&self, it: &WhyItem, w: f32) -> f32 {
        let tw = w - 28.0;
        let mut h = 10.0 + 18.0;
        if !it.value.is_empty() {
            h += 2.0 + 18.0;
        }
        if !it.code.is_empty() {
            h += 6.0 + self.gfx.para_h(&break_paths(&it.code), tw - 16.0, Font::Mono, 11.5, 400, 16.0) + 10.0;
        }
        if it.button.is_some() {
            h += 8.0 + 26.0;
        }
        if it.crit.is_some() {
            h += 8.0 + 20.0;
        }
        h + 10.0
    }

    fn paint_why_item(&mut self, it: &WhyItem, x: f32, y: f32, w: f32) {
        let t = self.t;
        let fg = if it.muted { t.t3 } else { t.t2 };
        self.gfx.glyph(it.glyph, Rect::new(x, y + 1.0, 16.0, 18.0), 16.0, fg);
        let tx = x + 28.0;
        let tw = w - 28.0;
        let mut y = y;
        self.gfx.text(&it.title, Rect::new(tx, y, tw, 18.0), Font::Ui, 13.0, if it.muted { 400 } else { 600 }, Align::Left, if it.muted { t.t3 } else { t.t1 });
        y += 18.0;
        if !it.value.is_empty() {
            self.gfx.text(&it.value, Rect::new(tx, y + 2.0, tw, 18.0), Font::Ui, 12.5, 400, Align::Left, t.t2);
            y += 20.0;
        }
        if !it.code.is_empty() {
            let code = break_paths(&it.code);
            let ch = self.gfx.para_h(&code, tw - 16.0, Font::Mono, 11.5, 400, 16.0) + 10.0;
            let cr = Rect::new(tx, y + 6.0, tw, ch);
            self.gfx.fill_round(cr, 4.0, t.code);
            self.gfx.para(&code, Rect::new(cr.x + 8.0, cr.y + 5.0, cr.w - 16.0, 400.0), Font::Mono, 11.5, 400, Align::Left, t.t1, 16.0);
            y += 6.0 + ch;
        }
        if let Some((label, h)) = &it.button {
            let bw = self.btn_w(label, 0, true);
            self.button(Rect::new(tx, y + 8.0, bw, 26.0), label, 0, Btn::Std, true, true, *h);
            y += 34.0;
        }
        if let Some(c) = &it.crit {
            let s = if c.is_empty() { "Suspicious".to_string() } else { c.clone() };
            self.pill(tx, y + 8.0 + 10.0, &s, 0xE7BA, t.crit, t.crit_bg, 20.0, 11.5, 400);
        }
    }

    // ------------------------------------------------------------------ Free up memory
    fn paint_free(&mut self, pr: Rect) -> f32 {
        let t = self.t;
        let (go, keep) = self.free_lists();
        let keep_open = self.pane.as_ref().map(|p| p.keep_open).unwrap_or(false);
        let x = pr.x + 17.0;
        let w = pr.w - 33.0;
        let y0 = pr.y + 16.0 - self.scroll[3];
        let mut y = y0;
        let s = self.perf.clone();
        let used = s.mem_total.saturating_sub(s.mem_avail);
        let can: u64 = go.iter().map(|i| i.mem).sum();
        self.gfx.text("Free up memory", Rect::new(x, y - 2.0, w - 40.0, 28.0), Font::Display, 20.0, 600, Align::Left, t.t1);
        let sub = format!("{} of {} in use · {} can go", model::fmt_gb(used), model::fmt_gb(s.mem_total), model::fmt_mem(can));
        self.gfx.text(&sub, Rect::new(x, y + 28.0, w, 16.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
        self.pane_close(pr, y);
        y += 46.0 + 12.0;
        // memory bar: in use, then the part that can go
        let bar = Rect::new(x, y, w, 8.0);
        self.gfx.fill_round(bar, 4.0, t.track);
        let total = s.mem_total.max(1) as f32;
        let uw = (w * used as f32 / total).min(w);
        let cw = (w * can as f32 / total).min(w - uw);
        self.gfx.fill_round(Rect::new(bar.x, bar.y, (uw - cw).max(4.0), 8.0), 4.0, t.mem);
        if cw > 1.0 {
            self.gfx.fill_round(Rect::new(bar.x + uw - cw + 2.0, bar.y, cw, 8.0), 2.0, t.mem.fade(0.4));
        }
        y += 8.0 + 14.0;
        // candidates
        let row_h = 71.0;
        if go.is_empty() {
            let r = Rect::new(x, y, w, 64.0);
            self.gfx.card(r, 8.0, t.card, t.card_s);
            self.gfx.text("Nothing can go without stopping something you use.", r.inset(14.0, 0.0), Font::Ui, 13.0, 400, Align::Left, t.t2);
            y = r.b() + 14.0;
        } else {
            let card = Rect::new(x, y, w, go.len() as f32 * row_h);
            self.gfx.card(card, 8.0, t.card, t.card_s);
            for (i, it) in go.iter().enumerate() {
                let ry = y + i as f32 * row_h;
                if i > 0 {
                    self.gfx.hline(x + 1.0, x + w - 1.0, ry, t.line);
                }
                let path = it.pids.first().and_then(|p| self.proc(*p)).map(|p| p.meta.path.clone()).unwrap_or_default();
                self.draw_icon(&path, Rect::new(x + 12.0, ry + 9.0, 20.0, 20.0), 0xE737);
                let tx = x + 12.0 + 20.0 + 12.0;
                let tw = w - (tx - x) - 12.0;
                let size = model::fmt_mem(it.mem);
                let sw = self.gfx.measure(&size, Font::Ui, 13.0, 400);
                let label = if it.pids.len() > 1 { format!("{} ({})", it.label, it.pids.len()) } else { it.label.clone() };
                self.gfx.text(&label, Rect::new(tx, ry + 9.0, tw - sw - 8.0, 20.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
                self.gfx.text(&size, Rect::new(tx + tw - sw - 1.0, ry + 9.0, sw + 1.0, 20.0), Font::Ui, 13.0, 400, Align::Right, t.t1);
                let by = ry + 9.0 + 20.0 + 6.0;
                let mut bx = tx + tw;
                if it.startup_entry.is_some() {
                    let bw = self.btn_w("End + disable at startup", 0, true);
                    bx -= bw;
                    self.button(Rect::new(bx, by, bw, 26.0), "End + disable at startup", 0, Btn::Std, true, true, Hit::FreeEndDisable(i));
                    bx -= 6.0;
                }
                let bw = self.btn_w("End", 0, true);
                bx -= bw;
                self.button(Rect::new(bx, by, bw, 26.0), "End", 0, Btn::Std, true, true, Hit::FreeEnd(i));
                let reason = free_reason(&it.effect);
                self.gfx.text(&reason, Rect::new(tx, by, bx - tx - 8.0, 26.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
            }
            y = card.b() + 14.0;
        }
        // kept list
        if keep_open && !keep.is_empty() {
            let kh = 48.0;
            let card = Rect::new(x, y, w, keep.len().min(12) as f32 * kh);
            self.gfx.card(card, 8.0, t.card, t.card_s);
            for (i, it) in keep.iter().take(12).enumerate() {
                let ry = y + i as f32 * kh;
                if i > 0 {
                    self.gfx.hline(x + 1.0, x + w - 1.0, ry, t.line);
                }
                let path = it.pids.first().and_then(|p| self.proc(*p)).map(|p| p.meta.path.clone()).unwrap_or_default();
                self.draw_icon(&path, Rect::new(x + 12.0, ry + 8.0, 16.0, 16.0), 0xE737);
                let size = model::fmt_mem(it.mem);
                let sw = self.gfx.measure(&size, Font::Ui, 12.5, 400);
                self.gfx.text(&it.label, Rect::new(x + 38.0, ry + 6.0, w - 50.0 - sw - 8.0, 18.0), Font::Ui, 13.0, 400, Align::Left, t.t1);
                self.gfx.text(&size, Rect::new(x + w - 12.0 - sw - 1.0, ry + 6.0, sw + 1.0, 18.0), Font::Ui, 12.5, 400, Align::Right, t.t2);
                let why = capitalize(&it.effect);
                self.gfx.text(&why, Rect::new(x + 38.0, ry + 24.0, w - 50.0, 18.0), Font::Ui, 12.0, 400, Align::Left, t.t2);
            }
            y = card.b() + 14.0;
        }
        // footer: pinned to the bottom when there is room
        let fy = (pr.b() - 16.0 - 32.0).max(y);
        let keep_label = format!("Better to keep ({})", keep.len());
        let kw = self.gfx.measure(&keep_label, Font::Ui, 13.0, 400) + 16.0 + 6.0 + 9.0;
        let kr = Rect::new(x - 8.0, fy, kw.ceil(), 32.0);
        self.hp(kr, 4.0, Hit::FreeKeep);
        self.gfx.text_at(&keep_label, kr.x + 8.0, fy, 32.0, kw, Font::Ui, 13.0, 400, t.t1);
        self.gfx.glyph(if keep_open { 0xE70E } else { 0xE70D }, Rect::new(kr.r() - 8.0 - 9.0, fy + 1.0, 9.0, 32.0), 9.0, t.t2);
        self.hit(kr, Hit::FreeKeep);
        if !go.is_empty() {
            let n = go.len();
            let label = if n == 1 { format!("End it · {}", model::fmt_mem(can)) } else { format!("End all {} · {}", NUM.get(n).copied().map(|s| s.to_string()).unwrap_or(n.to_string()), model::fmt_mem(can)) };
            let bw = self.btn_w(&label, 0, false);
            self.button(Rect::new(x + w - bw, fy, bw, 32.0), &label, 0, Btn::Accent, false, true, Hit::FreeAll);
        }
        fy + 32.0 + 16.0 - y0
    }
}

pub struct WhyItem {
    pub glyph: u32,
    pub title: String,
    pub value: String,
    pub code: String,
    pub button: Option<(String, Hit)>,
    pub muted: bool,
    pub crit: Option<String>,
}

/// Let long paths wrap after each backslash (zero-width space after it).
fn break_paths(s: &str) -> String {
    s.replace('\\', "\\\u{200B}")
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Short reason for a safe candidate.
fn free_reason(effect: &str) -> String {
    if effect.starts_with("nothing you use stops") { "Starts again when needed".into() } else { capitalize(effect) }
}
