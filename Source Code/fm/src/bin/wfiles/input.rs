//! Mouse, keyboard, actions, menus, dialogs and the `wfm gui` command set.
use crate::app::*;
use crate::fmt::Field;
use crate::gfx::Rect;
use crate::jobs;
use crate::model::*;
use crate::paint::{self, content_h, pane_geoms, row_geom};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const STYLES: [&str; 4] = ["Lowercase, words joined by -", "Lowercase, words joined by _", "Title Case With Spaces", "camelCase"];

const VK_F: u16 = 0x46;

impl App {
    // ------------------------------------------------------------------ helpers
    pub fn close_popups(&mut self) {
        let any = self.menu.is_some() || self.drop.is_some() || self.sub.is_some();
        self.menu = None;
        self.drop = None;
        self.sub = None;
        if any {
            self.invalidate();
        }
    }

    fn sel_items(&self) -> Vec<Item> {
        self.pane().selected()
    }

    fn sel_paths(&self) -> Vec<PathBuf> {
        self.pane().selected_paths()
    }

    pub fn jobs_changed(&mut self) {
        // finished jobs: refresh the folders they touched, show a toast once
        let done: Vec<jobs::Job> = {
            let q = self.jobs.0.lock().unwrap();
            q.jobs.iter().filter(|j| !j.active()).cloned().collect()
        };
        let mut refresh = false;
        for j in done {
            if self.seen_done(j.id) {
                continue;
            }
            refresh = true;
            if j.state == jobs::State::Done {
                let verb = if j.kind == jobs::Kind::Move { "Moved" } else { "Copied" };
                let n = j.srcs.len();
                let place = folder_title(&j.dest);
                let skipped = if j.identical > 0 { format!(" ({} identical skipped)", j.identical) } else { String::new() };
                self.toast(&format!("{verb} {n} item{} to ", if n == 1 { "" } else { "s" }), &format!("{place}{skipped}"), !j.journal.is_empty());
                self.toast_job = Some(j.id);
            } else if let jobs::State::Failed(e) = &j.state {
                self.toast(&format!("Could not finish: {e}"), "", false);
            }
        }
        if refresh {
            self.refresh_all();
        }
        self.invalidate();
    }

    fn seen_done(&mut self, id: u64) -> bool {
        if self.done_seen.contains(&id) {
            return true;
        }
        self.done_seen.insert(id);
        false
    }

    pub fn on_timer(&mut self, id: usize) {
        unsafe {
            match id {
                T_ANIM => {
                    let _ = KillTimer(Some(self.hwnd), T_ANIM);
                    self.invalidate();
                }
                T_RELIST => {
                    let _ = KillTimer(Some(self.hwnd), T_RELIST);
                    self.refresh_all();
                }
                T_SEARCH => {
                    let _ = KillTimer(Some(self.hwnd), T_SEARCH);
                    self.start_search();
                }
                T_TOAST => {
                    if let Some(t) = &self.toast {
                        if !t.hover && t.at.elapsed() > Duration::from_secs(6) {
                            self.toast = None;
                            let _ = KillTimer(Some(self.hwnd), T_TOAST);
                        }
                    } else {
                        let _ = KillTimer(Some(self.hwnd), T_TOAST);
                    }
                    self.invalidate();
                }
                9 => {
                    self.caret_on = !self.caret_on;
                    if self.path_edit.is_some() || self.search_focus || self.inline.is_some() || self.palette.is_some() || self.rename.is_some() {
                        self.invalidate();
                    }
                }
                _ => {}
            }
        }
    }

    fn the_dir(&self) -> Option<PathBuf> {
        self.pane().dir()
    }

    pub fn open_item(&mut self, it: &Item, new_tab: bool) {
        if it.recycled.is_some() {
            return;
        }
        if it.is_dir {
            if new_tab {
                self.new_tab(Loc::Folder(it.path.clone()));
            } else {
                self.go(Loc::Folder(it.path.clone()));
            }
            return;
        }
        // shortcuts to folders open here
        if it.ext() == "lnk"
            && let Some(l) = fmcore::info::read_shortcut(&it.path)
            && Path::new(&l.target).is_dir()
        {
            self.go(Loc::Folder(PathBuf::from(l.target)));
            return;
        }
        let p = it.path.clone();
        if let Err(e) = fmcore::shell::open(&p) {
            self.toast(&e, "", false);
        }
    }

    fn open_selection(&mut self, new_tab: bool) {
        let sel = self.sel_items();
        if sel.len() == 1 || (new_tab && !sel.is_empty()) {
            for it in sel {
                self.open_item(&it, new_tab);
            }
        } else {
            for it in sel.into_iter().filter(|i| !i.is_dir).take(20) {
                self.open_item(&it, false);
            }
        }
    }

    /// Run an action on the current selection (pill, menu, keys, palette, IPC).
    pub fn act(&mut self, a: Act) {
        let sel = self.sel_items();
        let paths: Vec<PathBuf> = sel.iter().map(|i| i.path.clone()).collect();
        let dir = self.the_dir();
        match a {
            Act::Open => self.open_selection(false),
            Act::OpenNewTab => self.open_selection(true),
            Act::Preview => self.preview_open(),
            Act::WhatIs => self.whatis_open(),
            Act::CopyPath => {
                let s = if paths.is_empty() { dir.map(|d| d.display().to_string()).unwrap_or_default() } else { paths.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join("\r\n") };
                crate::dnd::set_clipboard_text(self.hwnd, &s);
                self.toast("Path copied", "", false);
            }
            Act::Copy | Act::Cut => {
                if !paths.is_empty() && crate::dnd::set_clipboard_files(self.hwnd, &paths, a == Act::Cut) {
                    let n = paths.len();
                    self.toast(&format!("{} {n} item{}", if a == Act::Cut { "Cut" } else { "Copied" }, if n == 1 { "" } else { "s" }), "", false);
                }
            }
            Act::Paste => {
                let (files, cut) = crate::dnd::clipboard_files();
                let dest = if sel.len() == 1 && sel[0].is_dir { Some(sel[0].path.clone()) } else { dir };
                match dest {
                    Some(d) if !files.is_empty() => {
                        jobs::add(&self.jobs, if cut { jobs::Kind::Move } else { jobs::Kind::Copy }, files, d);
                    }
                    _ => self.toast("Nothing to paste", "", false),
                }
            }
            Act::Rename => {
                if sel.len() > 1 {
                    self.rename_open();
                } else if let Some(it) = sel.first() {
                    self.inline_rename(it.path.clone());
                }
            }
            Act::BatchRename => self.rename_open(),
            Act::Delete | Act::DeleteForGood => {
                if self.pane().loc == Loc::RecycleBin {
                    self.confirm = Some(("Delete these items for good? This cannot be undone.".into(), Act::DeleteForGood));
                    if a == Act::DeleteForGood {
                        self.confirm = None;
                        self.purge_selected();
                    }
                } else if !paths.is_empty() {
                    if a == Act::DeleteForGood {
                        self.confirm = Some((format!("Delete {} item{} for good? This cannot be undone.", paths.len(), if paths.len() == 1 { "" } else { "s" }), Act::DeleteForGood));
                    } else if self.ui.ask_delete {
                        self.confirm = Some((format!("Move {} item{} to the Recycle Bin?", paths.len(), if paths.len() == 1 { "" } else { "s" }), Act::Delete));
                    } else {
                        self.delete_now(paths, false);
                    }
                }
            }
            Act::Zip => {
                let Some(d) = dir else { return };
                if paths.is_empty() {
                    return;
                }
                if sel.len() == 1 && sel[0].is_archive() {
                    return self.act(Act::ExtractTo);
                }
                let stem = if paths.len() == 1 { sel[0].name.rsplit_once('.').filter(|_| !sel[0].is_dir).map(|x| x.0.to_string()).unwrap_or(sel[0].name.clone()) } else { folder_title(&d) };
                let mut out = d.join(format!("{stem}.zip"));
                if out.exists() {
                    out = fmcore::ops::unique_name(&out);
                }
                self.toast("Zipping...", "", false);
                self.spawn(move || match fmcore::zip::zip(&paths, &out) {
                    Ok(p) => Msg::Toast { text: format!("Zipped to {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), undo: true },
                    Err(e) => Msg::Toast { text: e, undo: false },
                });
            }
            Act::ExtractTo | Act::ExtractHere => {
                let (Some(d), Some(it)) = (dir, sel.first().cloned()) else { return };
                let target = if a == Act::ExtractHere {
                    d
                } else {
                    let stem = it.name.split('.').next().unwrap_or(&it.name).to_string();
                    d.join(stem)
                };
                self.toast("Extracting...", "", false);
                self.spawn(move || match fmcore::zip::unzip(&it.path, &target) {
                    Ok(p) => Msg::Toast { text: format!("Extracted to {}", folder_title(&p)), undo: true },
                    Err(e) => Msg::Toast { text: e, undo: false },
                });
            }
            Act::NewFolder => {
                let Some(d) = dir else { return };
                let mut p = d.join("New folder");
                if p.exists() {
                    p = fmcore::ops::unique_name(&p);
                }
                match fmcore::ops::mkdir(&p) {
                    Ok(p) => {
                        self.pending_rename = Some(p.clone());
                        self.pane_mut().want = Some(p);
                        let (t, pi) = (self.cur, self.tab().active);
                        self.load(t, pi, true);
                    }
                    Err(e) => self.toast(&e, "", false),
                }
            }
            Act::NewText => {
                let Some(d) = dir else { return };
                let mut p = d.join("New Text Document.txt");
                if p.exists() {
                    p = fmcore::ops::unique_name(&p);
                }
                match fmcore::ops::new_file(&p) {
                    Ok(p) => {
                        self.pending_rename = Some(p.clone());
                        self.pane_mut().want = Some(p);
                        let (t, pi) = (self.cur, self.tab().active);
                        self.load(t, pi, true);
                    }
                    Err(e) => self.toast(&e, "", false),
                }
            }
            Act::NewMenu => {
                let r = self.hits.iter().find(|h| h.1 == Hit::New).map(|h| h.0).unwrap_or(Rect::new(self.w - 140.0, TITLE_H + 8.0, 70.0, 32.0));
                self.open_drop(DropKind::New, r, 220.0);
            }
            Act::Properties => {
                let p = paths.first().cloned().or(dir);
                if let Some(p) = p {
                    let w = fmcore::util::wide(p.as_os_str());
                    unsafe {
                        let _ = windows::Win32::UI::Shell::SHObjectProperties(Some(self.hwnd), windows::Win32::UI::Shell::SHOP_FILEPATH, windows::core::PCWSTR(w.as_ptr()), None);
                    }
                }
            }
            Act::SelectAll => {
                let pane = self.pane_mut();
                let all: Vec<PathBuf> = pane.rows().iter().map(|i| i.path.clone()).collect();
                pane.sel = all.into_iter().collect();
            }
            Act::Terminal => {
                let d = if sel.len() == 1 && sel[0].is_dir { Some(sel[0].path.clone()) } else { dir };
                if let Some(d) = d {
                    let ok = std::process::Command::new("wt.exe").arg("-d").arg(&d).spawn().is_ok();
                    if !ok {
                        let _ = std::process::Command::new("powershell.exe").current_dir(&d).spawn();
                    }
                }
            }
            Act::CopyToMenu | Act::MoveToMenu => {
                let r = Rect::new(self.mx, self.my, 1.0, 1.0);
                self.open_drop(if a == Act::CopyToMenu { DropKind::CopyTo } else { DropKind::MoveTo }, r, 260.0);
            }
            Act::CopyOther | Act::MoveOther => {
                let tab = self.tab();
                if !tab.split() || paths.is_empty() {
                    return;
                }
                let other = 1 - tab.active;
                if let Some(d) = tab.panes[other].dir() {
                    jobs::add(&self.jobs, if a == Act::MoveOther { jobs::Kind::Move } else { jobs::Kind::Copy }, paths, d);
                }
            }
            Act::Split => self.toggle_split(),
            Act::Settings => self.open_settings(),
            Act::Undo => self.undo(),
            Act::Pin => {
                let p = if sel.len() == 1 && sel[0].is_dir { Some(sel[0].path.clone()) } else { dir };
                if let Some(p) = p {
                    let pinned = fmcore::pins::pins().iter().any(|x| fmcore::util::path_eq(Path::new(&x.path), &p));
                    let r = if pinned { fmcore::pins::unpin(&p.to_string_lossy()) } else { fmcore::pins::pin(&p, None) };
                    match r {
                        Ok(_) => {
                            self.build_side();
                            self.toast(if pinned { "Removed from the sidebar: " } else { "Pinned: " }, &folder_title(&p), false);
                        }
                        Err(e) => self.toast(&e, "", false),
                    }
                }
            }
            Act::Restore => {
                let recs: Vec<fmcore::recycle::Recycled> = sel.iter().filter_map(|i| i.recycled.and_then(|k| self.recycle.get(k).cloned())).collect();
                let n = recs.len();
                let mut err = None;
                for r in recs {
                    if let Err(e) = fmcore::recycle::restore(&r) {
                        err = Some(e);
                    }
                }
                match err {
                    Some(e) => self.toast(&e, "", false),
                    None => self.toast(&format!("Restored {n} item{}", if n == 1 { "" } else { "s" }), "", false),
                }
                self.refresh_all();
            }
            Act::ViewMenu => {
                let r = self.hits.iter().find(|h| h.1 == Hit::ViewBtn).map(|h| h.0).unwrap_or(Rect::new(self.w - 60.0, TITLE_H + 8.0, 53.0, 32.0));
                self.open_drop(DropKind::View, r, 220.0);
            }
            Act::SortMenu => {
                let r = Rect::new(self.mx, self.my, 1.0, 1.0);
                self.open_drop(DropKind::Sort, r, 200.0);
            }
            Act::More => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Pill(5)).map(|h| h.0).unwrap_or(Rect::new(self.w / 2.0, self.h - 90.0, 36.0, 36.0));
                // "More" = the full right-click menu above the pill
                self.context_menu(r.x - 120.0, r.y - 8.0, false);
                let h = self.menu_rect().map(|x| x.h).unwrap_or(400.0);
                if let Some(m2) = self.menu.as_mut() {
                    m2.y = (r.y - 8.0 - h).max(TITLE_H);
                }
            }
            Act::Transfers => self.xfer_open = !self.xfer_open,
            Act::Palette => self.palette_open(""),
            Act::Refresh => self.refresh_all(),
            Act::ToggleHidden => {
                self.ui.hidden = !self.ui.hidden;
                self.save_ui();
                self.refresh_all();
            }
            Act::OpenWith => {
                if let Some(it) = sel.first().cloned().filter(|i| !i.is_dir) {
                    let p = it.path.clone();
                    let ext = it.ext();
                    self.spawn(move || Msg::OpenWith { path: p, apps: fmcore::shell::open_with_list(&ext, false).unwrap_or_default() });
                }
            }
            Act::None => {}
        }
        self.invalidate();
    }

    fn delete_now(&mut self, paths: Vec<PathBuf>, permanent: bool) {
        let n = paths.len();
        self.spawn(move || match fmcore::ops::delete(&paths, permanent) {
            Ok(_) => Msg::Toast { text: format!("{} {n} item{}", if permanent { "Deleted" } else { "Moved to the Recycle Bin:" }, if n == 1 { "" } else { "s" }), undo: !permanent },
            Err(e) => Msg::Toast { text: e, undo: false },
        });
    }

    fn purge_selected(&mut self) {
        let recs: Vec<fmcore::recycle::Recycled> = self.sel_items().iter().filter_map(|i| i.recycled.and_then(|k| self.recycle.get(k).cloned())).collect();
        for r in &recs {
            let _ = fmcore::recycle::purge(r);
        }
        self.refresh_all();
    }

    pub fn undo(&mut self) {
        self.spawn(|| match fmcore::journal::undo_last() {
            Ok(op) => {
                let n = op.created.len().max(op.moved.len()).max(op.recycled.len()).max(1);
                let what = match op.kind.as_str() {
                    "rename" | "batch-rename" => "rename",
                    "delete" | "trash" => "delete",
                    "mkdir" | "new-file" => "new item",
                    k => k,
                };
                Msg::Toast { text: format!("Undone: {what} of {n} item{}", if n == 1 { "" } else { "s" }), undo: false }
            }
            Err(e) => Msg::Toast { text: e, undo: false },
        });
        self.toast = None;
        self.timer(T_RELIST, 400);
    }

    pub fn save_ui(&self) {
        let mut cfg = fmcore::pins::config();
        cfg["ui"] = json!({"theme": self.ui.theme, "folder_sizes": self.ui.sizes, "extensions": self.ui.ext, "hidden": self.ui.hidden, "ask_delete": self.ui.ask_delete, "scope_pc": self.ui.scope_pc});
        let _ = fmcore::pins::save_config(&cfg);
    }

    fn open_settings(&mut self) {
        if let Some(i) = self.tabs.iter().position(|t| t.pane().loc == Loc::Settings) {
            self.cur = i;
        } else {
            self.new_tab(Loc::Settings);
        }
        self.refresh_ai_flags();
    }

    pub fn toggle_split(&mut self) {
        let tab = self.tab_mut();
        if tab.split() {
            let keep = tab.active;
            let p = tab.panes.remove(keep);
            tab.panes = vec![p];
            tab.active = 0;
        } else {
            let loc = tab.pane().loc.clone();
            let loc = if loc == Loc::Settings { known("Downloads").map(Loc::Folder).unwrap_or(Loc::ThisPc) } else { loc };
            tab.panes.push(Pane::new(loc));
            tab.active = 1;
            tab.split_at = Instant::now();
            let t = self.cur;
            self.load(t, 1, false);
        }
        self.whatis = None;
        self.anim();
    }

    // ------------------------------------------------------------------ selection
    pub fn select_row(&mut self, p: usize, i: usize, ctrl: bool, shift: bool) {
        let tab = self.tab_mut();
        tab.active = p;
        let pane = &mut tab.panes[p];
        let Some(path) = pane.rows().get(i).map(|x| x.path.clone()) else { return };
        if shift {
            let a = pane.anchor.unwrap_or(i);
            let (lo, hi) = (a.min(i), a.max(i));
            if !ctrl {
                pane.sel.clear();
            }
            let paths: Vec<PathBuf> = pane.rows()[lo..=hi].iter().map(|x| x.path.clone()).collect();
            pane.sel.extend(paths);
        } else if ctrl {
            if !pane.sel.remove(&path) {
                pane.sel.insert(path);
            }
            pane.anchor = Some(i);
        } else {
            pane.sel.clear();
            pane.sel.insert(path);
            pane.anchor = Some(i);
        }
        pane.focus = Some(i);
        if let Some(w) = self.whatis.as_ref()
            && w.item.path != self.pane().rows()[i].path
            && self.pane().sel.len() == 1
        {
            self.whatis_open();
        }
    }

    fn move_focus(&mut self, d: i32, shift: bool, ctrl: bool) {
        let (t, p) = (self.cur, self.tab().active);
        let grid = self.tab().view == View::Grid && !self.pane().searching();
        let cols = if grid { paint::grid_cols(pane_geoms(self, t)[p].list) as i32 } else { 1 };
        let pane = self.pane_mut();
        let n = pane.rows().len() as i32;
        if n == 0 {
            return;
        }
        let step = match d {
            100 => -n,
            200 => n,
            300 => -10,
            400 => 10,
            -2 => -cols,
            2 => cols,
            x => x,
        };
        let cur = pane.focus.map(|f| f as i32).unwrap_or(-1);
        let next = if cur < 0 { 0 } else { (cur + step).clamp(0, n - 1) } as usize;
        pane.kb = true;
        if ctrl && !shift {
            pane.focus = Some(next);
        } else {
            self.select_row(p, next, false, shift);
        }
        self.scroll_to(t, p, next);
    }

    // ------------------------------------------------------------------ mouse
    pub fn mouse_move(&mut self, x: f32, y: f32, down: bool) {
        self.mx = x;
        self.my = y;
        if let Some((p, y0, s0)) = self.drag_scroll {
            let t = self.cur;
            let g = pane_geoms(self, t)[p];
            let ch = content_h(self, t, p);
            let th = (g.list.h * g.list.h / ch).max(30.0);
            let pane = &mut self.tabs[t].panes[p];
            pane.scroll = (s0 + (y - y0) * (ch - g.list.h) / (g.list.h - th).max(1.0)).max(0.0);
            self.invalidate();
            return;
        }
        if down
            && let Some((x0, y0, p, i)) = self.press
            && ((x - x0).abs() > 5.0 || (y - y0).abs() > 5.0)
        {
            self.press = None;
            let pane = &self.tab().panes[p];
            let it = pane.rows().get(i).cloned();
            if let Some(it) = it {
                let paths = if pane.sel.contains(&it.path) { pane.selected_paths() } else { vec![it.path.clone()] };
                if !paths.is_empty() && it.recycled.is_none() {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    self.dragging_out = true;
                    let _ = crate::dnd::drag_out(&paths);
                    self.dragging_out = false;
                    self.drag_in = None;
                    self.timer(T_RELIST, 300);
                }
            }
            return;
        }
        if self.nc_hover != Hit::None {
            self.nc_hover = Hit::None;
        }
        let h = self.hit_at(x, y);
        if h != self.hover {
            self.hover = h;
            // submenus open on hover
            if let Hit::MenuItem(i) = h
                && let Some(m) = &self.menu
                && m.items.get(i).map(|it| it.sub).unwrap_or(false)
            {
                self.open_sub(i);
            } else if let Hit::MenuItem(_) = h {
                self.sub = None;
            }
            if let Some(t) = self.toast.as_mut() {
                t.hover = matches!(h, Hit::Toast(_)) || matches!(h, Hit::Block) && t.hover;
            }
            unsafe {
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
            }
            self.invalidate();
        }
    }

    pub fn mouse_up(&mut self, _x: f32, _y: f32) {
        self.press = None;
        self.drag_scroll = None;
        self.pressed = Hit::None;
        self.invalidate();
    }

    pub fn middle_click(&mut self, x: f32, y: f32) {
        match self.hit_at(x, y) {
            Hit::Tab(i) | Hit::TabClose(i) => self.close_tab(i),
            Hit::Row(p, i) => {
                if let Some(it) = self.tab().panes[p].rows().get(i).cloned()
                    && it.is_dir
                {
                    self.new_tab(Loc::Folder(it.path));
                }
            }
            _ => {}
        }
        self.invalidate();
    }

    pub fn wheel(&mut self, x: f32, y: f32, d: f32) {
        if self.menu.is_some() || self.drop.is_some() {
            self.close_popups();
        }
        if let Some(pv) = self.pv.as_mut() {
            pv.scroll = (pv.scroll - d / 120.0 * 63.0).max(0.0);
            self.invalidate();
            return;
        }
        let t = self.cur;
        let geoms = pane_geoms(self, t);
        for (p, g) in geoms.iter().enumerate() {
            if g.all.has(x, y) || self.pane().loc == Loc::Settings {
                let pane = &mut self.tabs[t].panes[p];
                pane.scroll = (pane.scroll - d / 120.0 * 96.0).max(0.0);
                break;
            }
        }
        self.invalidate();
    }

    pub fn right_click(&mut self, x: f32, y: f32) {
        self.drop = None;
        self.sub = None;
        if self.pv.is_some() || self.rename.is_some() || self.palette.is_some() {
            return;
        }
        match self.hit_at(x, y) {
            Hit::Row(p, i) => {
                let path = self.tab().panes[p].rows().get(i).map(|x| x.path.clone());
                if let Some(path) = path
                    && !self.tab().panes[p].sel.contains(&path)
                {
                    self.select_row(p, i, false, false);
                }
                self.tab_mut().active = p;
                self.context_menu(x, y, false);
            }
            Hit::ListBg(p) | Hit::PaneHead(p) => {
                self.tab_mut().active = p;
                self.pane_mut().sel.clear();
                if self.pane().loc != Loc::Settings {
                    self.context_menu(x, y, true);
                }
            }
            Hit::Tab(i) => {
                self.cur = i;
            }
            _ => self.menu = None,
        }
        self.invalidate();
    }

    pub fn mouse_down(&mut self, x: f32, y: f32, ctrl: bool, shift: bool, dbl: bool) {
        let h = self.hit_at(x, y);
        self.pressed = h;
        // clicks outside a popup close it
        let in_menu = matches!(h, Hit::MenuItem(_) | Hit::MenuQuick(_) | Hit::SubItem(_) | Hit::Drop(_)) || (matches!(h, Hit::Block) && (self.menu_rect().map(|r| r.has(x, y)).unwrap_or(false) || self.drop_rect(false).map(|r| r.has(x, y)).unwrap_or(false) || self.drop_rect(true).map(|r| r.has(x, y)).unwrap_or(false)));
        if !in_menu && (self.menu.is_some() || self.drop.is_some()) {
            let drop_was = self.drop.as_ref().map(|d| d.kind);
            self.close_popups();
            // a click on the same dropdown button just closes it
            if matches!((h, drop_was), (Hit::ViewBtn, Some(DropKind::View)) | (Hit::New, Some(DropKind::New))) {
                return;
            }
        }
        if self.xfer_open && !matches!(h, Hit::Xfer | Hit::Job(..) | Hit::Clash(..) | Hit::SameChoice(_) | Hit::XferHead(_) | Hit::Block) {
            self.xfer_open = false;
        }
        if self.palette.is_some() && !matches!(h, Hit::Pal(_) | Hit::Block) {
            self.palette = None;
        }
        if let Some(c) = &self.confirm {
            let act = c.1;
            match h {
                Hit::Dlg(41) => {
                    self.confirm = None;
                    let paths = self.sel_paths();
                    if self.pane().loc == Loc::RecycleBin {
                        self.purge_selected();
                    } else {
                        self.delete_now(paths, act == Act::DeleteForGood);
                    }
                }
                Hit::Dlg(40) => self.confirm = None,
                _ => {}
            }
            self.invalidate();
            return;
        }
        // leaving an edit commits / cancels it
        if self.inline.is_some() && !matches!(h, Hit::Row(..)) {
            self.commit_inline();
        }
        if self.key_focus && h != Hit::Set(15) && h != Hit::Set(16) {
            self.key_focus = false;
        }
        if self.path_edit.is_some() && h != Hit::Omni {
            self.path_edit = None;
        }
        if !matches!(h, Hit::Omni | Hit::OmniClear | Hit::Scope(_) | Hit::Filter(_) | Hit::Meaning | Hit::Drop(_) | Hit::Row(..) | Hit::Block) {
            self.search_focus = false;
        }
        match h {
            Hit::Tab(i) => {
                self.cur = i;
                self.path_edit = None;
                self.search_focus = false;
                self.whatis = None;
            }
            Hit::TabClose(i) => self.close_tab(i),
            Hit::TabAdd => {
                let loc = self.pane().loc.clone();
                self.new_tab(if loc == Loc::Settings { Loc::ThisPc } else { loc });
            }
            Hit::Back => self.back(),
            Hit::Fwd => self.forward(),
            Hit::Up => self.up(),
            Hit::Crumb(i) => {
                let crumbs = self.crumbs_pub();
                if let Some((_, loc)) = crumbs.get(i) {
                    let loc = loc.clone();
                    self.go(loc);
                }
            }
            Hit::Omni => {
                if self.pane().search.is_some() {
                    self.search_focus = true;
                    let o = self.omni_rect();
                    if let Some(s) = self.pane_mut().search.as_mut() {
                        let idx = 0;
                        let _ = (o, idx);
                    }
                } else if self.path_edit.is_none() {
                    self.edit_path();
                } else {
                    let o = self.omni_rect();
                    let text = self.path_edit.as_ref().unwrap().text.clone();
                    let i = self.gfx.index_at(&text, crate::gfx::Font::Ui, 13.0, 400, x - o.x - 12.0);
                    let f = self.path_edit.as_mut().unwrap();
                    f.caret = i;
                    if !shift {
                        f.anchor = i;
                    }
                }
            }
            Hit::OmniSearch => self.search_mode(""),
            Hit::OmniClear => self.end_search(),
            Hit::New => self.act(Act::NewMenu),
            Hit::ViewBtn => self.act(Act::ViewMenu),
            Hit::Side(i) => self.side_click(i),
            Hit::SideSettings => self.open_settings(),
            Hit::Header(p, c) => {
                let pane = &mut self.tab_mut().panes[p];
                let col = [SortCol::Name, SortCol::Date, SortCol::Size][c as usize];
                pane.sort = if pane.sort.0 == col { (col, !pane.sort.1) } else { (col, col != SortCol::Name) };
                pane.sort_items();
            }
            Hit::Row(p, i) => {
                self.tab_mut().active = p;
                if let Some(inl) = &self.inline
                    && self.tab().panes[p].rows().get(i).map(|r| r.path == inl.path).unwrap_or(false)
                {
                    return;
                }
                if self.inline.is_some() {
                    self.commit_inline();
                }
                if self.pane().loc == Loc::Settings {
                    return;
                }
                if dbl {
                    if let Some(it) = self.tab().panes[p].rows().get(i).cloned() {
                        self.open_item(&it, ctrl);
                    }
                } else {
                    let path = self.tab().panes[p].rows().get(i).map(|x| x.path.clone());
                    let already = path.as_ref().map(|x| self.tab().panes[p].sel.contains(x)).unwrap_or(false);
                    if !(already && !ctrl && !shift) {
                        self.select_row(p, i, ctrl, shift);
                    } else {
                        self.tab_mut().panes[p].focus = Some(i);
                    }
                    self.tab_mut().panes[p].kb = false;
                    self.press = Some((x, y, p, i));
                }
            }
            Hit::ListBg(p) => {
                self.tab_mut().active = p;
                if !ctrl {
                    self.tab_mut().panes[p].sel.clear();
                }
            }
            Hit::PaneHead(p) => self.tab_mut().active = p,
            Hit::Scroll(p) => {
                let s = self.tab().panes[p].scroll;
                self.drag_scroll = Some((p, y, s));
            }
            Hit::Pill(id) => match id {
                0 => self.act(Act::Copy),
                1 => self.act(Act::Cut),
                2 => self.act(Act::Rename),
                3 => self.act(Act::Delete),
                4 => self.act(Act::Zip),
                5 => self.act(Act::More),
                10 => self.act(Act::Paste),
                11 => self.act(Act::NewFolder),
                12 => self.act(Act::SelectAll),
                13 => self.act(Act::Restore),
                _ => {}
            },
            Hit::Xfer => self.xfer_open = !self.xfer_open,
            Hit::XferHead(_) => jobs::toggle_pause(&self.jobs, None),
            Hit::Job(id, a) => match a {
                0 => jobs::toggle_pause(&self.jobs, Some(id)),
                1 => jobs::cancel(&self.jobs, id),
                _ => match jobs::undo(&self.jobs, id) {
                    Ok(_) => {
                        self.toast("Undone", "", false);
                        self.refresh_all();
                    }
                    Err(e) => self.toast(&e, "", false),
                },
            },
            Hit::Clash(id, k) => {
                let choice = [fmcore::ops::Conflict::Skip, fmcore::ops::Conflict::Replace, fmcore::ops::Conflict::KeepBoth][k as usize];
                let all = self.jobs.0.lock().unwrap().jobs.iter().find(|j| j.id == id).map(|j| j.same_for_rest).unwrap_or(false);
                jobs::decide(&self.jobs, id, 0, choice, all);
            }
            Hit::SameChoice(id) => {
                let mut q = self.jobs.0.lock().unwrap();
                if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
                    j.same_for_rest = !j.same_for_rest;
                }
            }
            Hit::Toast(k) => {
                if k == 1 {
                    let job = self.toast_job.take();
                    self.toast = None;
                    match job.map(|id| jobs::undo(&self.jobs, id)) {
                        Some(Ok(_)) => {
                            self.toast("Undone", "", false);
                            self.refresh_all();
                        }
                        Some(Err(e)) => self.toast(&e, "", false),
                        None => self.undo(),
                    }
                } else {
                    self.toast = None;
                }
            }
            Hit::Scope(i) => {
                if let Some(s) = self.pane_mut().search.as_mut() {
                    s.pc = i == 1;
                }
                self.ui.scope_pc = i == 1;
                self.save_ui();
                self.search_focus = true;
                self.start_search();
            }
            Hit::Filter(i) => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Filter(i)).map(|h| h.0).unwrap_or_default();
                self.open_drop([DropKind::FType, DropKind::FSize, DropKind::FDate][i as usize], r, 200.0);
            }
            Hit::Meaning => {
                if let Some(s) = self.pane_mut().search.as_mut() {
                    s.meaning = !s.meaning;
                }
                self.start_search();
            }
            Hit::MenuQuick(i) => {
                self.menu = None;
                self.act([Act::Cut, Act::Copy, Act::Rename, Act::Delete][i as usize]);
            }
            Hit::MenuItem(i) => self.menu_click(i),
            Hit::SubItem(i) => self.sub_click(i),
            Hit::Drop(i) => self.drop_click(i),
            Hit::Pv(k) => self.preview_click(k),
            Hit::Pal(i) => {
                if let Some(p) = self.palette.as_mut() {
                    p.sel = i;
                }
                self.palette_run(false);
            }
            Hit::Dlg(k) => self.dialog_click(k, x),
            Hit::Set(k) => self.settings_click(k),
            Hit::WhatIs(k) => match k {
                0 => self.whatis = None,
                1 => {
                    if let Some(w) = &self.whatis {
                        let p = w.item.path.clone();
                        self.select_path(&p);
                        self.act(Act::ExtractHere);
                    }
                }
                2 => {
                    if let Some(w) = &self.whatis {
                        let p = w.item.path.clone();
                        self.select_path(&p);
                        self.act(Act::Delete);
                        self.whatis = None;
                    }
                }
                _ => {
                    if let Some(w) = &self.whatis {
                        let it = w.item.clone();
                        self.open_item(&it, false);
                    }
                }
            },
            _ => {}
        }
        if let Some(p) = self.pending_rename.take() {
            self.pending_rename = Some(p);
        }
        self.invalidate();
    }

    pub fn select_path(&mut self, p: &Path) {
        let pane = self.pane_mut();
        if let Some(i) = pane.rows().iter().position(|r| fmcore::util::path_eq(&r.path, p)) {
            pane.sel.clear();
            pane.sel.insert(pane.rows()[i].path.clone());
            pane.focus = Some(i);
            pane.anchor = Some(i);
        }
    }

    fn side_click(&mut self, i: usize) {
        if i == usize::MAX {
            // rail menu button: leave split view to get the full sidebar back
            if self.tab().split() {
                self.toggle_split();
            }
            return;
        }
        let side_n = self.side.len();
        let drives_n = self.drives.len();
        if i < side_n {
            let loc = self.side[i].3.clone();
            self.go(loc);
        } else if i < side_n + drives_n {
            let root = self.drives[i - side_n].root.clone();
            self.go(Loc::Folder(root.into()));
        } else if let Some(r) = self.recent.get(i - side_n - drives_n).cloned() {
            let p = PathBuf::from(&r.target);
            if p.is_dir() {
                self.go(Loc::Folder(p));
            } else if let Some(parent) = p.parent() {
                self.go(Loc::Folder(parent.to_path_buf()));
                self.pane_mut().want = Some(p);
            }
        }
    }

    pub fn crumbs_pub(&self) -> Vec<(String, Loc)> {
        let loc = &self.pane().loc;
        let mut v = vec![];
        if let Loc::Folder(p) = loc {
            let s = p.to_string_lossy().to_string();
            if s.starts_with(r"\\") {
                let parts: Vec<&str> = s.trim_start_matches('\\').split('\\').filter(|x| !x.is_empty()).collect();
                let mut cur = format!(r"\\{}", parts.first().unwrap_or(&""));
                if parts.len() > 1 {
                    cur = format!(r"{cur}\{}", parts[1]);
                }
                v.push((cur.clone(), Loc::Folder(cur.clone().into())));
                for part in parts.iter().skip(2) {
                    cur = format!(r"{cur}\{part}");
                    v.push((part.to_string(), Loc::Folder(cur.clone().into())));
                }
            } else {
                v.push(("This PC".into(), Loc::ThisPc));
                let mut acc = PathBuf::new();
                for (i, c) in p.components().enumerate() {
                    acc.push(c.as_os_str());
                    if i == 0 {
                        acc.push("\\");
                        v.push((String::new(), Loc::Folder(acc.clone())));
                    } else if let std::path::Component::Normal(n) = c {
                        v.push((n.to_string_lossy().into_owned(), Loc::Folder(acc.clone())));
                    }
                }
            }
        } else {
            v.push((loc.title(), loc.clone()));
        }
        v
    }

    // ------------------------------------------------------------------ omnibox
    pub fn edit_path(&mut self) {
        let mut f = Field::new(&self.pane().loc.display());
        f.select_all();
        self.path_edit = Some(f);
        self.search_focus = false;
    }

    pub fn search_mode(&mut self, first: &str) {
        let pc = self.ui.scope_pc;
        let pane = self.pane_mut();
        if pane.search.is_none() {
            pane.search = Some(Search::new(first, pc));
        } else if let Some(s) = pane.search.as_mut() {
            s.field.insert(first);
        }
        self.search_focus = true;
        self.path_edit = None;
        if !first.is_empty() {
            self.timer(T_SEARCH, 30);
        }
    }

    pub fn end_search(&mut self) {
        let pane = self.pane_mut();
        pane.search = None;
        pane.sel.clear();
        pane.scroll = 0.0;
        self.search_focus = false;
    }

    fn commit_path(&mut self) {
        let Some(f) = self.path_edit.take() else { return };
        let s = f.text.trim().trim_matches('"').to_string();
        if s.is_empty() {
            return;
        }
        if let Some(cmd) = s.strip_prefix('>') {
            self.palette_open(cmd.trim());
            return;
        }
        let expanded = expand_env(&s);
        let looks_path = expanded.len() >= 2 && (expanded.as_bytes()[1] == b':' || expanded.starts_with(r"\\") || expanded.starts_with('~'));
        if looks_path || Path::new(&expanded).exists() {
            let p = if let Some(rest) = expanded.strip_prefix('~') { std::env::var("USERPROFILE").map(|h| format!("{h}{rest}")).unwrap_or(expanded.clone()) } else { expanded.clone() };
            let p = PathBuf::from(p);
            if p.is_dir() {
                self.go(Loc::Folder(fmcore::util::abs(&p)));
            } else if p.is_file() {
                if let Some(parent) = p.parent() {
                    self.go(Loc::Folder(parent.to_path_buf()));
                    self.pane_mut().want = Some(p.clone());
                }
            } else {
                self.toast(&format!("Not found: {}", p.display()), "", false);
            }
        } else if s.eq_ignore_ascii_case("this pc") {
            self.go(Loc::ThisPc);
        } else {
            self.search_mode(&s);
        }
    }

    // ------------------------------------------------------------------ inline rename
    fn inline_rename(&mut self, path: PathBuf) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut f = Field::new(&name);
        f.anchor = 0;
        let stem = if path.is_dir() { name.clone() } else { name.rsplit_once('.').map(|x| x.0.to_string()).unwrap_or(name.clone()) };
        f.caret = stem.encode_utf16().count();
        f.anchor = 0;
        let p = self.tab().active;
        self.inline = Some(Inline { pane: p, path, field: f });
        self.search_focus = false;
        self.path_edit = None;
    }

    fn commit_inline(&mut self) {
        let Some(inl) = self.inline.take() else { return };
        let old = inl.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let new = inl.field.text.trim().to_string();
        if new.is_empty() || new == old {
            return;
        }
        match fmcore::ops::rename(&inl.path, &new) {
            Ok(p) => {
                self.pane_mut().want = Some(p);
                self.toast("Renamed to ", &new, true);
                let (t, pi) = (self.cur, self.tab().active);
                self.load(t, pi, true);
            }
            Err(e) => self.toast(&e, "", false),
        }
    }

    // ------------------------------------------------------------------ keyboard
    pub fn char_in(&mut self, c: char) {
        if (c as u32) < 32 || c == '\u{7f}' {
            return;
        }
        let s = c.to_string();
        if self.key_focus {
            self.key_field.insert(&s);
            self.invalidate();
            return;
        }
        if let Some(r) = self.rename.as_mut() {
            match (r.mode, r.focus) {
                (0 | 2, 0) => r.pattern.insert(&s),
                (0 | 2, _) => {
                    if c.is_ascii_digit() {
                        r.start.insert(&s)
                    }
                }
                (1, 0) => r.find.insert(&s),
                (1, _) => r.repl.insert(&s),
                _ => {}
            }
            rename_preview(self);
        } else if let Some(p) = self.palette.as_mut() {
            p.field.insert(&s);
            self.palette_fill();
        } else if let Some(i) = self.inline.as_mut() {
            if !"\\/:*?\"<>|".contains(c) {
                i.field.insert(&s);
            }
        } else if let Some(f) = self.path_edit.as_mut() {
            f.insert(&s);
        } else if self.pv.is_some() || self.pane().loc == Loc::Settings {
        } else if self.search_focus && self.pane().search.is_some() {
            if let Some(se) = self.pane_mut().search.as_mut() {
                se.field.insert(&s);
            }
            self.timer(T_SEARCH, 30);
        } else if c != ' ' {
            // typing anywhere in the list starts a search
            self.search_mode(&s);
        }
        self.caret_on = true;
        self.invalidate();
    }

    fn field_mut(&mut self) -> Option<&mut Field> {
        if self.rename.is_some() {
            let r = self.rename.as_mut().unwrap();
            return Some(match (r.mode, r.focus) {
                (0 | 2, 0) => &mut r.pattern,
                (0 | 2, _) => &mut r.start,
                (1, 0) => &mut r.find,
                _ => &mut r.repl,
            });
        }
        if self.palette.is_some() {
            return self.palette.as_mut().map(|p| &mut p.field);
        }
        if self.inline.is_some() {
            return self.inline.as_mut().map(|i| &mut i.field);
        }
        if self.path_edit.is_some() {
            return self.path_edit.as_mut();
        }
        if self.search_focus {
            return self.pane_mut().search.as_mut().map(|s| &mut s.field);
        }
        None
    }

    /// Returns true when the key was handled.
    pub fn key(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.caret_on = true;
        let handled = self.key_inner(vk, ctrl, shift, alt);
        self.invalidate();
        handled
    }

    fn key_inner(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        const ESC: u16 = 0x1B;
        const ENTER: u16 = 0x0D;
        const SPACE: u16 = 0x20;
        const TAB: u16 = 0x09;
        // confirm box
        if self.confirm.is_some() {
            match vk {
                ESC => self.confirm = None,
                ENTER => {
                    let (_, act) = self.confirm.take().unwrap();
                    let paths = self.sel_paths();
                    if self.pane().loc == Loc::RecycleBin {
                        self.purge_selected();
                    } else {
                        self.delete_now(paths, act == Act::DeleteForGood);
                    }
                }
                _ => {}
            }
            return true;
        }
        if self.menu.is_some() || self.drop.is_some() {
            if vk == ESC {
                self.close_popups();
                return true;
            }
        }
        // API key field (masked; never logged)
        if self.key_focus {
            match vk {
                ESC => {
                    self.key_focus = false;
                    self.key_field = Field::default();
                }
                ENTER => self.save_key(),
                _ => {
                    if ctrl && vk == 0x56 {
                        if let Some(s) = crate::dnd::clipboard_text(self.hwnd) {
                            self.key_field.insert(s.trim());
                        }
                    } else if !(ctrl && (vk == 0x43 || vk == 0x58)) {
                        self.key_field.key(vk, ctrl, shift);
                    }
                }
            }
            return true;
        }
        // text fields
        if self.rename.is_some() {
            match vk {
                ESC => self.rename = None,
                ENTER => self.rename_apply(),
                TAB => {
                    let r = self.rename.as_mut().unwrap();
                    r.focus = 1 - r.focus.min(1);
                }
                _ => {
                    if ctrl && vk == 0x56 {
                        if let Some(s) = crate::dnd::clipboard_text(self.hwnd)
                            && let Some(f) = self.field_mut()
                        {
                            f.insert(&s);
                        }
                    } else if let Some(f) = self.field_mut() {
                        f.key(vk, ctrl, shift);
                    }
                    rename_preview(self);
                }
            }
            return true;
        }
        if self.palette.is_some() {
            match vk {
                ESC => self.palette = None,
                ENTER => self.palette_run(false),
                TAB => self.palette_run(true),
                0x26 | 0x28 => {
                    let p = self.palette.as_mut().unwrap();
                    let n = p.items.len().max(1);
                    p.sel = if vk == 0x26 { (p.sel + n - 1) % n } else { (p.sel + 1) % n };
                }
                _ => {
                    if let Some(f) = self.field_mut() {
                        f.key(vk, ctrl, shift);
                    }
                    self.palette_fill();
                }
            }
            return true;
        }
        if self.pv.is_some() {
            match vk {
                ESC | SPACE => self.pv = None,
                0x25 => self.preview_step(-1),
                0x27 => self.preview_step(1),
                ENTER => self.preview_click(1),
                0x21 => self.preview_click(6),
                0x22 => self.preview_click(7),
                0x26 | 0x28 => {
                    if let Some(pv) = self.pv.as_mut() {
                        pv.scroll = (pv.scroll + if vk == 0x26 { -63.0 } else { 63.0 }).max(0.0);
                    }
                }
                _ => {}
            }
            return true;
        }
        if self.inline.is_some() {
            match vk {
                ESC => self.inline = None,
                ENTER => self.commit_inline(),
                _ => {
                    if ctrl && vk == 0x56 {
                        if let Some(s) = crate::dnd::clipboard_text(self.hwnd) {
                            self.inline.as_mut().unwrap().field.insert(&s);
                        }
                    } else {
                        self.inline.as_mut().unwrap().field.key(vk, ctrl, shift);
                    }
                }
            }
            return true;
        }
        if self.path_edit.is_some() {
            match vk {
                ESC => self.path_edit = None,
                ENTER => self.commit_path(),
                _ => {
                    if ctrl && vk == 0x56 {
                        if let Some(s) = crate::dnd::clipboard_text(self.hwnd) {
                            self.path_edit.as_mut().unwrap().insert(&s);
                        }
                    } else if ctrl && vk == 0x43 {
                        let s = self.path_edit.as_ref().unwrap().selected();
                        crate::dnd::set_clipboard_text(self.hwnd, &s);
                    } else {
                        self.path_edit.as_mut().unwrap().key(vk, ctrl, shift);
                    }
                }
            }
            return true;
        }
        if self.search_focus && self.pane().search.is_some() {
            match vk {
                ESC => {
                    self.end_search();
                    return true;
                }
                ENTER => {
                    if ctrl {
                        // containing folder, with the item selected
                        if let Some(it) = self.sel_items().first().cloned()
                            && let Some(parent) = it.path.parent()
                        {
                            self.go(Loc::Folder(parent.to_path_buf()));
                            self.pane_mut().want = Some(it.path);
                        }
                    } else {
                        self.act(Act::Open);
                    }
                    return true;
                }
                0x26 | 0x28 | 0x21 | 0x22 => {}
                _ => {
                    if ctrl && vk == 0x56 {
                        if let Some(s) = crate::dnd::clipboard_text(self.hwnd)
                            && let Some(se) = self.pane_mut().search.as_mut()
                        {
                            se.field.insert(&s);
                        }
                        self.timer(T_SEARCH, 30);
                        return true;
                    }
                    if !(ctrl && matches!(vk, 0x4B | 0x54 | 0x57 | 0x4C)) {
                        let used = self.pane_mut().search.as_mut().map(|s| s.field.key(vk, ctrl, shift)).unwrap_or(false);
                        if used {
                            self.timer(T_SEARCH, 30);
                            return true;
                        }
                    }
                }
            }
        }
        // window keys
        match (vk, ctrl, shift, alt) {
            (ESC, ..) => {
                if self.xfer_open {
                    self.xfer_open = false;
                } else if self.whatis.is_some() {
                    self.whatis = None;
                } else if self.pane().search.is_some() {
                    self.end_search();
                } else {
                    self.pane_mut().sel.clear();
                }
            }
            (0x4B, true, false, _) => self.palette_open(""),
            (VK_F, true, false, _) | (0x45, true, false, _) => self.search_mode(""),
            (0x4C, true, _, _) | (0x73, false, false, false) => self.edit_path(),
            (0x44, false, false, true) => self.edit_path(),
            (0x54, true, false, _) => {
                let loc = self.pane().loc.clone();
                self.new_tab(if loc == Loc::Settings { Loc::ThisPc } else { loc });
            }
            (0x57, true, false, _) => {
                let c = self.cur;
                self.close_tab(c);
            }
            (TAB, true, s, _) => {
                let n = self.tabs.len();
                self.cur = if s { (self.cur + n - 1) % n } else { (self.cur + 1) % n };
            }
            (k @ 0x31..=0x39, true, false, _) => {
                let i = (k - 0x31) as usize;
                if i < self.tabs.len() {
                    self.cur = if k == 0x39 { self.tabs.len() - 1 } else { i };
                }
            }
            (k @ 0x31..=0x33, true, true, _) => {
                self.tab_mut().view = [View::Details, View::List, View::Grid][(k - 0x31) as usize];
            }
            (0x4E, true, true, _) => self.act(Act::NewFolder),
            (0x4E, true, false, _) => {
                let exe = std::env::current_exe().unwrap_or_default();
                let arg = self.pane().loc.path().map(|p| p.display().to_string()).unwrap_or_default();
                let _ = std::process::Command::new(exe).arg(arg).arg("--new-window").spawn();
            }
            (0x71, false, false, false) => self.act(Act::Rename),
            (0x52, true, true, _) => self.act(Act::BatchRename),
            (0x43, true, true, _) => self.act(Act::CopyPath),
            (0x43, true, false, _) => self.act(Act::Copy),
            (0x58, true, false, _) => self.act(Act::Cut),
            (0x56, true, false, _) => self.act(Act::Paste),
            (0x5A, true, false, _) => self.undo(),
            (0x41, true, false, _) => self.act(Act::SelectAll),
            (0x2E, _, s, _) => self.act(if s { Act::DeleteForGood } else { Act::Delete }),
            (0x49, true, false, _) => self.act(Act::WhatIs),
            (0x72, false, false, false) => self.toggle_split(),
            (TAB, false, false, false) if self.tab().split() => {
                let tab = self.tab_mut();
                tab.active = 1 - tab.active;
            }
            (0x74, false, false, false) => self.act(Act::CopyOther),
            (0x75, false, false, false) => self.act(Act::MoveOther),
            (0x51, true, true, _) => self.xfer_open = !self.xfer_open,
            (ENTER, false, false, true) => self.act(Act::Properties),
            (0xBC, true, false, _) => self.open_settings(),
            (0xC0, true, false, _) => self.act(Act::Terminal),
            (ENTER, c, ..) => self.act(if c { Act::OpenNewTab } else { Act::Open }),
            (0x4F, true, false, _) => self.act(Act::Open),
            (SPACE, false, false, false) => self.preview_open(),
            (0x25, false, false, true) | (0x08, false, false, false) if vk == 0x25 => self.back(),
            (0x08, false, false, false) => self.up(),
            (0x27, false, false, true) => self.forward(),
            (0x26, false, false, true) => self.up(),
            (0x26, c, s, false) => self.move_focus(if self.tab().view == View::Grid { -2 } else { -1 }, s, c),
            (0x28, c, s, false) => self.move_focus(if self.tab().view == View::Grid { 2 } else { 1 }, s, c),
            (0x25, c, s, false) if self.tab().view == View::Grid => self.move_focus(-1, s, c),
            (0x27, c, s, false) if self.tab().view == View::Grid => self.move_focus(1, s, c),
            (0x24, c, s, false) => self.move_focus(100, s, c),
            (0x23, c, s, false) => self.move_focus(200, s, c),
            (0x21, c, s, false) => self.move_focus(300, s, c),
            (0x22, c, s, false) => self.move_focus(400, s, c),
            (0x74, true, ..) => self.act(Act::Refresh),
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------------ menus
    pub fn context_menu(&mut self, x: f32, y: f32, bg: bool) {
        self.menu_gen += 1;
        let seq = self.menu_gen;
        let sel = self.sel_items();
        let paths: Vec<PathBuf> = sel.iter().map(|i| i.path.clone()).collect();
        let recycle = self.pane().loc == Loc::RecycleBin;
        let mut items = vec![];
        if bg || paths.is_empty() {
            let mut v = MItem::new("View", 0xE8FD, Act::ViewMenu);
            v.sub = true;
            items.push(v);
            let mut s = MItem::new("Sort by", 0xE8CB, Act::SortMenu);
            s.sub = true;
            items.push(s);
            let mut p = MItem::new("Paste", 0xE77F, Act::Paste).keys("Ctrl+V").sep();
            p.enabled = !crate::dnd::clipboard_files().0.is_empty() && !recycle;
            items.push(p);
            let mut n = MItem::new("New", 0xE710, Act::NewMenu);
            n.sub = true;
            n.enabled = !recycle;
            items.push(n);
            items.push(MItem::new("Open in Terminal", 0xE756, Act::Terminal).keys("Ctrl+`").sep());
            items.push(MItem::new("Copy path", 0xE71B, Act::CopyPath));
            items.push(MItem::new("Pin to sidebar", 0xE718, Act::Pin));
            items.push(MItem::new("Properties", 0xE946, Act::Properties).keys("Alt+Enter").sep());
            self.menu = Some(CtxMenu { x, y, items, paths: vec![], bg: true, seq, loading: false, shell: vec![], at: Instant::now() });
            return;
        }
        if recycle {
            items.push(MItem::new("Restore", 0xE777, Act::Restore));
            items.push(MItem::new("Delete for good", 0xE74D, Act::DeleteForGood).keys("Delete"));
            self.menu = Some(CtxMenu { x, y, items, paths, bg: true, seq, loading: false, shell: vec![], at: Instant::now() });
            return;
        }
        let one = sel.len() == 1;
        let mut o = MItem::new("Open", 0xE8E5, Act::Open).keys("Enter");
        o.bold = true;
        items.push(o);
        if one && !sel[0].is_dir {
            let mut ow = MItem::new("Open with", 0xE7AC, Act::OpenWith);
            ow.sub = true;
            items.push(ow);
        }
        if sel.iter().any(|i| i.is_dir) {
            items.push(MItem::new("Open in new tab", 0xE8A7, Act::OpenNewTab).keys("Ctrl+Enter"));
        }
        items.push(MItem::new("Preview", 0xE7B3, Act::Preview).keys("Space"));
        if one && sel[0].is_archive() {
            let stem = sel[0].name.split('.').next().unwrap_or("").to_string();
            items.push(MItem::new(&format!("Extract to \u{201C}{stem}\\\u{201D}"), 0xE8B5, Act::ExtractTo));
            items.push(MItem::new("Extract here", 0xE8B5, Act::ExtractHere));
        }
        if one {
            items.push(MItem::new("What is this file?", 0xF4A5, Act::WhatIs).keys("Ctrl+I"));
        }
        items.push(MItem::new("Copy path", 0xE71B, Act::CopyPath).keys("Ctrl+Shift+C"));
        let mut ct = MItem::new("Copy to", 0xE8C8, Act::CopyToMenu).sep();
        ct.sub = true;
        items.push(ct);
        let mut mt = MItem::new("Move to", 0xE8DE, Act::MoveToMenu);
        mt.sub = true;
        items.push(mt);
        let mut p = MItem::new("Paste", 0xE77F, Act::Paste).keys("Ctrl+V");
        p.enabled = !crate::dnd::clipboard_files().0.is_empty();
        items.push(p);
        let mut n = MItem::new("New", 0xE710, Act::NewMenu);
        n.sub = true;
        items.push(n);
        if !(one && sel[0].is_archive()) {
            items.push(MItem::new("Compress to zip", 0xF012, Act::Zip));
        }
        if one && sel[0].is_dir {
            items.push(MItem::new("Pin to sidebar", 0xE718, Act::Pin));
        }
        items.push(MItem::new("Properties", 0xE946, Act::Properties).keys("Alt+Enter").sep());
        self.menu = Some(CtxMenu { x, y, items, paths: paths.clone(), bg: false, seq, loading: true, shell: vec![], at: Instant::now() });
        // the shell's own items (7-Zip, Terminal, Defender, Send to ...) arrive a moment later
        self.spawn(move || Msg::ShellMenu { seq, items: fmcore::menu::list(&paths, false).unwrap_or_default() });
    }

    fn open_sub(&mut self, i: usize) {
        let Some(m) = self.menu.as_ref() else { return };
        let it = m.items[i].clone();
        // where is the row?
        let r = self.hits.iter().find(|h| h.1 == Hit::MenuItem(i)).map(|h| h.0).unwrap_or_default();
        let (kind, items) = match it.act {
            Act::ViewMenu => (DropKind::View, self.drop_items(DropKind::View)),
            Act::SortMenu => (DropKind::Sort, self.drop_items(DropKind::Sort)),
            Act::NewMenu => (DropKind::New, self.drop_items(DropKind::New)),
            Act::CopyToMenu => (DropKind::CopyTo, self.drop_items(DropKind::CopyTo)),
            Act::MoveToMenu => (DropKind::MoveTo, self.drop_items(DropKind::MoveTo)),
            Act::OpenWith => {
                if let Some(p) = m.paths.first().cloned() {
                    if self.open_with.as_ref().map(|o| o.0 != p).unwrap_or(true) {
                        let ext = fmcore::util::ext_of(&p.to_string_lossy());
                        self.open_with = Some((p.clone(), fmcore::shell::open_with_list(&ext, false).unwrap_or_default()));
                    }
                }
                (DropKind::OpenWith, self.drop_items(DropKind::OpenWith))
            }
            _ => {
                // a shell submenu (7-Zip, Send to)
                let parent = it.shell.clone().unwrap_or_default();
                let kids: Vec<MItem> = m
                    .shell
                    .iter()
                    .filter(|s| s.parent == parent && !s.text.is_empty())
                    .map(|s| {
                        let mut x = MItem::new(&clean(&s.text), 0, Act::None);
                        x.shell = Some(if s.verb.is_empty() { s.text.clone() } else { s.verb.clone() });
                        x.enabled = s.enabled;
                        x
                    })
                    .collect();
                (DropKind::Shell, kids)
            }
        };
        if items.is_empty() {
            self.sub = None;
            return;
        }
        let width = if kind == DropKind::Shell { 260.0 } else { 240.0 };
        self.sub = Some(DropMenu { kind, anchor: r, items, width, at: Instant::now(), child: true });
    }

    fn drop_items(&self, kind: DropKind) -> Vec<MItem> {
        let mut v = vec![];
        match kind {
            DropKind::New => {
                v.push(MItem::new("Folder", 0xE8F4, Act::NewFolder).keys("Ctrl+Shift+N"));
                v.push(MItem::new("Text document", 0xE8A5, Act::NewText));
            }
            DropKind::View => {
                let view = self.tab().view;
                for (i, (l, g, vv)) in [("Details", 0xE8FDu32, View::Details), ("Compact list", 0xE8FD, View::List), ("Grid", 0xF0E2, View::Grid)].iter().enumerate() {
                    let mut m = MItem::new(l, *g, Act::None).keys(&format!("Ctrl+Shift+{}", i + 1));
                    m.value = i as i64;
                    m.checked = view == *vv;
                    m.keys = if m.checked { String::new() } else { m.keys };
                    v.push(m);
                }
                let mut s = MItem::new("Split view", 0xE8A9, Act::Split).keys("F3").sep();
                s.checked = self.tab().split();
                if s.checked {
                    s.keys.clear();
                }
                v.push(s);
                let mut h = MItem::new("Hidden files", 0xE7B3, Act::ToggleHidden);
                h.checked = self.ui.hidden;
                v.push(h);
            }
            DropKind::Sort => {
                let (col, desc) = self.pane().sort;
                for (i, (l, c)) in [("Name", SortCol::Name), ("Date modified", SortCol::Date), ("Size", SortCol::Size)].iter().enumerate() {
                    let mut m = MItem::new(l, 0, Act::None);
                    m.value = 10 + i as i64;
                    m.checked = col == *c;
                    v.push(m);
                }
                let mut d = MItem::new(if desc { "Descending" } else { "Ascending" }, 0, Act::None).sep();
                d.value = 20;
                v.push(d);
            }
            DropKind::FType | DropKind::FSize | DropKind::FDate => {
                let (list, cur): (&[(&str, &str)], usize) = match (kind, self.pane().search.as_ref()) {
                    (DropKind::FType, Some(s)) => (&TYPES[..], s.ftype),
                    (DropKind::FSize, Some(s)) => (&SIZES[..], s.fsize),
                    (_, Some(s)) => (&DATES[..], s.fdate),
                    _ => (&TYPES[..], 0),
                };
                for (i, (l, _)) in list.iter().enumerate() {
                    let mut m = MItem::new(l, 0, Act::None);
                    m.value = i as i64;
                    m.checked = i == cur;
                    v.push(m);
                }
            }
            DropKind::Theme => {
                for (i, l) in ["Same as Windows", "Light", "Dark"].iter().enumerate() {
                    let mut m = MItem::new(l, 0, Act::None);
                    m.value = i as i64;
                    m.checked = self.ui.theme as usize == i;
                    v.push(m);
                }
            }
            DropKind::Lang => {
                let lang = crate::ai::lang();
                for (i, (l, code)) in [("English", "en"), ("Persian (فارسی)", "fa")].iter().enumerate() {
                    let mut m = MItem::new(l, 0, Act::None);
                    m.value = i as i64;
                    m.checked = lang == *code;
                    v.push(m);
                }
            }
            DropKind::OpenWith => {
                if let Some((_, apps)) = &self.open_with {
                    for (i, a) in apps.iter().take(12).enumerate() {
                        let mut m = MItem::new(&a.ui_name, 0, Act::None);
                        m.value = i as i64;
                        v.push(m);
                    }
                }
                if v.is_empty() {
                    let mut m = MItem::new("No apps found", 0, Act::None);
                    m.enabled = false;
                    v.push(m);
                }
            }
            DropKind::Insert => {
                for (i, l) in ["{name}  current name", "{n}  counter", "{date}  date modified", "{ext}  extension"].iter().enumerate() {
                    let mut m = MItem::new(l, 0, Act::None);
                    m.value = i as i64;
                    v.push(m);
                }
            }
            DropKind::Style => {
                for (i, l) in STYLES.iter().enumerate() {
                    let mut m = MItem::new(l, 0, Act::None);
                    m.value = i as i64;
                    m.checked = self.rename.as_ref().map(|r| r.style == i).unwrap_or(false);
                    v.push(m);
                }
            }
            DropKind::Digits => {
                for d in 1..=4 {
                    let mut m = MItem::new(&d.to_string(), 0, Act::None);
                    m.value = d;
                    m.checked = self.rename.as_ref().map(|r| r.digits == d as usize).unwrap_or(false);
                    v.push(m);
                }
            }
            DropKind::CopyTo | DropKind::MoveTo => {
                let tab = self.tab();
                if tab.split()
                    && let Some(d) = tab.panes[1 - tab.active].dir()
                {
                    let mut m = MItem::new(&format!("Other pane: {}", folder_title(&d)), 0xE8A9, Act::None);
                    m.shell = Some(d.display().to_string());
                    m.keys = if kind == DropKind::CopyTo { "F5".into() } else { "F6".into() };
                    v.push(m);
                }
                for (name, _, _, loc) in &self.side {
                    if let Loc::Folder(p) = loc {
                        let mut m = MItem::new(name, 0xE8B7, Act::None);
                        m.shell = Some(p.display().to_string());
                        v.push(m);
                    }
                }
                for d in &self.drives {
                    let letter = d.root.trim_end_matches('\\');
                    let mut m = MItem::new(&if d.label.is_empty() { drive_label(letter) } else { format!("{} ({letter})", d.label) }, 0xEDA2, Act::None);
                    m.shell = Some(d.root.clone());
                    v.push(m);
                }
            }
            DropKind::More | DropKind::Shell => {}
        }
        v
    }

    pub fn open_drop(&mut self, kind: DropKind, anchor: Rect, width: f32) {
        let items = self.drop_items(kind);
        self.drop = Some(DropMenu { kind, anchor, items, width, at: Instant::now(), child: false });
    }

    fn menu_click(&mut self, i: usize) {
        let Some(m) = self.menu.as_ref() else { return };
        let Some(it) = m.items.get(i).cloned() else { return };
        if !it.enabled {
            return;
        }
        if it.sub {
            self.open_sub(i);
            return;
        }
        let paths = m.paths.clone();
        self.menu = None;
        self.sub = None;
        if let Some(verb) = it.shell {
            // the shell's own item: run it on the UI thread (it may show its own window)
            let _c = fmcore::util::com();
            if let Err(e) = fmcore::menu::invoke(&paths, &verb, false) {
                self.toast(&e, "", false);
            }
            self.timer(T_RELIST, 600);
            return;
        }
        self.act(it.act);
    }

    fn sub_click(&mut self, i: usize) {
        let Some(d) = self.sub.take() else { return };
        let Some(it) = d.items.get(i).cloned() else { return };
        let paths = self.menu.as_ref().map(|m| m.paths.clone()).unwrap_or_default();
        self.menu = None;
        self.apply_drop(d.kind, &it, paths);
    }

    fn drop_click(&mut self, i: usize) {
        let Some(d) = self.drop.take() else { return };
        let Some(it) = d.items.get(i).cloned() else { return };
        let paths = self.sel_paths();
        self.apply_drop(d.kind, &it, paths);
    }

    fn apply_drop(&mut self, kind: DropKind, it: &MItem, paths: Vec<PathBuf>) {
        if !it.enabled {
            return;
        }
        match kind {
            DropKind::New => self.act(it.act),
            DropKind::View => {
                if it.act == Act::None {
                    self.tab_mut().view = [View::Details, View::List, View::Grid][it.value as usize];
                } else {
                    self.act(it.act);
                }
            }
            DropKind::Sort => {
                let pane = self.pane_mut();
                if it.value == 20 {
                    pane.sort.1 = !pane.sort.1;
                } else {
                    let col = [SortCol::Name, SortCol::Date, SortCol::Size][(it.value - 10) as usize];
                    pane.sort = (col, pane.sort.1);
                }
                pane.sort_items();
            }
            DropKind::FType | DropKind::FSize | DropKind::FDate => {
                if let Some(s) = self.pane_mut().search.as_mut() {
                    match kind {
                        DropKind::FType => s.ftype = it.value as usize,
                        DropKind::FSize => s.fsize = it.value as usize,
                        _ => s.fdate = it.value as usize,
                    }
                }
                self.start_search();
            }
            DropKind::Theme => {
                self.ui.theme = it.value as u8;
                self.forced_dark = None;
                self.save_ui();
                self.apply_theme();
            }
            DropKind::Lang => {
                let _ = crate::ai::set_lang(if it.value == 1 { "fa" } else { "en" });
            }
            DropKind::OpenWith => {
                if let Some((p, apps)) = self.open_with.clone()
                    && let Some(a) = apps.get(it.value as usize)
                    && let Err(e) = fmcore::shell::open_with(&p, &a.name)
                {
                    self.toast(&e, "", false);
                }
            }
            DropKind::Insert => {
                let tok = ["{name}", "{n}", "{date}", "{ext}"][it.value as usize];
                if let Some(r) = self.rename.as_mut() {
                    r.pattern.insert(tok);
                    r.focus = 0;
                }
                rename_preview(self);
            }
            DropKind::Style => {
                if let Some(r) = self.rename.as_mut() {
                    r.style = it.value as usize;
                }
                self.suggest_names();
            }
            DropKind::Digits => {
                if let Some(r) = self.rename.as_mut() {
                    r.digits = it.value as usize;
                }
                rename_preview(self);
            }
            DropKind::CopyTo | DropKind::MoveTo => {
                if let Some(dest) = &it.shell
                    && !paths.is_empty()
                {
                    jobs::add(&self.jobs, if kind == DropKind::MoveTo { jobs::Kind::Move } else { jobs::Kind::Copy }, paths, PathBuf::from(dest));
                }
            }
            DropKind::Shell => {
                if let Some(verb) = &it.shell {
                    let _c = fmcore::util::com();
                    if let Err(e) = fmcore::menu::invoke(&paths, verb, false) {
                        self.toast(&e, "", false);
                    }
                }
            }
            DropKind::More => {}
        }
    }

    // ------------------------------------------------------------------ preview
    pub fn preview_open(&mut self) {
        if self.pv.is_some() {
            self.pv = None;
            return;
        }
        let pane = self.pane();
        let idx = pane.focus.filter(|f| pane.rows().get(*f).map(|r| pane.sel.contains(&r.path)).unwrap_or(false)).or_else(|| pane.rows().iter().position(|r| pane.sel.contains(&r.path)));
        let Some(i) = idx else { return };
        let (t, p) = (self.cur, self.tab().active);
        let g = pane_geoms(self, t)[p];
        let (y, h, _) = row_geom(self, t, p, i);
        let from = Rect::new(g.list.x + 6.0, g.list.y + y - self.pane().scroll, g.list.w - 12.0, h);
        self.preview_at(i, from);
    }

    fn preview_at(&mut self, i: usize, from: Rect) {
        let Some(it) = self.pane().rows().get(i).cloned() else { return };
        let at = self.pv.as_ref().map(|p| p.at).unwrap_or_else(Instant::now);
        self.pv = Some(Pv { idx: i, path: it.path.clone(), data: None, scroll: 0.0, at, from });
        let (p, d) = (it.path.clone(), it.is_dir);
        self.spawn(move || Msg::Preview { data: crate::preview::load(&p, d) });
        self.anim();
    }

    fn preview_step(&mut self, d: i32) {
        let Some(pv) = self.pv.as_ref() else { return };
        let n = self.pane().rows().len() as i32;
        if n == 0 {
            return;
        }
        let i = (pv.idx as i32 + d).rem_euclid(n) as usize;
        let from = pv.from;
        let p = self.tab().active;
        self.select_row(p, i, false, false);
        self.preview_at(i, from);
    }

    fn preview_click(&mut self, k: u8) {
        match k {
            0 | 9 => self.pv = None,
            1 => {
                if let Some(pv) = self.pv.take() {
                    let it = self.pane().rows().get(pv.idx).cloned();
                    if let Some(it) = it {
                        self.open_item(&it, false);
                    }
                }
            }
            2 => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Pv(2)).map(|h| h.0).unwrap_or_default();
                if let Some(pv) = &self.pv {
                    let p = pv.path.clone();
                    let ext = fmcore::util::ext_of(&p.to_string_lossy());
                    self.open_with = Some((p, fmcore::shell::open_with_list(&ext, false).unwrap_or_default()));
                    self.open_drop(DropKind::OpenWith, r, 240.0);
                }
            }
            3 => {
                if let Some(d) = self.pv.as_ref().and_then(|p| p.data.as_ref()) {
                    let s: String = d.lines.iter().map(|l| l.iter().map(|x| x.0.as_str()).collect::<String>()).collect::<Vec<_>>().join("\r\n");
                    crate::dnd::set_clipboard_text(self.hwnd, &s);
                    self.toast("Copied", "", false);
                }
            }
            4 => self.preview_step(-1),
            5 => self.preview_step(1),
            6 | 7 => {
                let Some(d) = self.pv.as_ref().and_then(|p| p.data.as_ref()) else { return };
                let (pages, page) = (d.pages, d.page);
                let next = if k == 6 { page.saturating_sub(1) } else { (page + 1).min(pages.saturating_sub(1)) };
                if next != page {
                    let p = d.path.clone();
                    self.spawn(move || Msg::Preview { data: crate::preview::load_page(&p, next) });
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ what is this file?
    pub fn whatis_open(&mut self) {
        let sel = self.sel_items();
        let Some(it) = sel.first().cloned() else { return };
        if self.tab().split() {
            return;
        }
        let facts = crate::known::facts(&it.path, it.is_dir);
        let known = crate::known::describe(&it.name, it.is_dir);
        let answer = match known {
            Some((desc, _)) => Some(Ok(crate::ai::FileAnswer { what: desc, actions: vec![], lang: "en".into() })),
            None if self.ai_on => None,
            None => Some(Err("Unknown file type. The facts below come from the file itself.".to_string())),
        };
        let ask_ai = answer.is_none();
        self.whatis = Some(WhatIs { item: it.clone(), answer, facts });
        if ask_ai {
            let p = it.path.clone();
            self.spawn(move || Msg::WhatIs { path: p.clone(), res: crate::ai::explain_file(&p) });
        }
    }

    // ------------------------------------------------------------------ batch rename
    pub fn rename_open(&mut self) {
        let items = self.sel_items();
        if items.is_empty() {
            return;
        }
        let mut pat = Field::new("{name}");
        pat.select_all();
        self.rename = Some(Rename {
            mode: 0,
            pattern: pat,
            find: Field::default(),
            repl: Field::default(),
            start: Field::new("1"),
            digits: 2,
            focus: 0,
            items,
            rows: vec![],
            problem: None,
            ai: None,
            ai_busy: false,
            ai_err: None,
            style: 0,
            seq: 0,
            at: Instant::now(),
        });
        rename_preview(self);
    }

    fn suggest_names(&mut self) {
        let Some(r) = self.rename.as_mut() else { return };
        r.mode = 3;
        r.ai_busy = true;
        r.ai = None;
        r.seq += 1;
        let seq = r.seq;
        let paths: Vec<PathBuf> = r.items.iter().map(|i| i.path.clone()).collect();
        let style = STYLES[r.style].to_string();
        rename_preview(self);
        self.spawn(move || Msg::Names { seq, res: crate::ai::suggest_names(&paths, &style) });
    }

    fn rename_apply(&mut self) {
        let Some(r) = self.rename.as_ref() else { return };
        if r.problem.is_some() {
            return;
        }
        let pairs: Vec<(PathBuf, PathBuf)> = r.items.iter().zip(r.rows.iter()).filter(|(_, row)| row.0 != row.1).map(|(it, row)| (it.path.clone(), it.path.with_file_name(&row.1))).collect();
        if pairs.is_empty() {
            return;
        }
        let n = pairs.len();
        let plan = fmcore::ops::Plan { items: pairs, unchanged: 0, problems: vec![] };
        match fmcore::ops::batch_apply(&plan) {
            Ok(_) => {
                self.rename = None;
                self.toast(&format!("Renamed {n} item{}", if n == 1 { "" } else { "s" }), "", true);
                self.refresh_all();
            }
            Err(e) => {
                if let Some(r) = self.rename.as_mut() {
                    r.problem = Some(e);
                }
            }
        }
    }

    fn dialog_click(&mut self, k: u8, x: f32) {
        match k {
            0 => self.rename = None,
            1 => self.rename_apply(),
            10..=12 => {
                if let Some(r) = self.rename.as_mut() {
                    r.mode = k - 10;
                    r.focus = 0;
                }
                rename_preview(self);
            }
            13 => self.suggest_names(),
            20 | 22 => {
                let r = self.rename.as_mut().unwrap();
                r.focus = if k == 20 { 0 } else { 1 };
                let _ = x;
            }
            21 => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Dlg(21)).map(|h| h.0).unwrap_or_default();
                self.open_drop(DropKind::Insert, r, 230.0);
            }
            23 => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Dlg(23)).map(|h| h.0).unwrap_or_default();
                self.open_drop(DropKind::Digits, r, 96.0);
            }
            24 => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Dlg(24)).map(|h| h.0).unwrap_or_default();
                self.open_drop(DropKind::Style, r, r.w.max(260.0));
            }
            25 => self.suggest_names(),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ settings
    fn settings_click(&mut self, k: u8) {
        use crate::models::Section;
        match k {
            // the user's own click is the only thing that ever changes the default file manager
            1 => {
                let exe = std::env::current_exe().unwrap_or_default();
                match fmcore::defaultfm::set(&exe) {
                    Ok(()) => self.toast("Folders and Win+E now open in Win Tools Files", "", false),
                    Err(e) => self.toast(&e, "", false),
                }
            }
            2 => match fmcore::defaultfm::restore() {
                Ok(()) => self.toast("Explorer opens folders again", "", false),
                Err(e) => self.toast(&e, "", false),
            },
            3 => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Set(3)).map(|h| h.0).unwrap_or_default();
                self.open_drop(DropKind::Theme, r, r.w);
            }
            4 => {
                self.ui.sizes = !self.ui.sizes;
                self.save_ui();
            }
            5 => {
                self.ui.ext = !self.ui.ext;
                self.save_ui();
            }
            6 => {
                self.ui.hidden = !self.ui.hidden;
                self.save_ui();
                self.refresh_all();
            }
            7 => {
                self.ui.ask_delete = !self.ui.ask_delete;
                self.save_ui();
            }
            8 => {
                let on = !crate::models::switch_on(Section::Semsearch);
                let _ = crate::models::set_switch(Section::Semsearch, on && crate::models::semsearch_detect().is_some());
                self.refresh_ai_flags();
            }
            9 => {
                let on = !(crate::ai::switch_on() && crate::ai::has_key());
                match crate::ai::set_enabled(on) {
                    Ok(()) => {
                        self.set.msg = None;
                        self.set.msg_ok = false;
                    }
                    Err(e) => {
                        self.set.msg = Some(e);
                        self.set.msg_ok = false;
                    }
                }
                self.refresh_ai_flags();
            }
            11 => {
                let r = self.hits.iter().find(|h| h.1 == Hit::Set(11)).map(|h| h.0).unwrap_or_default();
                self.open_drop(DropKind::Lang, r, 200.0);
            }
            // "Get a free key": opens the page in the browser (only on the user's click)
            14 => {
                let url = fmcore::util::wide(crate::ai::GET_KEY_URL);
                unsafe {
                    windows::Win32::UI::Shell::ShellExecuteW(None, windows::core::w!("open"), windows::core::PCWSTR(url.as_ptr()), None, None, SW_SHOWNORMAL);
                }
            }
            15 => self.key_focus = true,
            16 => self.save_key(),
            17 => {
                let typed = self.key_field.text.trim().to_string();
                self.set.msg = Some("Testing...".into());
                self.set.msg_ok = false;
                self.spawn(move || Msg::KeyTest(crate::ai::test_key(if typed.is_empty() { None } else { Some(typed.as_str()) })));
            }
            _ => {}
        }
        self.invalidate();
    }

    fn save_key(&mut self) {
        let k = std::mem::take(&mut self.key_field);
        if k.text.trim().is_empty() {
            return;
        }
        match crate::ai::set_key(&k.text) {
            Ok(()) => {
                self.set.msg = Some("Key saved (encrypted for this Windows user)".into());
                self.set.msg_ok = true;
                self.key_focus = false;
            }
            Err(e) => {
                self.set.msg = Some(e);
                self.set.msg_ok = false;
                self.key_field = k;
            }
        }
        self.refresh_ai_flags();
    }

    pub fn inline_rename_pub(&mut self, p: PathBuf) {
        self.inline_rename(p);
    }

    // ------------------------------------------------------------------ palette
    pub fn palette_open(&mut self, text: &str) {
        self.palette = Some(Palette { field: Field::new(text), sel: 0, items: vec![], at: Instant::now() });
        self.palette_fill();
    }

    fn palette_fill(&mut self) {
        let Some(p) = self.palette.as_ref() else { return };
        let q = p.field.text.trim().to_lowercase();
        let mut items: Vec<(i32, PItem)> = vec![];
        // folders: sidebar, other tabs, Everything folder hits
        let mut seen = std::collections::HashSet::new();
        let mut folders: Vec<(String, PathBuf)> = vec![];
        for (name, _, _, loc) in &self.side {
            if let Loc::Folder(p) = loc {
                folders.push((name.clone(), p.clone()));
            }
        }
        for t in &self.tabs {
            for pn in &t.panes {
                if let Loc::Folder(p) = &pn.loc {
                    folders.push((folder_title(p), p.clone()));
                    for b in pn.back.iter().rev().take(5) {
                        if let Loc::Folder(p) = b {
                            folders.push((folder_title(p), p.clone()));
                        }
                    }
                }
            }
        }
        // Everything folder hits: names that start with the text, short (top-level) paths first, no system noise
        if q.len() >= 2
            && let Ok(r) = fmcore::evipc::query(&format!("folder: startwith:{q} parents:<=4"), 300, fmcore::evipc::REQ_NAME | fmcore::evipc::REQ_PATH, 1, false, Duration::from_millis(300))
        {
            let noise = ["$recycle.bin", r"\appdata\", r"\windows\", r"\program files", r"\programdata\", "node_modules", r"\.", r"\target\", r"\site-packages\", r"\build\"];
            let mut hits: Vec<(String, String)> = r.items.into_iter().map(|i| (i.name.clone(), i.full())).filter(|(_, f)| {
                let l = f.to_lowercase();
                !noise.iter().any(|n| l.contains(n))
            }).collect();
            hits.sort_by_key(|h| h.1.matches('\\').count() * 1000 + h.1.len());
            for (name, full) in hits.into_iter().take(4) {
                folders.push((name, PathBuf::from(full)));
            }
        }
        for (name, path) in folders {
            let key = path.to_string_lossy().to_lowercase();
            if !seen.insert(key) {
                continue;
            }
            if let Some((score, hl)) = fuzzy(&name, &q) {
                items.push((score, PItem { label: name, sub: path.parent().map(|x| x.display().to_string()).unwrap_or_default(), folder: true, glyph: 0, act: Act::None, loc: Some(Loc::Folder(path)), keys: vec![], hl }));
            }
        }
        items.sort_by(|a, b| b.0.cmp(&a.0));
        items.truncate(5);
        let sel_name = self.sel_items().first().map(|i| i.name.clone()).unwrap_or_default();
        let acts: Vec<(&str, u32, Act, Vec<&'static str>, String)> = vec![
            ("Copy path", 0xE8C8, Act::CopyPath, vec!["Ctrl", "Shift", "C"], sel_name.clone()),
            ("Compress to zip", 0xF012, Act::Zip, vec![], String::new()),
            ("Copy to other pane", 0xE8DE, Act::CopyOther, vec!["F5"], String::new()),
            ("Move to other pane", 0xE8DE, Act::MoveOther, vec!["F6"], String::new()),
            ("Open in Terminal", 0xE756, Act::Terminal, vec!["Ctrl", "`"], String::new()),
            ("Batch rename", 0xE8AC, Act::BatchRename, vec!["Ctrl", "Shift", "R"], String::new()),
            ("New folder", 0xE8F4, Act::NewFolder, vec!["Ctrl", "Shift", "N"], String::new()),
            ("Split view", 0xE8A9, Act::Split, vec!["F3"], String::new()),
            ("Transfers", 0xE8C8, Act::Transfers, vec!["Ctrl", "Shift", "Q"], String::new()),
            ("What is this file?", 0xE946, Act::WhatIs, vec!["Ctrl", "I"], sel_name.clone()),
            ("Show hidden files", 0xE7B3, Act::ToggleHidden, vec![], String::new()),
            ("Undo", 0xE7A7, Act::Undo, vec!["Ctrl", "Z"], String::new()),
            ("Settings", 0xE713, Act::Settings, vec!["Ctrl", ","], String::new()),
            ("Pin to sidebar", 0xE718, Act::Pin, vec![], String::new()),
            ("Properties", 0xE946, Act::Properties, vec!["Alt", "Enter"], sel_name),
        ];
        let mut a2: Vec<(i32, PItem)> = vec![];
        for (label, g, act, keys, sub) in acts {
            if let Some((score, hl)) = fuzzy(label, &q) {
                a2.push((score, PItem { label: label.into(), sub, folder: false, glyph: g, act, loc: None, keys, hl }));
            }
        }
        a2.sort_by(|a, b| b.0.cmp(&a.0));
        a2.truncate(if q.is_empty() { 6 } else { 5 });
        let p = self.palette.as_mut().unwrap();
        p.items = items.into_iter().chain(a2).map(|x| x.1).collect();
        p.sel = p.sel.min(p.items.len().saturating_sub(1));
    }

    fn palette_run(&mut self, new_tab: bool) {
        let Some(p) = self.palette.take() else { return };
        let Some(it) = p.items.into_iter().nth(p.sel) else { return };
        if let Some(loc) = it.loc {
            if new_tab {
                self.new_tab(loc);
            } else {
                self.go(loc);
            }
        } else {
            self.act(it.act);
        }
    }

    // ------------------------------------------------------------------ confirm box
    pub fn paint_confirm(&mut self, t: &crate::theme::Theme) {
        let Some((msg, act)) = self.confirm.clone() else { return };
        use crate::gfx::{Align, Font};
        self.gfx.fill(Rect::new(0.0, 0.0, self.w, self.h), t.scrim);
        self.hit(Rect::new(0.0, 0.0, self.w, self.h), Hit::Block);
        let w = 420.0;
        let r = Rect::new((self.w - w) / 2.0, (self.h - 170.0) / 2.0, w, 170.0);
        self.card(r, 10.0, t, true);
        self.hit(r, Hit::Block);
        let title = if act == Act::DeleteForGood { "Delete for good" } else { "Delete" };
        self.gfx.text(title, Rect::new(r.x + 24.0, r.y + 20.0, w - 48.0, 28.0), Font::Display, 20.0, 600, Align::Left, t.text);
        self.gfx.para(&msg, Rect::new(r.x + 24.0, r.y + 58.0, w - 48.0, 44.0), Font::Ui, 13.0, 400, Align::Left, t.text2, 20.0, false, 2);
        let bw = 100.0;
        self.button(Rect::new(r.r() - 24.0 - bw, r.b() - 24.0 - 32.0, bw, 32.0), "Delete", 0, crate::paint::Btn::Accent, Hit::Dlg(41), t);
        self.button(Rect::new(r.r() - 24.0 - 2.0 * bw - 8.0, r.b() - 24.0 - 32.0, bw, 32.0), "Cancel", 0, crate::paint::Btn::Std, Hit::Dlg(40), t);
    }
}

fn clean(s: &str) -> String {
    let s = s.replace("&&", "\u{1}").replace('&', "").replace('\u{1}', "&");
    s.split('\t').next().unwrap_or("").trim().to_string()
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out += &rest[..i];
        let after = &rest[i + 1..];
        if let Some(j) = after.find('%') {
            let name = &after[..j];
            out += &std::env::var(name).unwrap_or_else(|_| format!("%{name}%"));
            rest = &after[j + 1..];
        } else {
            out += &rest[i..];
            rest = "";
        }
    }
    out + rest
}

/// Subsequence match: (score, matched char indices). Empty query matches everything.
pub fn fuzzy(label: &str, q: &str) -> Option<(i32, Vec<usize>)> {
    if q.is_empty() {
        return Some((0, vec![]));
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let qc: Vec<char> = q.chars().filter(|c| !c.is_whitespace()).collect();
    let mut hl = vec![];
    let mut j = 0;
    let mut score = 0;
    let mut prev: Option<usize> = None;
    for (i, c) in l.iter().enumerate() {
        if j < qc.len() && *c == qc[j] {
            hl.push(i);
            score += if prev == Some(i.wrapping_sub(1)) { 5 } else { 1 };
            if i == 0 || matches!(l[i - 1], ' ' | '\\' | '-' | '_' | '.') {
                score += 3;
            }
            prev = Some(i);
            j += 1;
        }
    }
    (j == qc.len()).then_some((score - l.len() as i32 / 8, hl))
}

/// Shell menu items go in before "Properties"; ours are not repeated.
pub fn merge_shell_items(m: &mut CtxMenu) {
    const SKIP_VERBS: &[&str] = &["open", "opennew", "cut", "copy", "paste", "delete", "rename", "properties", "link", "pintohome", "openas", "edit", "print", "find", "explore", "opencontaining", "copyaspath", "pintostartscreen", "windows.share", "share"];
    const SKIP_TEXT: &[&str] = &["extract all...", "extract all", "open", "cut", "copy", "paste", "delete", "rename", "properties", "create shortcut", "open with", "pin to quick access", "copy as path", "open in new window", "open in new tab", "share", "print", "edit", "pin to start", "open file location"];
    let mut extra = vec![];
    for s in m.shell.iter().filter(|s| s.parent.is_empty()) {
        let text = clean(&s.text);
        if text.is_empty() || SKIP_VERBS.contains(&s.verb.to_lowercase().as_str()) || SKIP_TEXT.contains(&text.to_lowercase().as_str()) {
            continue;
        }
        let has_kids = m.shell.iter().any(|k| k.parent == s.text);
        let mut it = MItem::new(&text, 0, Act::None);
        it.shell = Some(if has_kids { s.text.clone() } else if s.verb.is_empty() { s.text.clone() } else { s.verb.clone() });
        it.sub = has_kids;
        it.enabled = s.enabled;
        extra.push(it);
    }
    if extra.is_empty() {
        return;
    }
    extra[0].sep = true;
    let at = m.items.iter().position(|i| i.act == Act::Properties).unwrap_or(m.items.len());
    for (k, it) in extra.into_iter().enumerate() {
        m.items.insert(at + k, it);
    }
}

pub fn show_open_with(a: &mut App) {
    let r = Rect::new(a.mx, a.my, 1.0, 1.0);
    a.open_drop(DropKind::OpenWith, r, 260.0);
}

fn case_style(words: &[String], style: usize) -> String {
    match style {
        0 => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-"),
        1 => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_"),
        2 => words.iter().map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
        }).collect::<Vec<_>>().join(" "),
        _ => words.iter().enumerate().map(|(i, w)| {
            let lw = w.to_lowercase();
            if i == 0 { lw } else { let mut c = lw.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() }
        }).collect(),
    }
}

/// Live preview of the batch rename: rows (old, new, changed range) and the clash check.
pub fn rename_preview(a: &mut App) {
    let Some(r) = a.rename.as_mut() else { return };
    let start: usize = r.start.text.parse().unwrap_or(1);
    let mut rows = vec![];
    for (k, it) in r.items.iter().enumerate() {
        let (stem, ext) = match it.name.rfind('.') {
            Some(i) if i > 0 && !it.is_dir => (it.name[..i].to_string(), it.name[i + 1..].to_string()),
            _ => (it.name.clone(), String::new()),
        };
        let n = format!("{:0width$}", start + k, width = r.digits);
        let date = crate::fmt::date_ymd(it.modified);
        let new_stem = match r.mode {
            0 => {
                let p = if r.pattern.text.is_empty() { "{name}".to_string() } else { r.pattern.text.clone() };
                p.replace("{name}", &stem).replace("{n}", &n).replace("{date}", &date).replace("{ext}", &ext)
            }
            1 => {
                if r.find.text.is_empty() {
                    stem.clone()
                } else {
                    let lower = stem.to_lowercase();
                    let f = r.find.text.to_lowercase();
                    let mut out = String::new();
                    let mut i = 0;
                    while let Some(j) = lower[i..].find(&f) {
                        out += &stem[i..i + j];
                        out += &r.repl.text;
                        i += j + f.len();
                    }
                    out + &stem[i..]
                }
            }
            2 => {
                let p = if r.pattern.text.is_empty() || r.pattern.text == "{name}" { "{n}".to_string() } else { r.pattern.text.clone() };
                p.replace("{name}", &stem).replace("{n}", &n).replace("{date}", &date).replace("{ext}", &ext)
            }
            _ => match &r.ai {
                Some(names) if k < names.len() => {
                    let words: Vec<String> = names[k].split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect();
                    case_style(&words, r.style)
                }
                _ => {
                    rows.clear();
                    break;
                }
            },
        };
        let had_ext_tok = r.mode == 0 && r.pattern.text.contains("{ext}");
        let new = if ext.is_empty() || had_ext_tok { new_stem.clone() } else { format!("{new_stem}.{ext}") };
        // the changed part, for the accent colour
        let common = stem.chars().zip(new_stem.chars()).take_while(|(a, b)| a == b).count();
        let s16 = new_stem.chars().take(common).map(|c| c.len_utf16()).sum::<usize>() as u32;
        let l16 = new_stem.chars().skip(common).map(|c| c.len_utf16()).sum::<usize>() as u32;
        rows.push((it.name.clone(), new, (l16 > 0).then_some((s16, l16))));
    }
    // clash check: duplicates in the batch, bad names, names taken by files outside the batch
    let mut problem = None;
    let mut seen = std::collections::HashSet::new();
    let batch: std::collections::HashSet<String> = r.items.iter().map(|i| i.name.to_lowercase()).collect();
    for ((_, new, _), it) in rows.iter().zip(r.items.iter()) {
        if let Err(e) = fmcore::ops::check_name(new) {
            problem = Some(e);
            break;
        }
        if !seen.insert(new.to_lowercase()) {
            problem = Some(format!("Two items would be called \u{201C}{new}\u{201D}"));
            break;
        }
        let target = it.path.with_file_name(new);
        if !batch.contains(&new.to_lowercase()) && target.exists() {
            problem = Some(format!("\u{201C}{new}\u{201D} already exists"));
            break;
        }
    }
    r.rows = rows;
    r.problem = problem;
}

/// "By meaning" through semsearch: its local server when it runs, else its command line (hidden).
pub fn meaning_search(words: &str, top: usize) -> Result<(u32, Vec<Item>), String> {
    let cmd = crate::models::semsearch_detect().ok_or("meaning search is not installed")?;
    let enc: String = words.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let body = match crate::ai_http_get(&format!("http://127.0.0.1:8791/search?q={enc}&top={top}")) {
        Ok(b) => b,
        Err(_) => {
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new("cmd.exe").arg("/c").arg(&cmd).arg(words).arg("--json").arg("--top").arg(top.to_string()).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    };
    let v: serde_json::Value = serde_json::from_str(body.trim()).map_err(|_| "meaning search gave no answer".to_string())?;
    let arr = v.as_array().cloned().or_else(|| v["results"].as_array().cloned()).unwrap_or_default();
    let items: Vec<Item> = arr
        .iter()
        .filter_map(|x| {
            let path = PathBuf::from(x["path"].as_str()?);
            let md = std::fs::metadata(&path).ok();
            let name = path.file_name()?.to_string_lossy().into_owned();
            let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            Some(Item {
                kind: crate::ficons::kind_of(&name, is_dir),
                hl: vec![],
                size: md.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
                modified: md.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| fmcore::util::unix_ft(d.as_secs() as i64)).unwrap_or(0),
                attrs: 0,
                folder: path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                is_dir,
                name,
                path,
                recycled: None,
            })
        })
        .collect();
    Ok((items.len() as u32, items))
}

// ---------------------------------------------------------------------- drag and drop into the window
impl crate::dnd::Sink for App {
    fn drag_enter(&mut self, paths: Vec<PathBuf>, x: i32, y: i32, keys: u32) -> u32 {
        self.drag_in = Some(DragIn { paths, x: 0.0, y: 0.0, target: None, effect: 0 });
        self.drag_over(x, y, keys)
    }
    fn drag_over(&mut self, x: i32, y: i32, keys: u32) -> u32 {
        let mut pt = POINT { x, y };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut pt);
        }
        let s = self.scale();
        let (fx, fy) = (pt.x as f32 / s, pt.y as f32 / s);
        let h = self.hit_at(fx, fy);
        let mut target: Option<PathBuf> = None;
        let mut dest: Option<PathBuf> = None;
        match h {
            Hit::Row(p, i) => {
                let it = self.tab().panes[p].rows().get(i).cloned();
                if let Some(it) = it.filter(|i| i.is_dir) {
                    target = Some(it.path.clone());
                    dest = Some(it.path);
                } else {
                    dest = self.tab().panes[p].dir();
                }
            }
            Hit::ListBg(p) | Hit::PaneHead(p) => dest = self.tab().panes[p].dir(),
            Hit::Side(i) if i < self.side.len() => {
                if let Loc::Folder(p) = &self.side[i].3 {
                    dest = Some(p.clone());
                    target = Some(p.clone());
                }
            }
            _ => {}
        }
        let Some(d) = self.drag_in.as_mut() else { return 0 };
        d.x = fx;
        d.y = fy;
        d.target = target;
        let ok = dest.as_ref().map(|dd| !d.paths.iter().any(|p| fmcore::util::path_eq(p, dd) || p.parent().map(|pp| fmcore::util::path_eq(pp, dd)).unwrap_or(false) && keys & 0x8 == 0)).unwrap_or(false);
        // Explorer's rule: Shift = move, Ctrl = copy, else move on the same drive and copy across drives
        d.effect = if !ok {
            0
        } else if keys & 0x4 != 0 {
            2
        } else if keys & 0x8 != 0 {
            1
        } else if d.paths.first().zip(dest.as_ref()).map(|(a, b)| fmcore::util::same_volume(a, b)).unwrap_or(false) {
            2
        } else {
            1
        };
        let e = d.effect;
        self.invalidate();
        e
    }
    fn drag_leave(&mut self) {
        self.drag_in = None;
        self.invalidate();
    }
    fn drop_files(&mut self, x: i32, y: i32, keys: u32) -> u32 {
        let e = self.drag_over(x, y, keys);
        let Some(d) = self.drag_in.take() else { return 0 };
        if e == 0 {
            return 0;
        }
        let dest = d.target.clone().or_else(|| {
            let h = self.hit_at(d.x, d.y);
            match h {
                Hit::Row(p, _) | Hit::ListBg(p) | Hit::PaneHead(p) => self.tab().panes[p].dir(),
                _ => self.pane().dir(),
            }
        });
        if let Some(dest) = dest {
            jobs::add(&self.jobs, if e == 2 { jobs::Kind::Move } else { jobs::Kind::Copy }, d.paths, dest);
        }
        self.invalidate();
        // we did the move ourselves through the queue: tell the source it was a copy so it deletes nothing
        if e == 2 && !self.dragging_out { 1 } else { e }
    }
}

// ---------------------------------------------------------------------- `wfm gui ...`
fn parse_keys(s: &str) -> Option<(u16, bool, bool, bool)> {
    let (mut c, mut sh, mut alt) = (false, false, false);
    let mut vk = 0u16;
    for part in s.to_lowercase().split('+') {
        match part {
            "ctrl" | "control" => c = true,
            "shift" => sh = true,
            "alt" => alt = true,
            "enter" | "return" => vk = 0x0D,
            "esc" | "escape" => vk = 0x1B,
            "space" => vk = 0x20,
            "tab" => vk = 0x09,
            "backspace" => vk = 0x08,
            "delete" | "del" => vk = 0x2E,
            "left" => vk = 0x25,
            "up" => vk = 0x26,
            "right" => vk = 0x27,
            "down" => vk = 0x28,
            "home" => vk = 0x24,
            "end" => vk = 0x23,
            "pageup" => vk = 0x21,
            "pagedown" => vk = 0x22,
            "`" => vk = 0xC0,
            "," => vk = 0xBC,
            f if f.starts_with('f') && f.len() > 1 && f[1..].parse::<u16>().is_ok() => vk = 0x6F + f[1..].parse::<u16>().unwrap(),
            k if k.len() == 1 => vk = k.chars().next().unwrap().to_ascii_uppercase() as u16,
            _ => return None,
        }
    }
    (vk != 0).then_some((vk, c, sh, alt))
}

pub fn ipc(a: &mut App, line: &str) -> String {
    let line = line.trim();
    let (cmd, arg) = line.split_once(' ').map(|(c, r)| (c, r.trim())).unwrap_or((line, ""));
    let ok = || json!({"ok": true}).to_string();
    let err = |e: &str| json!({"ok": false, "error": e}).to_string();
    let num = |s: &str| s.parse::<f32>().ok();
    let r = match cmd {
        "state" => return a.state_json().to_string(),
        "open" | "go" => {
            let p = PathBuf::from(arg.trim_matches('"'));
            if arg.eq_ignore_ascii_case("this pc") {
                a.go(Loc::ThisPc);
            } else if arg.eq_ignore_ascii_case("recycle bin") {
                a.go(Loc::RecycleBin);
            } else if p.is_dir() {
                a.go(Loc::Folder(fmcore::util::abs(&p)));
            } else if p.is_file() {
                a.go(Loc::Folder(p.parent().unwrap().to_path_buf()));
                a.pane_mut().want = Some(p);
            } else {
                return err("not found");
            }
            ok()
        }
        "search" => {
            a.end_search();
            a.search_mode(arg);
            a.start_search();
            ok()
        }
        "scope" => {
            if let Some(s) = a.pane_mut().search.as_mut() {
                s.pc = arg == "pc";
            }
            a.start_search();
            ok()
        }
        "filter" => {
            // filter type|size|date N
            let mut it = arg.split_whitespace();
            let (k, v) = (it.next().unwrap_or(""), it.next().and_then(|v| v.parse::<usize>().ok()).unwrap_or(0));
            if let Some(s) = a.pane_mut().search.as_mut() {
                match k {
                    "type" => s.ftype = v.min(TYPES.len() - 1),
                    "size" => s.fsize = v.min(SIZES.len() - 1),
                    _ => s.fdate = v.min(DATES.len() - 1),
                }
            }
            a.start_search();
            ok()
        }
        "select" => {
            let names: Vec<String> = arg.split('|').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
            let p = a.tab().active;
            let pane = a.pane_mut();
            pane.sel.clear();
            let mut first = None;
            for (i, r) in pane.rows().clone().iter().enumerate() {
                if names.contains(&r.name.to_lowercase()) || names.iter().any(|n| n.parse::<usize>().ok() == Some(i)) {
                    pane.sel.insert(r.path.clone());
                    first.get_or_insert(i);
                }
            }
            if let Some(i) = first {
                pane.focus = Some(i);
                pane.anchor = Some(i);
                let t = a.cur;
                a.scroll_to(t, p, i);
            }
            json!({"ok": true, "selected": a.pane().sel.len()}).to_string()
        }
        "select-all" => {
            a.act(Act::SelectAll);
            ok()
        }
        "clear" => {
            a.pane_mut().sel.clear();
            ok()
        }
        "tab" => {
            let mut it = arg.split_whitespace();
            match it.next() {
                Some("new") => {
                    let rest: Vec<&str> = it.collect();
                    let p = rest.join(" ");
                    let loc = if p.is_empty() { a.pane().loc.clone() } else { Loc::Folder(PathBuf::from(p)) };
                    a.new_tab(loc);
                }
                Some("close") => {
                    let i = it.next().and_then(|x| x.parse().ok()).unwrap_or(a.cur);
                    a.close_tab(i);
                }
                Some(n) => match n.parse::<usize>() {
                    Ok(i) if i < a.tabs.len() => a.cur = i,
                    _ => return err("tab new [path] | close [N] | N"),
                },
                None => return err("tab new [path] | close [N] | N"),
            }
            ok()
        }
        "view" => {
            a.tab_mut().view = match arg {
                "details" => View::Details,
                "list" | "compact" => View::List,
                "grid" | "thumbnails" => View::Grid,
                _ => return err("view details|list|grid"),
            };
            ok()
        }
        "split" => {
            let on = a.tab().split();
            if (arg == "on" && !on) || (arg == "off" && on) || arg.is_empty() || arg == "toggle" {
                a.toggle_split();
            }
            if arg.starts_with("on ") {
                let p = PathBuf::from(&arg[3..]);
                if !a.tab().split() {
                    a.toggle_split();
                }
                let t = a.cur;
                a.go_in(t, 1, Loc::Folder(p), true);
            }
            ok()
        }
        "pane" => {
            let i: usize = arg.parse().unwrap_or(0);
            if i < a.tab().panes.len() {
                a.tab_mut().active = i;
            }
            ok()
        }
        "preview" => {
            if a.pv.is_some() && arg != "next" && arg != "prev" {
                a.pv = None;
            } else if arg == "next" {
                a.preview_step(1);
            } else if arg == "prev" {
                a.preview_step(-1);
            } else {
                a.preview_open();
            }
            ok()
        }
        "menu" => {
            let (t, p) = (a.cur, a.tab().active);
            let g = pane_geoms(a, t)[p];
            let i = a.pane().focus.unwrap_or(0);
            let (y, h, _) = row_geom(a, t, p, i);
            let (x, y) = if arg == "bg" { (g.list.x + g.list.w * 0.5, g.list.b() - 160.0) } else { (g.list.x + 230.0, g.list.y + y - a.pane().scroll + h / 2.0) };
            a.context_menu(x, y, arg == "bg" || a.pane().sel.is_empty());
            ok()
        }
        "whatis" => {
            a.act(Act::WhatIs);
            ok()
        }
        "rename" => {
            if a.pane().sel.len() > 1 {
                a.rename_open();
                if !arg.is_empty()
                    && let Some(r) = a.rename.as_mut()
                {
                    r.pattern = Field::new(arg);
                }
                rename_preview(a);
            } else {
                a.act(Act::Rename);
            }
            ok()
        }
        "suggest" => {
            a.suggest_names();
            ok()
        }
        "settings" => {
            a.open_settings();
            ok()
        }
        "palette" => {
            a.palette_open(arg);
            ok()
        }
        "transfers" => {
            a.xfer_open = !a.xfer_open;
            ok()
        }
        "copy" => {
            a.act(Act::Copy);
            ok()
        }
        "cut" => {
            a.act(Act::Cut);
            ok()
        }
        "paste" => {
            a.act(Act::Paste);
            ok()
        }
        "delete" => {
            a.act(Act::Delete);
            ok()
        }
        "undo" => {
            a.undo();
            ok()
        }
        "copy-to" | "move-to" => {
            let paths = a.pane().selected_paths();
            if paths.is_empty() {
                return err("nothing selected");
            }
            jobs::add(&a.jobs, if cmd == "move-to" { jobs::Kind::Move } else { jobs::Kind::Copy }, paths, PathBuf::from(arg));
            ok()
        }
        "pause" => {
            jobs::toggle_pause(&a.jobs, None);
            ok()
        }
        "conflict" => {
            // conflict skip|replace|keep-both [all]
            let mut it = arg.split_whitespace();
            let c = fmcore::ops::Conflict::parse(it.next().unwrap_or("skip")).unwrap_or(fmcore::ops::Conflict::Skip);
            let all = it.next() == Some("all");
            let id = a.jobs.0.lock().unwrap().jobs.iter().find(|j| j.state == jobs::State::Asking).map(|j| j.id);
            match id {
                Some(id) => {
                    jobs::decide(&a.jobs, id, 0, c, all);
                    ok()
                }
                None => err("no job is waiting for an answer"),
            }
        }
        "newfolder" => {
            let Some(d) = a.pane().dir() else { return err("not a folder") };
            let name = if arg.is_empty() { "New folder" } else { arg };
            match fmcore::ops::mkdir(&d.join(name)) {
                Ok(p) => {
                    a.pane_mut().want = Some(p);
                    a.refresh_all();
                    ok()
                }
                Err(e) => err(&e),
            }
        }
        "key" => {
            let Some((vk, c, s, al)) = parse_keys(arg) else { return err("key: e.g. ctrl+shift+n, f2, enter, esc, space, down") };
            a.key(vk, c, s, al);
            ok()
        }
        "type" => {
            for ch in arg.chars() {
                a.char_in(ch);
            }
            ok()
        }
        "click" | "dblclick" | "rclick" | "hover" => {
            let mut it = arg.split_whitespace();
            let (Some(x), Some(y)) = (it.next().and_then(num), it.next().and_then(num)) else { return err("click X Y (DIPs)") };
            a.mouse_move(x, y, false);
            match cmd {
                "click" => {
                    a.mouse_down(x, y, false, false, false);
                    a.mouse_up(x, y);
                }
                "dblclick" => {
                    a.mouse_down(x, y, false, false, false);
                    a.mouse_up(x, y);
                    a.mouse_down(x, y, false, false, true);
                    a.mouse_up(x, y);
                }
                "rclick" => a.right_click(x, y),
                _ => {}
            }
            ok()
        }
        "theme" => {
            a.forced_dark = match arg {
                "light" => Some(false),
                "dark" => Some(true),
                _ => None,
            };
            a.apply_theme();
            ok()
        }
        "size" => {
            let mut it = arg.split_whitespace();
            let (Some(w), Some(h)) = (it.next().and_then(num), it.next().and_then(num)) else { return err("size W H (DIPs)") };
            let s = a.scale();
            unsafe {
                let _ = SetWindowPos(a.hwnd, None, 0, 0, (w * s) as i32, (h * s) as i32, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
            ok()
        }
        "shot" => {
            if arg.is_empty() {
                return err("shot needs a .png path");
            }
            // settle: apply finished background work first
            a.take_bg();
            match a.shot(arg) {
                Ok(()) => json!({"ok": true, "file": arg}).to_string(),
                Err(e) => err(&e),
            }
        }
        "toast" => {
            a.toast(arg, "", arg.contains("Undo"));
            ok()
        }
        "close-popups" => {
            a.close_popups();
            a.pv = None;
            a.palette = None;
            a.rename = None;
            a.xfer_open = false;
            a.whatis = None;
            a.confirm = None;
            ok()
        }
        "quit" => {
            unsafe {
                let _ = PostMessageW(Some(a.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            ok()
        }
        _ => err("unknown gui command (see: wfm help)"),
    };
    a.invalidate();
    r
}

pub fn content_size(a: &App) -> f32 {
    content_h(a, a.cur, a.tab().active)
}
