//! File operations: copy / move (with progress, cancel, pause and conflict choices), rename, batch rename, new
//! folder, new file, delete (Recycle Bin by default). Every change is written to the undo journal.
//!
//! Copy uses CopyFileExW (its progress callback feeds `Progress`); a move inside one volume is a rename
//! (MoveFileExW, instant); a move to another drive copies with progress and then removes the source.
use crate::journal::{self, Op};
use crate::rx::Regex;
use crate::util::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{ERROR_NOT_SAME_DEVICE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, COPY_FILE_NO_BUFFERING, COPYFILE_FLAGS, COPYPROGRESSROUTINE_PROGRESS, CopyFileExW, LPPROGRESS_ROUTINE_CALLBACK_REASON,
    MOVE_FILE_FLAGS, MOVEFILE_COPY_ALLOWED, MoveFileExW, MoveFileWithProgressW, PROGRESS_CANCEL, PROGRESS_CONTINUE,
};
use windows::core::PCWSTR;

/// What to do when the target name is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// Leave the existing item, skip this one.
    Skip,
    /// Send the existing item to the Recycle Bin, then write (undo brings it back).
    Replace,
    /// Write next to it as "name (2).ext".
    KeepBoth,
    /// Stop with an error for this item.
    Fail,
}

impl Conflict {
    pub fn parse(s: &str) -> Option<Conflict> {
        Some(match s {
            "skip" => Conflict::Skip,
            "replace" | "overwrite" => Conflict::Replace,
            "keep-both" | "keepboth" | "rename" => Conflict::KeepBoth,
            "fail" | "error" => Conflict::Fail,
            _ => return None,
        })
    }
}

/// Live progress of a copy or move.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub current: PathBuf,
    pub elapsed_ms: u64,
    /// Bytes per second since the start.
    pub speed: f64,
    /// Seconds left (None until the speed is known).
    pub eta_s: Option<u64>,
    /// True on the last event.
    pub finished: bool,
}

/// Cancel and pause switches a GUI can flip from another thread while an operation runs.
#[derive(Clone, Default)]
pub struct Control {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

impl Control {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Result of a copy / move.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// (source, final target) of each top-level item that was done.
    pub done: Vec<(PathBuf, PathBuf)>,
    pub skipped: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub bytes: u64,
    pub files: u64,
    pub cancelled: bool,
    /// Undo journal id (0 = nothing recorded).
    pub journal_id: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Copy,
    Move,
}

struct Ctx<'a> {
    mode: Mode,
    conflict: Conflict,
    ctl: &'a Control,
    on: &'a mut dyn FnMut(&Progress),
    p: Progress,
    t0: Instant,
    last: Instant,
    op: Op,
    rep: Report,
    file_base: u64,
}

struct Cancelled;

impl Ctx<'_> {
    fn emit(&mut self, force: bool) {
        let now = Instant::now();
        if !force && now.duration_since(self.last) < Duration::from_millis(100) {
            return;
        }
        self.last = now;
        let el = now.duration_since(self.t0);
        self.p.elapsed_ms = el.as_millis() as u64;
        let secs = el.as_secs_f64();
        self.p.speed = if secs > 0.05 { self.p.bytes_done as f64 / secs } else { 0.0 };
        self.p.eta_s = if self.p.speed > 0.0 && self.p.bytes_total >= self.p.bytes_done { Some(((self.p.bytes_total - self.p.bytes_done) as f64 / self.p.speed) as u64) } else { None };
        (self.on)(&self.p);
    }

    fn wait_if_paused(&self) {
        while self.ctl.pause.load(Ordering::Relaxed) && !self.ctl.cancelled() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn err(&mut self, e: String) {
        self.rep.errors.push(e);
    }

    /// Get `dst` out of the way (Replace): Recycle Bin when the drive has one, else delete. Recorded for undo.
    fn clear_target(&mut self, dst: &Path) -> Result<(), String> {
        if crate::recycle::has_recycle_bin(dst) {
            let r = crate::recycle::recycle(&[dst.to_path_buf()])?;
            self.op.recycled.extend(r);
            Ok(())
        } else {
            crate::recycle::delete_permanently(dst)
        }
    }

    fn item(&mut self, src: &Path, dst: &Path, record: bool) -> Result<(), Cancelled> {
        if self.ctl.cancelled() {
            return Err(Cancelled);
        }
        self.wait_if_paused();
        let Ok(smd) = std::fs::metadata(src) else {
            self.err(format!("not found: {}", src.display()));
            return Ok(());
        };
        let src_dir = smd.is_dir();
        let mut dst = dst.to_path_buf();
        // A move that only changes letter case ("a.txt" -> "A.txt") is a plain rename, not a conflict.
        let case_only = self.mode == Mode::Move && path_eq(src, &dst) && src.as_os_str() != dst.as_os_str();
        if journal::exists(&dst) && !case_only {
            let dst_dir = dst.is_dir();
            if path_eq(src, &dst) {
                if self.conflict == Conflict::KeepBoth {
                    dst = unique_name(&dst);
                } else {
                    self.skip(src, &smd);
                    return Ok(());
                }
            } else if src_dir && dst_dir && self.conflict != Conflict::KeepBoth {
                return self.merge(src, &dst);
            } else {
                match self.conflict {
                    Conflict::Skip => {
                        self.skip(src, &smd);
                        return Ok(());
                    }
                    Conflict::KeepBoth => dst = unique_name(&dst),
                    Conflict::Replace => {
                        if let Err(e) = self.clear_target(&dst) {
                            self.err(e);
                            return Ok(());
                        }
                    }
                    Conflict::Fail => {
                        self.err(format!("already exists: {}", dst.display()));
                        return Ok(());
                    }
                }
            }
        }
        if src_dir && !case_only && is_inside(&dst, src) {
            self.err(format!("cannot put a folder inside itself: {}", src.display()));
            return Ok(());
        }
        self.p.current = src.to_path_buf();
        match self.mode {
            Mode::Move => {
                let (a, b) = (wide_path(src), wide_path(&dst));
                // Same volume: a rename, instant for files and whole folders.
                match unsafe { MoveFileExW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr()), MOVE_FILE_FLAGS(0)) } {
                    Ok(()) => {
                        if record {
                            self.op.moved.push((src.to_path_buf(), dst.clone()));
                        }
                        let (bytes, files) = if src_dir { (0, 1) } else { (smd.len(), 1) };
                        self.p.bytes_done += bytes;
                        self.p.files_done += files;
                        self.rep.bytes += bytes;
                        self.rep.files += files;
                        self.emit(false);
                        return Ok(());
                    }
                    Err(e) if e.code() == ERROR_NOT_SAME_DEVICE.to_hresult() => {}
                    Err(e) => {
                        self.err(win_err(&format!("cannot move {}", src.display()), &e));
                        return Ok(());
                    }
                }
                // Another volume.
                if src_dir {
                    if let Err(e) = std::fs::create_dir(&dst) {
                        self.err(format!("cannot create {}: {e}", dst.display()));
                        return Ok(());
                    }
                    if record {
                        self.op.moved.push((src.to_path_buf(), dst.clone()));
                    }
                    self.children(src, &dst, false)?;
                    let _ = std::fs::remove_dir(src);
                } else if self.file_transfer(src, &dst, smd.len())? {
                    if record {
                        self.op.moved.push((src.to_path_buf(), dst.clone()));
                    }
                }
            }
            Mode::Copy => {
                if src_dir {
                    if let Err(e) = std::fs::create_dir(&dst) {
                        self.err(format!("cannot create {}: {e}", dst.display()));
                        return Ok(());
                    }
                    if record {
                        self.op.created.push(dst.clone());
                    }
                    self.children(src, &dst, false)?;
                } else if self.file_transfer(src, &dst, smd.len())? && record {
                    self.op.created.push(dst.clone());
                }
            }
        }
        Ok(())
    }

    fn skip(&mut self, src: &Path, md: &std::fs::Metadata) {
        self.rep.skipped.push(src.to_path_buf());
        // Count skipped bytes as done so the bar still reaches 100%.
        let (b, f) = if md.is_dir() { tree_size(src) } else { (md.len(), 1) };
        self.p.bytes_done += b;
        self.p.files_done += f;
        self.emit(false);
    }

    /// Both are folders: put the children of `src` into the existing `dst`.
    fn merge(&mut self, src: &Path, dst: &Path) -> Result<(), Cancelled> {
        self.children(src, dst, true)?;
        if self.mode == Mode::Move {
            let _ = std::fs::remove_dir(src); // only succeeds when everything moved
        }
        Ok(())
    }

    fn children(&mut self, src: &Path, dst: &Path, record: bool) -> Result<(), Cancelled> {
        match crate::list::list_dir(src, true) {
            Ok(list) => {
                for e in list {
                    self.item(&src.join(&e.name), &dst.join(&e.name), record)?;
                }
            }
            Err(e) => self.err(e),
        }
        Ok(())
    }

    /// Copy (or cross-volume move) one file with progress. Ok(true) when done.
    fn file_transfer(&mut self, src: &Path, dst: &Path, size: u64) -> Result<bool, Cancelled> {
        let (a, b) = (wide_path(src), wide_path(dst));
        self.file_base = self.p.bytes_done;
        let data = self as *mut Ctx as *const core::ffi::c_void;
        let r = unsafe {
            match self.mode {
                Mode::Copy => {
                    // Unbuffered I/O is faster for big files and keeps the cache clean (Explorer does the same).
                    let flags = if size > 256 * 1024 * 1024 { COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_NO_BUFFERING } else { COPY_FILE_FAIL_IF_EXISTS };
                    CopyFileExW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr()), Some(progress_cb), Some(data), None, COPYFILE_FLAGS(flags.0))
                }
                Mode::Move => MoveFileWithProgressW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr()), Some(progress_cb), Some(data), MOVEFILE_COPY_ALLOWED),
            }
        };
        match r {
            Ok(()) => {
                self.p.bytes_done = self.file_base + size;
                self.p.files_done += 1;
                self.rep.bytes += size;
                self.rep.files += 1;
                self.emit(false);
                Ok(true)
            }
            Err(e) => {
                if self.ctl.cancelled() {
                    return Err(Cancelled);
                }
                self.err(win_err(&format!("cannot {} {}", if self.mode == Mode::Copy { "copy" } else { "move" }, src.display()), &e));
                Ok(false)
            }
        }
    }
}

unsafe extern "system" fn progress_cb(
    _total: i64,
    done: i64,
    _ssize: i64,
    _sdone: i64,
    _stream: u32,
    _reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _hs: HANDLE,
    _hd: HANDLE,
    data: *const core::ffi::c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    // SAFETY: `data` is the &mut Ctx of the running transfer on this same thread (CopyFileExW is synchronous).
    let ctx = unsafe { &mut *(data as *mut Ctx) };
    ctx.p.bytes_done = ctx.file_base + done as u64;
    ctx.emit(false);
    ctx.wait_if_paused();
    if ctx.ctl.cancelled() { PROGRESS_CANCEL } else { PROGRESS_CONTINUE }
}

/// Is `p` equal to `dir` or below it?
pub fn is_inside(p: &Path, dir: &Path) -> bool {
    let a = p.to_string_lossy().to_lowercase();
    let b = dir.to_string_lossy().to_lowercase();
    a == b || a.starts_with(&format!("{}\\", b.trim_end_matches('\\')))
}

/// "name (2).ext", "name (3).ext", ... : the first free name next to `p`.
pub fn unique_name(p: &Path) -> PathBuf {
    let parent = p.parent().unwrap_or(Path::new(""));
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let is_dir = p.is_dir();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !is_dir => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.clone(), String::new()),
    };
    for n in 2.. {
        let c = parent.join(format!("{stem} ({n}){ext}"));
        if !journal::exists(&c) {
            return c;
        }
    }
    unreachable!()
}

/// Total bytes and files below a path (a file counts as itself).
pub fn tree_size(p: &Path) -> (u64, u64) {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => {
            let f = crate::info::folder_size(p, false, Duration::from_secs(3600));
            (f.bytes, f.files)
        }
        Ok(m) => (m.len(), 1),
        Err(_) => (0, 0),
    }
}

/// Work out the target of each source: inside `dest` when it is a folder; `dest` itself for a single source.
fn targets(srcs: &[PathBuf], dest: &Path) -> Res<Vec<(PathBuf, PathBuf)>> {
    if srcs.is_empty() {
        return Err("nothing to do".into());
    }
    let dest = abs(dest);
    let into = dest.is_dir() || srcs.len() > 1;
    if into && !dest.exists() {
        std::fs::create_dir_all(&dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    }
    if !into {
        if let Some(parent) = dest.parent() {
            if !parent.exists() {
                return Err(format!("folder not found: {}", parent.display()));
            }
        }
    }
    let mut out = vec![];
    for s in srcs {
        let s = abs(s);
        if !journal::exists(&s) {
            return Err(format!("not found: {}", s.display()));
        }
        let t = if into {
            let name = s.file_name().ok_or_else(|| format!("cannot copy a drive root: {}", s.display()))?;
            dest.join(name)
        } else {
            dest.clone()
        };
        out.push((s, t));
    }
    Ok(out)
}

fn run(mode: Mode, srcs: &[PathBuf], dest: &Path, conflict: Conflict, ctl: &Control, on: &mut dyn FnMut(&Progress), journal_kind: Option<&str>) -> Res<Report> {
    let pairs = targets(srcs, dest)?;
    let mut ctx = Ctx {
        mode,
        conflict,
        ctl,
        on,
        p: Progress::default(),
        t0: Instant::now(),
        last: Instant::now() - Duration::from_secs(1),
        op: Op::new(journal_kind.unwrap_or("")),
        rep: Report::default(),
        file_base: 0,
    };
    // Totals: needed when data really moves (copy, or move to another drive).
    for (s, t) in &pairs {
        let parent = t.parent().unwrap_or(t);
        if mode == Mode::Copy || !same_volume(s, parent) {
            let (b, f) = tree_size(s);
            ctx.p.bytes_total += b;
            ctx.p.files_total += f;
        } else {
            ctx.p.files_total += 1;
            if let Ok(m) = std::fs::metadata(s) {
                if !m.is_dir() {
                    ctx.p.bytes_total += m.len();
                }
            }
        }
    }
    ctx.emit(true);
    for (s, t) in &pairs {
        let before_err = ctx.rep.errors.len();
        let before_skip = ctx.rep.skipped.len();
        match ctx.item(s, t, true) {
            Ok(()) => {}
            Err(Cancelled) => {
                ctx.rep.cancelled = true;
                break;
            }
        }
        if ctx.rep.errors.len() == before_err && ctx.rep.skipped.len() == before_skip {
            // Report the real target (keep-both may have picked another name).
            let real = ctx
                .op
                .created
                .iter()
                .rev()
                .find(|c| c.parent() == t.parent())
                .cloned()
                .or_else(|| ctx.op.moved.iter().rev().find(|m| path_eq(&m.0, s)).map(|m| m.1.clone()))
                .unwrap_or_else(|| t.clone());
            ctx.rep.done.push((s.clone(), real));
        }
    }
    ctx.p.finished = true;
    ctx.p.current = PathBuf::new();
    ctx.emit(true);
    let mut rep = std::mem::take(&mut ctx.rep);
    if journal_kind.is_some() {
        rep.journal_id = journal::record_quiet(std::mem::take(&mut ctx.op));
    }
    Ok(rep)
}

/// Copy files and folders into `dest` (a folder), or to `dest` itself for one source. Progress events arrive at
/// most every 100 ms plus a final one. Recorded for undo.
pub fn copy(srcs: &[PathBuf], dest: &Path, conflict: Conflict, ctl: &Control, on: &mut dyn FnMut(&Progress)) -> Res<Report> {
    run(Mode::Copy, srcs, dest, conflict, ctl, on, Some("copy"))
}

/// Move files and folders (rename within a volume, copy + remove across volumes). Recorded for undo.
pub fn move_items(srcs: &[PathBuf], dest: &Path, conflict: Conflict, ctl: &Control, on: &mut dyn FnMut(&Progress)) -> Res<Report> {
    run(Mode::Move, srcs, dest, conflict, ctl, on, Some("move"))
}

/// Move `from` back to `to` without journaling (used by undo). Folders merge back into an existing folder.
pub fn move_back(from: &Path, to: &Path) -> Res<()> {
    if !journal::exists(from) {
        return Err(format!("cannot undo: {} is gone", from.display()));
    }
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    }
    let ctl = Control::default();
    let mut noop = |_: &Progress| {};
    let rep = run(Mode::Move, &[from.to_path_buf()], to, Conflict::Fail, &ctl, &mut noop, None)?;
    if rep.errors.is_empty() { Ok(()) } else { Err(rep.errors.join("; ")) }
}

/// Characters Windows does not allow in a name.
pub fn check_name(name: &str) -> Res<String> {
    let n = name.trim_end_matches([' ', '.']).trim_start();
    if n.is_empty() || n == "." || n == ".." {
        return Err(format!("not a valid name: '{name}'"));
    }
    if let Some(c) = n.chars().find(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (*c as u32) < 32) {
        return Err(format!("a name cannot contain '{c}': '{name}'"));
    }
    let stem = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    if reserved.contains(&stem.as_str()) {
        return Err(format!("'{name}' is a reserved name in Windows"));
    }
    Ok(n.to_string())
}

fn raw_rename(a: &Path, b: &Path) -> Res<()> {
    let (wa, wb) = (wide_path(a), wide_path(b));
    unsafe { MoveFileExW(PCWSTR(wa.as_ptr()), PCWSTR(wb.as_ptr()), MOVE_FILE_FLAGS(0)) }.map_err(|e| win_err(&format!("cannot rename {}", a.display()), &e))
}

/// Rename one item inside its folder. Returns the new path. Recorded for undo.
pub fn rename(path: &Path, new_name: &str) -> Res<PathBuf> {
    let path = abs(path);
    if !journal::exists(&path) {
        return Err(format!("not found: {}", path.display()));
    }
    let name = check_name(new_name)?;
    let target = path.parent().ok_or("cannot rename a drive")?.join(&name);
    if journal::exists(&target) && !path_eq(&target, &path) {
        return Err(format!("already exists: {}", target.display()));
    }
    raw_rename(&path, &target)?;
    let mut op = Op::new("rename");
    op.moved.push((path, target.clone()));
    journal::record_quiet(op);
    Ok(target)
}

/// Options for batch rename.
#[derive(Clone, Debug, Default)]
pub struct BatchOpts {
    /// Replace every match in a name, not only the first.
    pub all: bool,
    /// Include folders (files only by default).
    pub folders: bool,
    /// Include hidden items.
    pub hidden: bool,
    /// Match only the name without its extension (the extension is kept).
    pub stem_only: bool,
}

/// A checked batch rename: what would change and any problems (apply refuses while there are problems).
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub items: Vec<(PathBuf, PathBuf)>,
    pub unchanged: usize,
    pub problems: Vec<String>,
}

/// Build the new name for each item in `dir` whose name matches `pattern`. The matched part is replaced by
/// `template`, which may use $0..$9 (groups), {n} / {n:3} (counter, zero padded), {name}, {ext}, {date}, {parent}.
pub fn batch_plan(dir: &Path, pattern: &str, template: &str, o: &BatchOpts) -> Res<Plan> {
    let dir = abs(dir);
    let re = Regex::new(pattern)?;
    let mut list = crate::list::list_dir(&dir, o.hidden)?;
    list.retain(|e| o.folders || !e.is_dir());
    crate::list::sort_entries(&mut list, crate::list::SortKey::Name, false, false);
    let mut plan = Plan::default();
    let mut counter = 0u64;
    let parent_name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for e in &list {
        let (subject, keep_ext) = if o.stem_only && !e.is_dir() {
            match e.name.rfind('.') {
                Some(i) if i > 0 => (e.name[..i].to_string(), e.name[i..].to_string()),
                _ => (e.name.clone(), String::new()),
            }
        } else {
            (e.name.clone(), String::new())
        };
        if re.find(&subject, 0).is_none() {
            plan.unchanged += 1;
            continue;
        }
        counter += 1;
        let stem = match e.name.rfind('.') {
            Some(i) if i > 0 && !e.is_dir() => e.name[..i].to_string(),
            _ => e.name.clone(),
        };
        let vars = TemplateVars { n: counter, name: &stem, ext: &e.ext(), date: &fmt_time(e.modified).chars().take(10).collect::<String>(), parent: &parent_name };
        let replaced = re.replace(&subject, |caps| expand(template, caps, &vars), o.all);
        let new_name = format!("{replaced}{keep_ext}");
        if new_name == e.name {
            plan.unchanged += 1;
            continue;
        }
        match check_name(&new_name) {
            Ok(n) => plan.items.push((dir.join(&e.name), dir.join(n))),
            Err(err) => plan.problems.push(format!("{}: {err}", e.name)),
        }
    }
    // Collisions: two items to one name, or a name taken by an item outside the batch.
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    let sources: std::collections::HashSet<String> = plan.items.iter().map(|(a, _)| lower(a)).collect();
    let mut seen = std::collections::HashSet::new();
    for (a, b) in &plan.items {
        let lb = lower(b);
        if !seen.insert(lb.clone()) {
            plan.problems.push(format!("two items would be named {}", b.file_name().unwrap().to_string_lossy()));
        } else if journal::exists(b) && !sources.contains(&lb) && lower(a) != lb {
            plan.problems.push(format!("{} already exists", b.file_name().unwrap().to_string_lossy()));
        }
    }
    Ok(plan)
}

struct TemplateVars<'a> {
    n: u64,
    name: &'a str,
    ext: &'a str,
    date: &'a str,
    parent: &'a str,
}

fn expand(t: &str, caps: &[Option<String>], v: &TemplateVars) -> String {
    let c: Vec<char> = t.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '$' && i + 1 < c.len() {
            if c[i + 1] == '$' {
                out.push('$');
                i += 2;
                continue;
            }
            if let Some(d) = c[i + 1].to_digit(10) {
                out.push_str(caps.get(d as usize).cloned().flatten().unwrap_or_default().as_str());
                i += 2;
                continue;
            }
            if c[i + 1] == '{' {
                if let Some(end) = c[i + 2..].iter().position(|&x| x == '}') {
                    let num: String = c[i + 2..i + 2 + end].iter().collect();
                    if let Ok(d) = num.parse::<usize>() {
                        out.push_str(caps.get(d).cloned().flatten().unwrap_or_default().as_str());
                        i += 3 + end;
                        continue;
                    }
                }
            }
        }
        if c[i] == '{' {
            if let Some(end) = c[i + 1..].iter().position(|&x| x == '}') {
                let tok: String = c[i + 1..i + 1 + end].iter().collect();
                let rep = match tok.as_str() {
                    "name" => Some(v.name.to_string()),
                    "ext" => Some(v.ext.to_string()),
                    "date" => Some(v.date.to_string()),
                    "parent" => Some(v.parent.to_string()),
                    "n" => Some(v.n.to_string()),
                    _ => tok.strip_prefix("n:").and_then(|w| w.parse::<usize>().ok()).map(|w| format!("{:0w$}", v.n)),
                };
                if let Some(r) = rep {
                    out.push_str(&r);
                    i += 2 + end;
                    continue;
                }
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

/// Rename many items at once. Safe for swaps and chains (a->b, b->c): it goes through temporary names when needed.
pub fn rename_pairs(pairs: &[(PathBuf, PathBuf)]) -> Res<()> {
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    let sources: std::collections::HashSet<String> = pairs.iter().map(|(a, _)| lower(a)).collect();
    let chained = pairs.iter().any(|(a, b)| sources.contains(&lower(b)) && lower(a) != lower(b));
    if !chained {
        let mut done: Vec<(PathBuf, PathBuf)> = vec![];
        for (a, b) in pairs {
            if let Err(e) = raw_rename(a, b) {
                for (x, y) in done.iter().rev() {
                    let _ = raw_rename(y, x);
                }
                return Err(e);
            }
            done.push((a.clone(), b.clone()));
        }
        return Ok(());
    }
    let pid = std::process::id();
    let mut temps: Vec<(PathBuf, PathBuf)> = vec![];
    for (i, (a, _)) in pairs.iter().enumerate() {
        let t = a.with_file_name(format!("~wfm{pid}_{i}.tmp"));
        if let Err(e) = raw_rename(a, &t) {
            for (x, y) in temps.iter().rev() {
                let _ = raw_rename(y, x);
            }
            return Err(e);
        }
        temps.push((a.clone(), t));
    }
    let mut errs = vec![];
    for (i, (_, b)) in pairs.iter().enumerate() {
        if let Err(e) = raw_rename(&temps[i].1, b) {
            let _ = raw_rename(&temps[i].1, &temps[i].0);
            errs.push(e);
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs.join("; ")) }
}

/// Apply a plan made by `batch_plan`. Recorded for undo as one step.
pub fn batch_apply(plan: &Plan) -> Res<u64> {
    if !plan.problems.is_empty() {
        return Err(format!("batch rename has problems: {}", plan.problems.join("; ")));
    }
    if plan.items.is_empty() {
        return Ok(0);
    }
    rename_pairs(&plan.items)?;
    let mut op = Op::new("batch-rename");
    op.moved = plan.items.clone();
    Ok(journal::record_quiet(op))
}

/// New folder (and missing parents). Recorded for undo.
pub fn mkdir(path: &Path) -> Res<PathBuf> {
    let path = abs(path);
    if journal::exists(&path) {
        return Err(format!("already exists: {}", path.display()));
    }
    if let Some(n) = path.file_name() {
        check_name(&n.to_string_lossy())?;
    }
    // The top-most folder that does not exist yet is what undo removes.
    let mut top = path.clone();
    while let Some(p) = top.parent() {
        if journal::exists(p) {
            break;
        }
        top = p.to_path_buf();
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let mut op = Op::new("mkdir");
    op.created.push(top);
    journal::record_quiet(op);
    Ok(path)
}

/// New empty file (fails when the name is taken). Recorded for undo.
pub fn new_file(path: &Path) -> Res<PathBuf> {
    let path = abs(path);
    if let Some(n) = path.file_name() {
        check_name(&n.to_string_lossy())?;
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists { format!("already exists: {}", path.display()) } else { format!("cannot create {}: {e}", path.display()) }
    })?;
    let mut op = Op::new("new-file");
    op.created.push(path.clone());
    journal::record_quiet(op);
    Ok(path)
}

/// Delete items: to the Recycle Bin (recorded for undo), or for good with `permanent`.
pub fn delete(paths: &[PathBuf], permanent: bool) -> Res<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = paths.iter().map(abs).collect();
    for p in &paths {
        if !journal::exists(p) {
            return Err(format!("not found: {}", p.display()));
        }
        if p.parent().is_none() {
            return Err(format!("refusing to delete a drive root: {}", p.display()));
        }
    }
    if permanent {
        for p in &paths {
            crate::recycle::delete_permanently(p)?;
        }
        return Ok(paths);
    }
    let rec = crate::recycle::recycle(&paths)?;
    let mut op = Op::new("delete");
    op.recycled = rec;
    journal::record_quiet(op);
    Ok(paths)
}
