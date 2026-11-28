//! Cleanup tab state, input and background work. All filesystem work for a scan or move stays off the UI thread.
#[path = "cleanup_paint.rs"]
mod drawing;

use crate::app::*;
use crate::gfx::Rect;
use crate::model::Loc;
use crate::motion::Follow;
use fmcore::cleanup::{self, BinInfo, Classification, Group, GroupId, Item, MoveReport, Skip, Verdict};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc};
use std::time::{Duration, Instant};

pub const BROOM: u32 = 0xEA99;
const GROUPS: [(&str, &str, u32); 6] = [
    ("Temporary files", "In temp folders, unchanged for 7 days or more", 0xE823),
    ("App caches and crash dumps", "Caches the apps rebuild, and old crash reports", 0xE8F1),
    ("Developer caches", cleanup::DEVELOPER_CACHE_REASON, 0xE943),
    ("Duplicate files", "Extra copies of files 10 MB or larger", 0xE8C8),
    ("Old installers in Downloads", "Setup files unchanged for more than 30 days", 0xE7B8),
    ("Large old files", "Over 500 MB, unchanged for 180 days", 0xE7C3),
];

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Target {
    Entry, Drive(char), Scan, Stop, Scope, Rescan, Ai,
    Tile(usize), Segment(usize), Items(usize), All, Row(usize), PanelClose,
    Move, Bin, Undo, Empty, Confirm, Cancel, Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open, Drive, SideDrive, Reveal, Copy, Hide, Properties, Unhide,
}

use Target::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Scan,
    Results,
    Review,
    Moving,
    Done,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Total {
    bytes: u64,
    count: usize,
}

impl Total {
    fn add(&mut self, other: Self) {
        self.bytes += other.bytes;
        self.count += other.count;
    }
}

#[derive(Clone)]
struct RowData {
    item: Item,
    on: bool,
    advice: Option<Classification>,
    advice_at: Option<Instant>,
}

#[derive(Default)]
struct TileData {
    rows: Vec<RowData>,
    total: Total,
    selected: Total,
    drives: BTreeMap<char, Total>,
    verdicts: [usize; 3],
    parts: u8,
    at: Option<Instant>,
    tick_at: Option<Instant>,
    fill: Follow,
}

impl TileData {
    fn recalc(&mut self) {
        let previous = self.selected;
        self.total = Total::default();
        self.selected = Total::default();
        self.drives.clear();
        self.verdicts = [0; 3];
        for r in &self.rows {
            let total = Total { bytes: r.item.size, count: 1 };
            self.total.add(total);
            let drive = drive_letter(&r.item.path).map(|d| self.drives.entry(d).or_default());
            if r.on {
                self.selected.add(total);
                if let Some(drive) = drive { drive.add(total); }
            }
            if let Some(a) = &r.advice {
                self.verdicts[verdict_rank(a.verdict) as usize] += 1;
            }
        }
        let fraction = self.selected.bytes as f32 / self.total.bytes.max(1) as f32;
        self.fill.to(fraction);
        if previous != self.selected {
            self.tick_at = Some(Instant::now());
        }
    }
}

fn verdict_rank(v: Verdict) -> u8 {
    match v { Verdict::CheckFirst => 0, Verdict::Keep => 1, Verdict::Safe => 2 }
}

fn group_index(id: GroupId) -> Option<usize> {
    use GroupId::*;
    Some(match id {
        Temp => 0, AppCaches | CrashDumps => 1, BuildCaches => 2,
        DuplicateFiles => 3, Installers => 4, LargeFiles => 5, RecycleBin => return None,
    })
}

fn drive_letter(path: &Path) -> Option<char> {
    use std::path::{Component, Prefix};
    match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(letter) => Some(char::from(letter.to_ascii_uppercase())),
            _ => None,
        },
        _ => None,
    }
}

type CacheKey = (PathBuf, u64, u64);

fn cache_key(item: &Item) -> CacheKey {
    (item.path.clone(), item.size, item.modified)
}

struct ConfirmData {
    items: Vec<Item>,
    limits: Option<Vec<(PathBuf, Skip)>>,
    by_group: [Total; 6],
    by_drive: BTreeMap<char, Total>,
    flagged: [Vec<String>; 2],
    on_cancel: bool,
    at: Instant,
}

struct DoneData {
    moved: Total,
    by_drive: HashMap<String, u64>,
    in_use: usize,
    too_big: usize,
    no_bin: usize,
    skipped: usize,
    journal: u64,
    records: Vec<fmcore::recycle::Recycled>,
    at: Instant,
}

enum Dialog {
    Move(ConfirmData),
    Empty { on_cancel: bool, at: Instant },
}

#[derive(Default)]
pub struct Page {
    phase: Phase,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    review_cancel: Option<Arc<ReviewCancel>>,
    scan_worker: Option<mpsc::Sender<ScanTask>>,
    waiting_for_drives: bool,
    scope: Option<Vec<String>>,
    scope_draft: Option<Vec<String>>,
    pub scope_focus: Option<usize>,
    review_after_scan: bool,
    return_tab: Option<u64>,
    tiles: [TileData; 6],
    bin: BinInfo,
    scan_at: Option<Instant>,
    seconds: f32,
    limited: bool,
    panel: Option<usize>,
    panel_at: Option<Instant>,
    reveal_panel_tile: bool,
    order: Vec<usize>,
    offsets: Vec<f32>,
    rows_moving: bool,
    focus: usize,
    row_focus: usize,
    keyboard: bool,
    scroll: Follow,
    panel_scroll: Follow,
    scroll_max: f32,
    list_height: f32,
    panel_max: f32,
    columns: usize,
    review_done: usize,
    show_notes: bool,
    cache: HashMap<CacheKey, Classification>,
    dialog: Option<Dialog>,
    done: Option<DoneData>,
    hidden: Vec<PathBuf>,
    progress: Follow,
    progress_at: Option<Instant>,
    found: Follow,
    selected: Follow,
    selected_count: Follow,
    pill_at: Option<Instant>,
    reduce_motion: Option<bool>,
}

impl Page {
    fn next(&mut self) -> u64 {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(cancel) = self.review_cancel.take() {
            cancel.stop();
        }
        self.generation += 1;
        self.generation
    }

    fn accepts(&self, generation: u64) -> bool {
        self.generation == generation
    }

    fn totals(&self, selected: bool) -> Total {
        let mut total = Total::default();
        for g in &self.tiles {
            total.add(if selected { g.selected } else { g.total });
        }
        total
    }

    fn by_drive(&self) -> BTreeMap<char, Total> {
        let mut drives = BTreeMap::<char, Total>::new();
        for tile in &self.tiles {
            for (&drive, &total) in &tile.drives {
                drives.entry(drive).or_default().add(total);
            }
        }
        drives
    }

    fn update_totals(&mut self) {
        let found = self.totals(false);
        let selected = self.totals(true);
        self.found.to(found.bytes as f32);
        self.selected.to(selected.bytes as f32);
        self.selected_count.to(selected.count as f32);
        if selected.count > 0 && self.pill_at.is_none() {
            self.pill_at = Some(Instant::now());
        }
    }

    fn receive_group(&mut self, group: Group) {
        let Some(i) = group_index(group.id) else { return };
        let tile = &mut self.tiles[i];
        tile.rows.extend(group.items.into_iter().map(|item| RowData {
            on: i < 3 && !item.is_package_download(), item, advice: None, advice_at: None,
        }));
        tile.parts += 1;
        if i != 1 || tile.parts == 2 {
            tile.at = Some(Instant::now());
        }
        self.limited |= group.limited;
        tile.recalc();
        self.update_totals();
        self.progress.to(self.tiles.iter().filter(|g| g.at.is_some()).count() as f32 / 6.0);
    }

    fn has_notes(&self) -> bool {
        self.tiles.iter().any(|tile| tile.verdicts != [0; 3])
    }

    fn visible(&self) -> Vec<usize> {
        (0..6).filter(|&i| self.phase == Phase::Scan || self.tiles[i].total.count > 0).collect()
    }

    fn set_group(&mut self, group: usize, on: bool) {
        for row in &mut self.tiles[group].rows {
            row.on = on;
        }
        self.tiles[group].recalc();
        self.update_totals();
    }

    fn toggle_group(&mut self, group: usize) {
        let t = &self.tiles[group];
        self.set_group(group, t.total.count != t.selected.count);
    }

    fn toggle_row(&mut self, row: usize) {
        if let Some(group) = self.panel && let Some(r) = self.tiles[group].rows.get_mut(row) {
            r.on = !r.on;
            self.tiles[group].recalc();
            self.update_totals();
        }
    }

    fn rate(&mut self, rows: Vec<(usize, usize, Classification)>) {
        self.review_done += rows.len();
        self.offsets.clear();
        let mut updated = [false; 6];
        for (g, i, advice) in rows {
            let row = &mut self.tiles[g].rows[i];
            row.on = advice.verdict == Verdict::Safe;
            row.advice_at = Some(Instant::now());
            self.cache.insert(cache_key(&row.item), advice.clone());
            row.advice = Some(advice);
            updated[g] = true;
        }
        for (tile, updated) in self.tiles.iter_mut().zip(updated) {
            if updated {
                tile.recalc();
            }
        }
        self.update_totals();
        self.progress.to(self.review_done as f32 / self.totals(false).count.max(1) as f32);
    }

    fn sort_panel(&mut self) {
        self.order.clear();
        self.offsets.clear();
        if let Some(group) = self.panel {
            let rows = &self.tiles[group].rows;
            self.order.extend(0..rows.len());
            self.order.sort_by_key(|&i| {
                let v = if self.show_notes && self.phase != Phase::Review { rows[i].advice.as_ref().map_or(3, |a| verdict_rank(a.verdict)) } else { 0 };
                (v, std::cmp::Reverse(rows[i].item.size))
            });
        }
        self.row_focus = self.row_focus.min(self.order.len().saturating_sub(1));
    }

    fn scope(&self, drives: &[fmcore::shell::Drive]) -> Vec<String> {
        self.scope.clone().unwrap_or_else(|| drives.iter().filter(|d| d.kind == "fixed" && !d.read_only).map(|d| d.root.clone()).collect())
    }

    fn selected_snapshot(&self) -> ConfirmData {
        let mut by_drive = self.by_drive();
        by_drive.retain(|_, total| total.count > 0);
        let mut snapshot = ConfirmData { items: vec![], limits: None, by_group: [Total::default(); 6], by_drive, flagged: Default::default(), on_cancel: false, at: Instant::now() };
        for (i, tile) in self.tiles.iter().enumerate() {
            snapshot.by_group[i] = tile.selected;
            for row in tile.rows.iter().filter(|r| r.on) {
                if let Some(a) = &row.advice && a.verdict != Verdict::Safe {
                    snapshot.flagged[verdict_rank(a.verdict) as usize].push(item_name(&row.item).into_owned());
                }
                snapshot.items.push(row.item.clone());
            }
        }
        snapshot
    }

    pub fn modal(&self) -> bool {
        self.dialog.is_some()
    }

    pub fn moving(&self) -> bool {
        self.phase == Phase::Moving
    }

    fn can_select(&self) -> bool {
        matches!(self.phase, Phase::Results | Phase::Review | Phase::Done)
    }

    fn can_move(&self) -> bool {
        matches!(self.phase, Phase::Results | Phase::Done) && self.totals(true).count != 0
    }
}

fn item_name(item: &Item) -> std::borrow::Cow<'_, str> {
    item.path.file_name().unwrap_or_default().to_string_lossy()
}

pub enum Update {
    Group(Group),
    Scanned(Result<(BinInfo, bool), String>),
    Rated(Vec<(usize, usize, Classification)>),
    Reviewed(Result<(), String>),
    Limits(Vec<(PathBuf, Skip)>),
    Moved(MoveReport, BinInfo),
    Undone(Result<(), String>),
    Emptied(Result<BinInfo, String>),
}

struct ScanTask {
    scope: Vec<String>,
    cancel: Arc<AtomicBool>,
    report: Box<dyn Fn(Update) + Send>,
}

fn scan_worker() -> mpsc::Sender<ScanTask> {
    let (tx, rx) = mpsc::channel::<ScanTask>();
    std::thread::spawn(move || {
        let _com = fmcore::util::com();
        for ScanTask { scope, cancel, report } in rx {
            if cancel.load(Ordering::Relaxed) {
                continue;
            }
            let bin = cleanup::bin_info(&scope);
            let result = cleanup::scan_in(Some(&scope), &cancel, &|_| {}, &mut |g| report(Update::Group(g.clone())))
                .map(|groups| (bin, groups.iter().any(|g| g.limited)));
            if !cancel.load(Ordering::Relaxed) {
                report(Update::Scanned(result));
            }
        }
    });
    tx
}

#[derive(Default)]
struct ReviewCancel(Mutex<bool>);

impl ReviewCancel {
    fn stopped(&self) -> bool {
        *self.0.lock().unwrap()
    }

    fn stop(&self) {
        *self.0.lock().unwrap() = true;
    }

    fn publish(&self, post: impl FnOnce()) {
        // Serialize cancel with the actual post, then let the generation reject any already queued updates.
        let stopped = self.0.lock().unwrap();
        if !*stopped {
            post();
        }
    }
}

fn review_batches(
    count: usize, cancel: &ReviewCancel, retry_delay: Duration,
    review: impl Fn(usize) -> Result<Vec<(usize, usize, Classification)>, String> + Sync,
    mut post: impl FnMut(Update),
) {
    const POLL: Duration = Duration::from_millis(25);
    std::thread::scope(|scope| {
        let (send, replies) = mpsc::channel();
        let mut pending = 0..count;
        let mut retries = VecDeque::new();
        let mut limit = 4;
        let mut in_flight = 0;
        let mut ready_at = Instant::now();
        while !cancel.stopped() {
            while in_flight < limit && Instant::now() >= ready_at && !cancel.stopped() {
                let Some(batch) = retries.pop_front().or_else(|| pending.next()) else { break };
                let (send, review) = (send.clone(), &review);
                let started_at_limit = limit;
                scope.spawn(move || {
                    if !cancel.stopped() {
                        let _ = send.send((batch, started_at_limit, review(batch)));
                    }
                });
                in_flight += 1;
            }
            if in_flight == 0 && pending.is_empty() && retries.is_empty() {
                cancel.publish(|| post(Update::Reviewed(Ok(()))));
                break;
            }
            let Ok((batch, started_at_limit, result)) = replies.recv_timeout(POLL) else { continue };
            in_flight -= 1;
            match result {
                Ok(rows) => cancel.publish(|| post(Update::Rated(rows))),
                Err(e) if e == fmcore::ai::FREE_QUOTA && started_at_limit > 1 => {
                    // Replies from the old tier may still arrive. Each tier can drop the cap only once.
                    limit = limit.min(started_at_limit / 2);
                    ready_at = Instant::now() + retry_delay;
                    retries.push_back(batch);
                }
                Err(e) => {
                    cancel.publish(|| post(Update::Reviewed(Err(e))));
                    cancel.stop();
                    break;
                }
            }
        }
    });
}

fn empty_confirmed(confirmed: bool, empty: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if confirmed { empty() } else { Err("Empty Recycle Bin cancelled".into()) }
}

impl App {
    pub fn cleanup_status(&self) -> serde_json::Value {
        let p = &self.cleanup;
        let total = p.totals(false);
        let selected = p.totals(true);
        serde_json::json!({
            "phase": format!("{:?}", p.phase), "panel": p.panel, "confirm": p.modal(),
            "ready": matches!(&p.dialog, Some(Dialog::Move(c)) if c.limits.is_some()),
            "found_items": total.count, "found_bytes": total.bytes,
            "selected_items": selected.count, "selected_bytes": selected.bytes,
            "groups": p.visible().into_iter().map(|g| serde_json::json!({
                "name": GROUPS[g].0, "items": p.tiles[g].total.count, "selected": p.tiles[g].selected.count,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn cleanup_tabs_changed(&mut self) {
        if !self.cleanup.moving() && self.tabs.iter().all(|t| t.pane().loc != Loc::Cleanup) {
            self.close_cleanup();
        }
    }

    /// The drives Cleanup can scan: ready and writable (never a DVD, a mounted ISO or a write-protected card).
    pub fn scan_drives(&self) -> Vec<fmcore::shell::Drive> {
        self.drives.iter().filter(|d| d.ready && !d.read_only).cloned().collect()
    }

    /// The drive list changed: drives that went away (or became read-only) leave the chosen scope, and an open
    /// "Drives" menu is rebuilt from the new list (its rows are drives by letter, never by position).
    pub fn cleanup_drives_changed(&mut self) {
        let scan = self.scan_drives();
        let keep = |v: &mut Vec<String>| v.retain(|r| scan.iter().any(|d| d.root.eq_ignore_ascii_case(r)));
        if let Some(s) = &mut self.cleanup.scope {
            keep(s);
            if s.is_empty() {
                self.cleanup.scope = None;
            }
        }
        let fallback = self.cleanup.scope(&scan);
        if let Some(s) = &mut self.cleanup.scope_draft {
            keep(s);
            if s.is_empty() {
                *s = fallback;
            }
        }
        if self.drop.as_ref().is_some_and(|d| d.kind == DropKind::CleanupScope) {
            let items = self.cleanup_drop_items(DropKind::CleanupScope);
            let n = items.len();
            if let Some(drop) = &mut self.drop {
                drop.items = items;
            }
            if let Some(f) = &mut self.cleanup.scope_focus {
                *f = (*f).min(n.saturating_sub(1));
            }
        }
    }

    pub fn cleanup_drives_ready(&mut self) {
        if std::mem::take(&mut self.cleanup.waiting_for_drives) {
            self.cleanup.phase = Phase::Idle;
            if self.drives.is_empty() {
                self.toast_err("No drives are available to scan");
            } else {
                self.start_cleanup_scan(false);
            }
        }
    }

    pub fn cleanup_keyboard_menu(&mut self) {
        if let Some(&i) = self.cleanup.order.get(self.cleanup.row_focus) {
            if let Some(r) = self.hits.iter().find(|(_, h)| *h == Hit::Cleanup(Row(i))).map(|(r, _)| *r) {
                self.cleanup_menu(i, r.x + 36.0, r.cy());
            }
        }
    }

    pub fn refresh_cleanup_hidden(&mut self) {
        self.cleanup.hidden = cleanup::ignored();
    }

    pub fn cleanup_motion_changed(&mut self) {
        self.cleanup.reduce_motion = None;
    }

    fn cleanup_post(&self, id: u64) -> impl Fn(Update) + Send + use<> {
        let bg = self.bg.clone();
        let hw = self.hwnd.0 as isize;
        move |m| post_bg(&bg, hw, Msg::Cleanup(id, m))
    }

    pub fn open_cleanup(&mut self, drive: Option<&Path>) {
        if self.pane().loc != Loc::Cleanup {
            self.cleanup.return_tab = Some(self.tab().id);
        }
        let was_open = self.tabs.iter().position(|t| t.pane().loc == Loc::Cleanup);
        if let Some(i) = was_open {
            self.select_tab(i);
        } else {
            self.new_tab(Loc::Cleanup);
        }
        self.search_focus = false;
        self.path_edit = None;
        self.whatis = None;
        if self.cleanup.moving() {
            return;
        }
        if let Some(drive) = drive {
            self.cleanup.scope = fmcore::recycle::drive_root(drive).map(|r| vec![r]);
            self.start_cleanup_scan(false);
        } else if was_open.is_none() || self.cleanup.phase == Phase::Idle {
            self.start_cleanup_scan(false);
        }
    }

    pub fn cleanup_back_tab(&self) -> Option<usize> {
        let id = self.cleanup.return_tab?;
        (self.pane().loc == Loc::Cleanup).then_some(())?;
        self.tabs.iter().position(|t| t.id == id)
    }

    fn start_cleanup_scan(&mut self, review: bool) {
        if self.cleanup.moving() {
            return;
        }
        let scope = self.cleanup.scope(&self.drives);
        if scope.is_empty() {
            if self.drives.is_empty() && self.cleanup.scope.is_none() && self.cleanup.phase == Phase::Idle {
                self.cleanup.waiting_for_drives = true;
                self.cleanup.phase = Phase::Scan;
                self.cleanup.scan_at = Some(Instant::now());
            }
            return;
        }
        self.cleanup.scope = Some(scope.clone());
        self.cleanup.review_after_scan = review;
        let id = self.cleanup.next();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cleanup.cancel = Some(cancel.clone());
        self.cleanup.phase = Phase::Scan;
        self.cleanup.tiles = Default::default();
        self.cleanup.bin = BinInfo::default();
        self.cleanup.limited = false;
        self.cleanup.dialog = None;
        self.cleanup.done = None;
        self.cleanup.scan_at = Some(Instant::now());
        self.cleanup.progress_at = None;
        self.cleanup.progress.set(0.0);
        self.cleanup.found.set(0.0);
        self.cleanup.selected.set(0.0);
        self.cleanup.selected_count.set(0.0);
        self.cleanup.pill_at = None;
        self.cleanup.scroll.set(0.0);
        self.cleanup.show_notes = false;
        self.cleanup_panel(None);
        let report = Box::new(self.cleanup_post(id));
        let worker = self.cleanup.scan_worker.get_or_insert_with(scan_worker);
        if worker.send(ScanTask { scope, cancel, report }).is_err() {
            self.cleanup.phase = Phase::Idle;
            self.cleanup.scan_worker = None;
            self.toast_err("Could not start Cleanup");
        }
        self.invalidate();
    }

    pub fn close_cleanup(&mut self) {
        self.cleanup.next();
        self.cleanup.phase = Phase::Idle;
        self.cleanup.waiting_for_drives = false;
        self.cleanup.dialog = None;
        self.cleanup.scope = None;
        self.cleanup.scope_draft = None;
        self.cleanup.scope_focus = None;
        self.cleanup.return_tab = None;
        self.cleanup.tiles = Default::default();
        self.cleanup.done = None;
        self.cleanup_panel(None);
    }

    pub fn stop_cleanup_review(&mut self) {
        if self.cleanup.phase == Phase::Review {
            self.cleanup.next();
            self.cleanup.phase = if self.cleanup.done.is_some() { Phase::Done } else { Phase::Results };
            self.cleanup.progress_at = Some(Instant::now());
            self.cleanup.sort_panel();
        }
    }

    fn cleanup_stop(&mut self) {
        match self.cleanup.phase {
            Phase::Scan => {
                self.cleanup.next();
                self.cleanup.phase = Phase::Idle;
                self.cleanup.waiting_for_drives = false;
                self.cleanup.tiles = Default::default();
            }
            Phase::Review => self.stop_cleanup_review(),
            _ => {}
        }
    }

    fn cleanup_panel(&mut self, group: Option<usize>) {
        self.cleanup.panel = group;
        self.cleanup.panel_at = group.map(|_| Instant::now());
        self.cleanup.reveal_panel_tile = group.is_some();
        self.cleanup.panel_scroll.set(0.0);
        self.cleanup.row_focus = 0;
        if let Some(i) = group {
            self.cleanup.focus = i;
            self.cleanup.pill_at = Some(Instant::now());
        }
        self.cleanup.sort_panel();
    }

    fn cleanup_review_click(&mut self) {
        if self.ai_on && self.cleanup.phase == Phase::Results && self.cleanup.has_notes() {
            self.start_cleanup_scan(true);
        } else {
            self.start_cleanup_review();
        }
    }

    fn start_cleanup_review(&mut self) {
        if !self.ai_on || !matches!(self.cleanup.phase, Phase::Results | Phase::Done) {
            return;
        }
        let mut items = vec![];
        let mut positions = vec![];
        let mut cached = vec![];
        for (g, tile) in self.cleanup.tiles.iter().enumerate() {
            for (i, row) in tile.rows.iter().enumerate() {
                if let Some(advice) = self.cleanup.cache.get(&cache_key(&row.item)) {
                    cached.push((g, i, advice.clone()));
                } else {
                    positions.push((g, i));
                    items.push(row.item.clone());
                }
            }
        }
        if items.is_empty() && cached.is_empty() {
            return;
        }
        let id = self.cleanup.next();
        let cancel = Arc::new(ReviewCancel::default());
        self.cleanup.review_cancel = Some(cancel.clone());
        self.cleanup.phase = Phase::Review;
        self.cleanup.show_notes = true;
        self.cleanup.review_done = 0;
        self.cleanup.progress.set(0.0);
        self.cleanup.progress_at = None;
        for tile in &mut self.cleanup.tiles {
            for row in &mut tile.rows {
                row.advice = None;
                row.advice_at = None;
            }
            tile.recalc();
        }
        if !cached.is_empty() {
            self.cleanup.rate(cached);
        }
        self.cleanup.sort_panel();
        let post = self.cleanup_post(id);
        std::thread::spawn(move || {
            let size = cleanup::REVIEW_BATCH;
            review_batches(items.len().div_ceil(size), &cancel, Duration::from_secs(2), |batch| {
                let start = batch * size;
                let end = (start + size).min(items.len());
                let advice = cleanup::review_cancellable(&items[start..end], &|| cancel.stopped())?;
                Ok(positions[start..end].iter().zip(advice).map(|(&(g, i), answer)| (g, i, answer)).collect())
            }, post);
        });
        self.invalidate();
    }

    fn cleanup_confirm(&mut self) {
        if !self.cleanup.can_move() {
            return;
        }
        let snapshot = self.cleanup.selected_snapshot();
        let items = snapshot.items.clone();
        let id = self.cleanup.next();
        self.cleanup.dialog = Some(Dialog::Move(snapshot));
        self.spawn(move || Msg::Cleanup(id, Update::Limits(cleanup::bin_limits(&items))));
    }

    fn cleanup_commit(&mut self) {
        if matches!(&self.cleanup.dialog, Some(Dialog::Move(c)) if c.limits.is_none()) {
            return;
        }
        let Some(dialog) = self.cleanup.dialog.take() else { return };
        // Test windows exercise the same confirm UI without ever changing the host's Recycle Bin.
        if std::env::var_os("WFILES_TEST").is_some() || self.offscreen {
            self.toast_err("Cleanup moves are disabled in test windows");
            return;
        }
        let id = self.cleanup.next();
        let roots = self.cleanup.scope(&self.drives);
        self.cleanup.phase = Phase::Moving;
        self.spawn(move || Msg::Cleanup(id, match dialog {
            Dialog::Move(c) => {
                let skipped = c.limits.unwrap_or_default();
                let paths: HashSet<_> = skipped.iter().map(|(p, _)| p).collect();
                let mut report = cleanup::move_items(c.items.into_iter().filter(|i| !paths.contains(&i.path)).collect());
                report.skipped.extend(skipped);
                Update::Moved(report, cleanup::bin_info(&roots))
            }
            Dialog::Empty { .. } => Update::Emptied(empty_confirmed(true, fmcore::recycle::empty_all).map(|()| BinInfo::default())),
        }));
    }

    fn cleanup_undo(&mut self) {
        if std::env::var_os("WFILES_TEST").is_some() || self.offscreen {
            return;
        }
        let Some(done) = &self.cleanup.done else {
            if self.cleanup.phase == Phase::Results || self.cleanup.phase == Phase::Idle {
                self.spawn(|| match fmcore::journal::load().into_iter().rev().find(|o| o.kind == "cleanup") {
                    Some(op) => match fmcore::journal::undo_cleanup(op.id, &op.recycled) {
                        Ok(_) => Msg::Refresh,
                        Err(e) => Msg::ToastErr(e),
                    },
                    None => Msg::ToastErr("Nothing to undo".into()),
                });
            }
            return;
        };
        if self.cleanup.phase != Phase::Done || done.records.is_empty() {
            return;
        }
        let (journal, records) = (done.journal, done.records.clone());
        let id = self.cleanup.next();
        self.cleanup.phase = Phase::Moving;
        self.spawn(move || {
            let result = if journal != 0 {
                fmcore::journal::undo_cleanup(journal, &records).map(|_| ())
            } else {
                records.iter().filter(|r| r.data.exists()).map(fmcore::recycle::restore).collect::<Result<Vec<_>, _>>().map(|_| ())
            };
            Msg::Cleanup(id, Update::Undone(result))
        });
    }

    pub fn cleanup_updated(&mut self, id: u64, m: Update) {
        if !self.cleanup.accepts(id) {
            return;
        }
        match m {
            Update::Group(g) => self.cleanup.receive_group(g),
            Update::Scanned(res) => match res {
                Ok((bin, limited)) => {
                    self.cleanup.bin = bin;
                    self.cleanup.limited = limited;
                    self.cleanup.phase = Phase::Results;
                    self.cleanup.seconds = self.cleanup.scan_at.map_or(0.0, |t| t.elapsed().as_secs_f32());
                    self.cleanup.progress_at = Some(Instant::now());
                    self.cleanup.pill_at = Some(Instant::now());
                    self.cleanup.focus = self.cleanup.visible().first().copied().unwrap_or(0);
                    if std::mem::take(&mut self.cleanup.review_after_scan) {
                        self.start_cleanup_review();
                    }
                }
                Err(e) => {
                    self.cleanup.phase = Phase::Idle;
                    self.toast_err(&e);
                }
            },
            Update::Rated(rows) => self.cleanup.rate(rows),
            Update::Reviewed(r) => {
                self.stop_cleanup_review();
                if let Err(e) = r {
                    self.toast_err(&e);
                }
            }
            Update::Limits(limits) => {
                if let Some(Dialog::Move(c)) = &mut self.cleanup.dialog {
                    c.limits = Some(limits);
                }
            }
            Update::Moved(r, bin) => {
                self.cleanup.bin = bin;
                self.cleanup_moved(r);
                self.refresh_all();
            }
            Update::Undone(r) => match r {
                Ok(()) => {
                    self.cleanup.phase = Phase::Results;
                    self.toast("Cleanup undone", "", false);
                    self.refresh_all();
                    self.start_cleanup_scan(false);
                }
                Err(e) => {
                    self.cleanup.phase = Phase::Done;
                    self.toast_err(&e);
                }
            },
            Update::Emptied(r) => {
                self.cleanup.phase = Phase::Done;
                match r {
                    Ok(bin) => {
                        self.cleanup.bin = bin;
                        if let Some(done) = &mut self.cleanup.done {
                            done.by_drive.clear();
                            done.records.clear();
                            done.journal = 0;
                        }
                        self.toast("The Recycle Bin is empty", "", false);
                        self.refresh_all();
                        // free space changed (a full read: drives_fast has no labels or sizes, and This PC lost them)
                        self.refresh_drives();
                    }
                    Err(e) => self.toast_err(&e),
                }
            }
        }
        self.invalidate();
    }

    fn cleanup_moved(&mut self, r: MoveReport) {
        let paths: HashSet<_> = r.moved.iter().map(|i| &i.path).collect();
        let mut by_drive = HashMap::new();
        let mut moved = Total::default();
        for i in &r.moved {
            moved.add(Total { bytes: i.size, count: 1 });
            if let Some(d) = fmcore::recycle::drive_root(&i.path) {
                *by_drive.entry(d).or_default() += i.size;
            }
        }
        self.cleanup.phase = Phase::Done;
        self.cleanup.scroll.set(0.0);
        self.cleanup.pill_at = None;
        self.cleanup.done = Some(DoneData {
            moved, by_drive, in_use: r.skipped.iter().filter(|(_, why)| *why == Skip::InUse).count(),
            too_big: r.skipped.iter().filter(|(_, why)| *why == Skip::TooBig).count(),
            no_bin: r.skipped.iter().filter(|(_, why)| *why == Skip::NoBin).count(),
            skipped: r.skipped.len(), journal: r.journal, records: r.records, at: Instant::now(),
        });
        for tile in &mut self.cleanup.tiles {
            tile.rows.retain(|r| !paths.contains(&r.item.path));
            for row in &mut tile.rows {
                row.on = false;
            }
            tile.recalc();
        }
        self.cleanup.update_totals();
        self.cleanup_panel(None);
        if let Some(e) = r.warning {
            self.toast_err(&e);
        }
    }
}

impl App {
    pub fn cleanup_click(&mut self, target: Target, r: Rect, double: bool) {
        if self.cleanup.modal() {
            match target {
                Confirm => self.cleanup_commit(),
                Cancel => self.cleanup.dialog = None,
                _ => {}
            }
            return;
        }
        self.cleanup.keyboard = false;
        match target {
            Entry => self.open_cleanup(None),
            Drive(l) if self.cleanup.phase == Phase::Idle => self.cleanup_pick_drive(l),
            Scan | Rescan => self.start_cleanup_scan(false),
            Stop => self.cleanup_stop(),
            Scope => {
                let scan = self.scan_drives();
                let mut draft = self.cleanup.scope(&scan);
                draft.retain(|r| scan.iter().any(|d| &d.root == r));
                self.cleanup.scope_draft = Some(draft);
                self.cleanup.scope_focus = None;
                self.open_drop(DropKind::CleanupScope, r, 240.0);
            }
            Ai => self.cleanup_review_click(),
            Tile(g) if double => self.cleanup_panel(Some(g)),
            Tile(g) if self.cleanup.can_select() => {
                self.cleanup.focus = g;
                self.cleanup.toggle_group(g);
            }
            Items(g) => self.cleanup_panel((self.cleanup.panel != Some(g)).then_some(g)),
            PanelClose => self.cleanup_panel(None),
            All if let Some(g) = self.cleanup.panel && self.cleanup.can_select() => self.cleanup.toggle_group(g),
            Row(i) if double && let Some(path) = self.cleanup_path(i) => self.cleanup_action(Command::Reveal, Some(path)),
            Row(i) if self.cleanup.can_select() => {
                self.cleanup.row_focus = self.cleanup.order.iter().position(|&r| r == i).unwrap_or(0);
                self.cleanup.toggle_row(i);
            }
            Move => self.cleanup_confirm(),
            Bin => self.new_tab(Loc::RecycleBin),
            Undo => self.cleanup_undo(),
            Empty if self.cleanup.phase == Phase::Done && self.cleanup.bin.count != 0 => {
                self.cleanup.dialog = Some(Dialog::Empty { on_cancel: true, at: Instant::now() });
            }
            Hidden => self.open_drop(DropKind::CleanupHidden, r, r.w.clamp(240.0, 620.0)),
            _ => {}
        }
    }

    fn cleanup_pick_drive(&mut self, letter: char) {
        let scan = self.scan_drives();
        let Some(d) = scan.iter().find(|d| d.root.starts_with(letter)) else { return };
        let mut scope = self.cleanup.scope(&scan);
        if let Some(i) = scope.iter().position(|r| r == &d.root) {
            scope.remove(i);
        } else {
            scope.push(d.root.clone());
        }
        self.cleanup.scope = Some(scope);
    }

    fn cleanup_path(&self, row: usize) -> Option<PathBuf> {
        let g = self.cleanup.panel?;
        Some(self.cleanup.tiles[g].rows.get(row)?.item.path.clone())
    }

    pub fn cleanup_menu(&mut self, row: usize, x: f32, y: f32) {
        let Some(path) = self.cleanup_path(row) else { return };
        self.cleanup.row_focus = self.cleanup.order.iter().position(|&i| i == row).unwrap_or(0);
        let cl = |label, glyph, act| MItem::new(label, glyph, Act::Cleanup(act));
        let mut first = cl("Show in folder", 0xE8E5, Command::Reveal).keys("Enter");
        first.bold = true;
        let items = vec![
            first,
            cl("Copy path", 0xE71B, Command::Copy).keys("Ctrl+Shift+C"),
            cl("Never list this", 0xE7B3, Command::Hide).sep(),
            cl("Properties", 0xE946, Command::Properties).keys("Alt+Enter").sep(),
        ];
        self.menu_gen += 1;
        self.menu = Some(CtxMenu { x, y, items, paths: vec![path], bg: true, seq: self.menu_gen, loading: false,
            shell: vec![], apps: vec![], at: Instant::now() });
    }

    pub fn cleanup_menu_key(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        let Some(m) = &self.menu else { return false };
        if !m.items.first().is_some_and(|i| i.act == Act::Cleanup(Command::Reveal)) {
            return false;
        }
        let cmd = match (vk, ctrl, shift, alt) {
            (0x0D, false, false, false) => Command::Reveal,
            (0x0D, false, false, true) => Command::Properties,
            (0x43, true, true, false) => Command::Copy,
            _ => return false,
        };
        let path = m.paths.first().cloned();
        self.retire_menu();
        self.cleanup_action(cmd, path);
        true
    }

    pub fn cleanup_action(&mut self, cmd: Command, path: Option<PathBuf>) {
        match cmd {
            Command::Open => self.open_cleanup(None),
            Command::Drive | Command::SideDrive if let Some(p) = path => self.open_cleanup(Some(&p)),
            Command::Reveal if let Some(p) = path.as_ref() && let Some(parent) = p.parent() => {
                self.new_tab(Loc::Folder(parent.to_path_buf()));
                self.pane_mut().want = Some(p.clone());
            }
            Command::Copy if let Some(p) = path => {
                crate::dnd::set_clipboard_text(self.hwnd, &format!("\"{}\"", p.display()));
                self.toast("Path copied", "", false);
            }
            Command::Properties if let Some(p) = path => {
                let w = fmcore::util::wide_path(&p);
                unsafe {
                    use windows::Win32::UI::Shell::{SHObjectProperties, SHOP_FILEPATH};
                    let _ = SHObjectProperties(Some(self.hwnd), SHOP_FILEPATH, windows::core::PCWSTR(w.as_ptr()), None);
                }
            }
            Command::Hide | Command::Unhide if let Some(p) = path => {
                let hide = cmd == Command::Hide;
                if let Err(e) = cleanup::set_ignored(&p, hide) {
                    self.toast_err(&e);
                    return;
                }
                self.cleanup.hidden = cleanup::ignored();
                if hide {
                    self.stop_cleanup_review();
                    if let Some(g) = self.cleanup.panel {
                        let tile = &mut self.cleanup.tiles[g];
                        tile.rows.retain(|r| !fmcore::util::path_eq(&p, &r.item.path));
                        tile.recalc();
                        self.cleanup.update_totals();
                        if self.cleanup.tiles[g].total.count == 0 {
                            self.cleanup_panel(None);
                        } else {
                            self.cleanup.sort_panel();
                        }
                    }
                    self.toast("Hidden from Cleanup", "", false);
                } else {
                    self.toast("Cleanup can list this item again", "", false);
                }
            }
            _ => {}
        }
    }

    pub fn cleanup_drop_items(&self, kind: DropKind) -> Vec<MItem> {
        match kind {
            DropKind::CleanupScope => {
                let scope = self.cleanup.scope_draft.as_deref().unwrap_or_default();
                let scan = self.scan_drives();
                let mut all = MItem::new("All drives", 0, Act::None);
                all.checked = scan.iter().all(|d| (d.kind == "fixed") == scope.contains(&d.root));
                let mut items = vec![all];
                // each row carries its drive's root: a click picks that drive even if the list changed meanwhile
                for (i, d) in scan.iter().enumerate() {
                    let mut item = MItem::new(&drive_name(d), 0xEDA2, Act::None);
                    item.checked = scope.contains(&d.root);
                    item.shell = Some(d.root.clone());
                    item.sep = i == 0;
                    items.push(item);
                }
                items
            }
            DropKind::CleanupHidden => self.cleanup.hidden.iter().enumerate().map(|(i, p)| MItem {
                value: i as i64,
                ..MItem::new(&p.to_string_lossy(), 0xE7A7, Act::Cleanup(Command::Unhide)).keys("Show again")
            }).collect(),
            _ => vec![],
        }
    }

    /// A row of the open "Drives" menu was clicked (or Enter / Space on it): None = "All drives", else that drive.
    pub fn cleanup_toggle_scope(&mut self, root: Option<String>) {
        let scan = self.scan_drives();
        let Some(scope) = &mut self.cleanup.scope_draft else { return };
        if root.is_none() {
            let all: Vec<_> = scan.iter().filter(|d| d.kind == "fixed").map(|d| d.root.clone()).collect();
            if !all.is_empty() {
                if all.len() == scope.len() && all.iter().all(|d| scope.contains(d)) {
                    scope.truncate(1);
                } else {
                    *scope = all;
                }
            }
        } else if let Some(d) = scan.iter().find(|d| root.as_ref().is_some_and(|r| r.eq_ignore_ascii_case(&d.root))) {
            if let Some(i) = scope.iter().position(|r| r == &d.root) {
                if scope.len() > 1 {
                    scope.remove(i);
                }
            } else {
                scope.push(d.root.clone());
            }
        }
        let items = self.cleanup_drop_items(DropKind::CleanupScope);
        if let Some(drop) = &mut self.drop {
            drop.items = items;
        }
        self.invalidate();
    }

    pub fn cleanup_finish_scope(&mut self) -> bool {
        self.cleanup.scope_focus = None;
        let Some(scope) = self.cleanup.scope_draft.take() else { return false };
        let before = self.cleanup.scope(&self.scan_drives());
        if scope.len() != before.len() || !scope.iter().all(|d| before.contains(d)) {
            self.cleanup.scope = Some(scope);
            self.start_cleanup_scan(false);
            return true;
        }
        false
    }

    pub fn cleanup_scope_key(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) {
        if ctrl || shift || alt { return; }
        let Some(drop) = &self.drop else { return };
        let n = drop.items.len();
        match vk {
            0x26 => self.cleanup.scope_focus = Some(self.cleanup.scope_focus.map_or(n - 1, |i| (i + n - 1) % n)),
            0x28 => self.cleanup.scope_focus = Some(self.cleanup.scope_focus.map_or(0, |i| (i + 1) % n)),
            0x0D | 0x20 => {
                let i = *self.cleanup.scope_focus.get_or_insert(0);
                let root = drop.items.get(i).and_then(|it| it.shell.clone());
                self.cleanup_toggle_scope(root);
            }
            _ => {}
        }
    }

    pub fn cleanup_apply_drop(&mut self, item: &MItem) {
        self.cleanup_action(Command::Unhide, self.cleanup.hidden.get(item.value as usize).cloned());
    }

    pub fn cleanup_tip(&self, h: Target) -> Option<String> {
        if !self.hits.iter().any(|(_, hit)| *hit == Hit::Cleanup(h)) {
            return None;
        }
        Some(match h {
            Entry if self.toolbar_compact() => "Clean up".into(),
            Entry => "Find items to clean up on these drives".into(),
            Rescan => "Scan again (F5)".into(),
            Stop if self.cleanup.phase == Phase::Scan => "Cancel scan (Esc)".into(),
            Stop => "Stop AI Review (Esc)".into(),
            Undo => "Undo cleanup (Ctrl+Z)".into(),
            Ai => "Sends the names, folders, sizes and dates of the listed items (Shift+Enter)".into(),
            Move if self.cleanup.phase == Phase::Review => "Stop AI Review first".into(),
            Segment(g) | Tile(g) => format!("{} · {}", GROUPS[g].0, crate::fmt::size(self.cleanup.tiles[g].total.bytes)),
            Items(_) => "Show the items in this group".into(),
            Row(i) => {
                let group = self.cleanup.panel?;
                let kept = self.cleanup.tiles[group].rows.get(i)?.item.copy_of.as_ref()?;
                format!("Copy of {}", kept.display())
            }
            PanelClose => "Close Items (Esc)".into(),
            Scan => "Scan the selected drives".into(),
            Hidden => "Show hidden items and add them back to Cleanup".into(),
            _ => return None,
        })
    }

    pub fn cleanup_wheel(&mut self, x: f32, y: f32, amount: f32) -> bool {
        if self.cleanup.modal() || !crate::paint::content_rect(self).has(x, y) {
            return self.cleanup.modal();
        }
        let c = crate::paint::content_rect(self);
        let panel = self.cleanup.panel.is_some() && x >= c.r() - c.w.min(440.0);
        let (f, max) = if panel { (&mut self.cleanup.panel_scroll, self.cleanup.panel_max) } else { (&mut self.cleanup.scroll, self.cleanup.scroll_max) };
        f.to((f.target - amount / 120.0 * 78.0).clamp(0.0, max));
        self.anim();
        self.invalidate();
        true
    }

    pub fn cleanup_key(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        const ESC: u16 = 0x1B;
        const ENTER: u16 = 0x0D;
        const SPACE: u16 = 0x20;
        const TAB: u16 = 0x09;
        if let Some(dialog) = &mut self.cleanup.dialog {
            let on_cancel = match dialog { Dialog::Move(c) => &mut c.on_cancel, Dialog::Empty { on_cancel, .. } => on_cancel };
            match (vk, ctrl, alt) {
                (ESC, ..) => self.cleanup.dialog = None,
                (TAB, false, false) => *on_cancel = !*on_cancel,
                (ENTER | SPACE, false, false) if *on_cancel => self.cleanup.dialog = None,
                (ENTER | SPACE, false, false) if !shift => self.cleanup_commit(),
                _ => {}
            }
            return true;
        }
        match (vk, ctrl, shift, alt) {
            // Keep the window's tab and palette keys.
            (0x4B | 0x54 | 0x57 | TAB | 0x31..=0x39 | 0xBC, true, ..) | (0x25 | 0x27, false, false, true) => return false,
            (ESC, ..) if matches!(self.cleanup.phase, Phase::Scan | Phase::Review) => self.cleanup_stop(),
            (ESC, ..) => {
                self.cleanup.keyboard = true;
                self.cleanup_panel(None);
            }
            (0x74, false, false, false) => self.start_cleanup_scan(false),
            (0x79, false, true, _) | (0x5D, false, false, false) => self.cleanup_keyboard_menu(),
            (ENTER, false, true, false) => self.cleanup_review_click(),
            (ENTER, false, false, false) if self.cleanup.phase == Phase::Idle => self.start_cleanup_scan(false),
            (0x5A, true, false, false) => self.cleanup_undo(),
            (0x2E, false, false, false) => self.cleanup_confirm(),
            _ if !self.cleanup.can_select() => {}
            (0x41, true, _, false) => {
                if let Some(g) = self.cleanup.panel {
                    self.cleanup.set_group(g, !shift);
                } else {
                    for g in 0..6 {
                        self.cleanup.set_group(g, !shift);
                    }
                }
            }
            _ if self.cleanup.panel.is_some() => self.cleanup_row_key(vk, ctrl, shift, alt),
            (0x23..=0x28 | SPACE | ENTER, false, false, false) => self.cleanup_tile_key(vk),
            _ => {}
        }
        true
    }

    fn cleanup_tile_key(&mut self, vk: u16) {
        let visible = self.cleanup.visible();
        let Some(&first) = visible.first() else { return };
        let i = visible.iter().position(|&g| g == self.cleanup.focus).unwrap_or(0);
        let cols = self.cleanup.columns.max(1);
        let next = match vk {
            0x23 => visible.len() - 1,
            0x24 => 0,
            0x25 => i.saturating_sub(1),
            0x26 => i.saturating_sub(cols),
            0x27 => i + 1,
            0x28 => i + cols,
            0x20 => {
                self.cleanup.toggle_group(*visible.get(i).unwrap_or(&first));
                i
            }
            0x0D => {
                self.cleanup_panel(Some(*visible.get(i).unwrap_or(&first)));
                return;
            }
            _ => i,
        }.min(visible.len() - 1);
        self.cleanup.keyboard = true;
        self.cleanup.focus = visible[next];
        self.cleanup_reveal_tile(next);
    }

    fn cleanup_row_key(&mut self, vk: u16, ctrl: bool, shift: bool, alt: bool) {
        let p = &mut self.cleanup;
        let n = p.order.len();
        let Some(&i) = p.order.get(p.row_focus) else { return };
        let cmd = match (vk, ctrl, shift, alt) {
            (0x23, false, false, false) => { p.row_focus = n - 1; None }
            (0x24, false, false, false) => { p.row_focus = 0; None }
            (0x26, false, false, false) => { p.row_focus = p.row_focus.saturating_sub(1); None }
            (0x28, false, false, false) => { p.row_focus = (p.row_focus + 1).min(n - 1); None }
            (0x20, false, false, false) => { p.toggle_row(i); None }
            (0x0D, false, false, false) => Some(Command::Reveal),
            (0x0D, false, false, true) => Some(Command::Properties),
            (0x43, true, true, false) => Some(Command::Copy),
            _ => return,
        };
        if let Some(cmd) = cmd {
            self.cleanup_action(cmd, self.cleanup_path(i));
        } else {
            self.cleanup.keyboard = true;
            let row = self.cleanup.row_focus;
            if let (Some(&y), Some(&b)) = (self.cleanup.offsets.get(row), self.cleanup.offsets.get(row + 1)) {
                let c = crate::paint::content_rect(self);
                let above_pill = c.w < 900.0 && (self.cleanup.done.is_none() || self.cleanup.totals(true).count > 0);
                let h = (c.h - 156.0 - if above_pill { 70.0 } else { 0.0 }).max(0.0);
                let current = self.cleanup.panel_scroll.target;
                self.cleanup.panel_scroll.to(reveal(current, h, y, b).clamp(0.0, self.cleanup.panel_max));
            }
        }
    }
}

fn reveal(scroll: f32, height: f32, top: f32, bottom: f32) -> f32 {
    if top < scroll { top } else if bottom > scroll + height { bottom - height } else { scroll }
}

/// Explorer's name of a drive ("Main (D:)", "USB Drive (E:)", a share's name), the same as everywhere else.
fn drive_name(d: &fmcore::shell::Drive) -> String {
    crate::drives::drive_name(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: GroupId, rows: &[(u64, &str)]) -> Group {
        Group { id, title: "", reason: "", limited: false, total_size: rows.iter().map(|r| r.0).sum(),
            items: rows.iter().map(|(size, path)| Item { path: (*path).into(), size: *size, modified: 0, reason: "", copy_of: None }).collect() }
    }

    #[test]
    fn duplicates_start_unselected() {
        let mut page = Page::default();
        for (g, size) in [(GroupId::Temp, 10), (GroupId::AppCaches, 20), (GroupId::CrashDumps, 3),
            (GroupId::BuildCaches, 40), (GroupId::DuplicateFiles, 50), (GroupId::Installers, 100), (GroupId::LargeFiles, 300)]
        {
            page.receive_group(candidate(g, &[(size, g.as_str())]));
        }
        assert_eq!(page.totals(false), Total { bytes: 523, count: 7 });
        assert_eq!(page.totals(true), Total { bytes: 73, count: 4 });
        assert_eq!(page.tiles[1].total.count, 2);
        page.panel = Some(1);
        page.toggle_row(0);
        assert_eq!(page.totals(true), Total { bytes: 53, count: 3 });
        page.toggle_group(4);
        assert_eq!(page.selected_snapshot().items.len(), 4);
        assert_eq!(page.totals(true), Total { bytes: 153, count: 4 });
        page.toggle_group(3);
        assert_eq!(page.totals(true), Total { bytes: 203, count: 5 });
    }

    #[test]
    fn developer_cache_defaults_select_only_build_output() {
        let mut page = Page::default();
        let mut group = candidate(GroupId::BuildCaches, &[(8, r"C:\project\target"), (9, r"D:\uv\package.whl")]);
        group.items[1].reason = "Downloaded package; fetched again when needed";
        page.receive_group(group);
        assert!(page.tiles[2].rows[0].on);
        assert!(!page.tiles[2].rows[1].on);
        assert_eq!(page.tiles[2].selected, Total { bytes: 8, count: 1 });
        assert_eq!(page.by_drive().get(&'D'), Some(&Total::default()));
    }

    #[test]
    fn work_id_rejects_late_results_after_cancel() {
        let mut page = Page::default();
        let scan = page.next();
        let token = Arc::new(AtomicBool::new(false));
        page.cancel = Some(token.clone());
        let review = page.next();
        assert!(token.load(Ordering::Relaxed));
        assert!(!page.accepts(scan));
        assert!(page.accepts(review));
    }

    #[test]
    fn review_batches_run_in_parallel_and_retry_quota_at_lower_limits() {
        let cancel = ReviewCancel::default();
        let (started, attempts) = mpsc::channel();
        let (posted, updates) = mpsc::channel();
        let timeout = Duration::from_secs(3);
        let advice = Classification { verdict: Verdict::Safe, reason: "Test reason".into() };
        std::thread::scope(|scope| {
            scope.spawn(|| review_batches(4, &cancel, Duration::from_millis(40), |batch| {
                let (answer, reply) = mpsc::channel::<Result<(), String>>();
                started.send((batch, answer)).unwrap();
                reply.recv_timeout(timeout).map_err(|e| e.to_string())?
                    .map(|()| vec![(0, batch, advice.clone())])
            }, |update| posted.send(update).unwrap()));
            let take_attempts = |count| (0..count).map(|_| attempts.recv_timeout(timeout).unwrap()).collect::<BTreeMap<_, _>>();
            let expect_rated = |batch| {
                assert!(matches!(updates.recv_timeout(timeout).unwrap(), Update::Rated(rows) if rows == [(0, batch, advice.clone())]));
            };
            let first = take_attempts(4);
            assert_eq!(first.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3]);
            first[&3].send(Ok(())).unwrap();
            expect_rated(3);
            first[&2].send(Ok(())).unwrap();
            expect_rated(2);
            let quota = || Err(fmcore::ai::FREE_QUOTA.to_string());
            first[&0].send(quota()).unwrap();
            first[&1].send(quota()).unwrap();
            let second = take_attempts(2);
            assert_eq!(second.keys().copied().collect::<Vec<_>>(), [0, 1]);
            second[&0].send(quota()).unwrap();
            // The next retry must wait until the other request at the two-batch limit returns.
            assert!(matches!(attempts.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)));
            second[&1].send(Ok(())).unwrap();
            expect_rated(1);
            let (batch, answer) = attempts.recv_timeout(timeout).unwrap();
            assert_eq!(batch, 0);
            answer.send(quota()).unwrap();
            assert!(matches!(updates.recv_timeout(timeout).unwrap(), Update::Reviewed(Err(e)) if e == fmcore::ai::FREE_QUOTA));
        });
        assert!(updates.try_recv().is_err());
        assert!(attempts.try_recv().is_err());
    }

    #[test]
    fn cancelled_review_stops_workers_without_posting_late_results() {
        let cancel = ReviewCancel::default();
        let (started, attempts) = mpsc::channel();
        let (posted, updates) = mpsc::channel();
        let timeout = Duration::from_secs(3);
        std::thread::scope(|scope| {
            scope.spawn(|| review_batches(8, &cancel, Duration::from_secs(2), |batch| {
                started.send(batch).unwrap();
                let start = Instant::now();
                while !cancel.stopped() && start.elapsed() < timeout {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(vec![])
            }, |update| posted.send(update).unwrap()));
            let mut batches: Vec<_> = (0..4).map(|_| attempts.recv_timeout(timeout).unwrap()).collect();
            batches.sort_unstable();
            assert_eq!(batches, [0, 1, 2, 3]);
            cancel.stop();
        });
        assert!(updates.try_recv().is_err());
        assert!(attempts.try_recv().is_err());
    }

    #[test]
    fn review_stops_and_reports_a_non_quota_error_once() {
        let cancel = ReviewCancel::default();
        let mut updates = vec![];
        review_batches(8, &cancel, Duration::ZERO, |_| Err("fake error".into()), |update| updates.push(update));
        assert!(matches!(&updates[..], [Update::Reviewed(Err(e))] if e == "fake error"));
    }

    #[test]
    fn review_ticks_safe_rows_and_manual_ticks_carry_warnings() {
        let mut page = Page::default();
        page.receive_group(candidate(GroupId::LargeFiles, &[(8, r"C:\large.zip"), (9, r"D:\backup.zip"), (6, r"D:\old video.mp4")]));
        assert_eq!(page.by_drive(), BTreeMap::from([('C', Total::default()), ('D', Total::default())]));
        let advice = |verdict| Classification { verdict, reason: "Test reason".into() };
        page.rate(vec![(5, 0, advice(Verdict::Safe)), (5, 1, advice(Verdict::Keep)), (5, 2, advice(Verdict::CheckFirst))]);
        assert_eq!(page.totals(true), Total { bytes: 8, count: 1 });
        assert_eq!(page.cache.len(), 3);
        page.panel = Some(5);
        page.show_notes = true;
        page.sort_panel();
        assert_eq!(page.order, [2, 1, 0]);
        page.toggle_row(2);
        let picked = page.selected_snapshot();
        assert_eq!(picked.flagged[0], ["old video.mp4"]);
        assert!(picked.flagged[1].is_empty());
        assert_eq!(picked.by_group[5], Total { bytes: 14, count: 2 });
        assert_eq!(picked.by_drive, BTreeMap::from([
            ('C', Total { bytes: 8, count: 1 }), ('D', Total { bytes: 6, count: 1 }),
        ]));
    }

    #[test]
    fn empty_bin_requires_confirmation() {
        let mut calls = 0;
        let mut empty = || { calls += 1; Ok(()) };
        assert!(empty_confirmed(false, &mut empty).is_err());
        assert!(empty_confirmed(true, &mut empty).is_ok());
        assert_eq!(calls, 1);
    }
}
