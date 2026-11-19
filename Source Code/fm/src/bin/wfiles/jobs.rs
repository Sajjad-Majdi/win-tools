//! The copy queue: every copy and move goes through one worker, in order. Before a job runs, its top-level targets
//! are checked: identical files (same size and time) are skipped without a question, the rest of the clashes
//! wait inside the job (Skip / Replace / Keep both) while every other item is copied first.
use fmcore::ops::{self, Conflict, Control, Progress};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Copy,
    Move,
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Waiting,
    Running,
    /// clashes need an answer (the other items are done)
    Asking,
    Done,
    Failed(String),
    Cancelled,
    Undone,
}

#[derive(Clone, Debug)]
pub struct Clash {
    pub src: PathBuf,
    pub target: PathBuf,
    pub src_size: u64,
    pub src_time: u64,
    pub dst_size: u64,
    pub dst_time: u64,
    pub choice: Option<Conflict>,
}

#[derive(Clone)]
pub struct Job {
    pub id: u64,
    pub kind: Kind,
    pub srcs: Vec<PathBuf>,
    pub dest: PathBuf,
    pub state: State,
    pub done: u64,
    pub total: u64,
    pub files_done: u64,
    pub files_total: u64,
    pub speed: f64,
    pub eta: Option<u64>,
    pub ctl: Control,
    pub clashes: Vec<Clash>,
    pub same_for_rest: bool,
    pub journal: Vec<u64>,
    pub identical: usize,
    pub errors: Vec<String>,
    pub results: Vec<PathBuf>,
    pub started: Option<Instant>,
}

impl Job {
    pub fn title(&self) -> String {
        let n = self.srcs.len();
        let verb = match (self.kind, &self.state) {
            (Kind::Copy, State::Done | State::Undone) => "Copied",
            (Kind::Move, State::Done | State::Undone) => "Moved",
            (Kind::Copy, _) => "Copying",
            (Kind::Move, _) => "Moving",
        };
        format!("{verb} {n} item{}", if n == 1 { "" } else { "s" })
    }
    pub fn paused(&self) -> bool {
        self.ctl.pause.load(Relaxed)
    }
    pub fn active(&self) -> bool {
        matches!(self.state, State::Waiting | State::Running | State::Asking)
    }
    pub fn open_clashes(&self) -> usize {
        self.clashes.iter().filter(|c| c.choice.is_none()).count()
    }
}

#[derive(Default)]
pub struct Queue {
    pub jobs: Vec<Job>,
    next: u64,
}

pub type Shared = Arc<(Mutex<Queue>, Condvar)>;

pub fn new_queue() -> Shared {
    Arc::new((Mutex::new(Queue::default()), Condvar::new()))
}

/// Add a job; `notify` is called (from the worker) on every change so the window repaints.
pub fn add(q: &Shared, kind: Kind, srcs: Vec<PathBuf>, dest: PathBuf) -> u64 {
    let (m, cv) = &**q;
    let mut g = m.lock().unwrap();
    g.next += 1;
    let id = g.next;
    g.jobs.push(Job {
        id,
        kind,
        srcs,
        dest,
        state: State::Waiting,
        done: 0,
        total: 0,
        files_done: 0,
        files_total: 0,
        speed: 0.0,
        eta: None,
        ctl: Control::default(),
        clashes: vec![],
        same_for_rest: true,
        journal: vec![],
        identical: 0,
        errors: vec![],
        results: vec![],
        started: None,
    });
    cv.notify_all();
    id
}

/// Answer a clash. `all` = use the same choice for the other open clashes of the job.
pub fn decide(q: &Shared, job: u64, idx: usize, choice: Conflict, all: bool) {
    let (m, cv) = &**q;
    let mut g = m.lock().unwrap();
    if let Some(j) = g.jobs.iter_mut().find(|j| j.id == job) {
        if all {
            for c in j.clashes.iter_mut().filter(|c| c.choice.is_none()) {
                c.choice = Some(choice);
            }
        } else if let Some(c) = j.clashes.iter_mut().filter(|c| c.choice.is_none()).nth(idx) {
            c.choice = Some(choice);
        }
    }
    cv.notify_all();
}

pub fn cancel(q: &Shared, job: u64) {
    let (m, cv) = &**q;
    let mut g = m.lock().unwrap();
    if let Some(j) = g.jobs.iter_mut().find(|j| j.id == job) {
        j.ctl.cancel.store(true, Relaxed);
        j.ctl.pause.store(false, Relaxed);
        if j.state == State::Waiting || j.state == State::Asking {
            j.state = State::Cancelled;
        }
    }
    cv.notify_all();
}

pub fn toggle_pause(q: &Shared, job: Option<u64>) {
    let (m, cv) = &**q;
    let mut g = m.lock().unwrap();
    let all_paused = g.jobs.iter().filter(|j| j.active()).all(|j| j.paused());
    for j in g.jobs.iter_mut().filter(|j| job.is_none_or(|id| j.id == id)) {
        let v = if job.is_none() { !all_paused } else { !j.paused() };
        j.ctl.pause.store(v, Relaxed);
    }
    cv.notify_all();
}

/// Undo a finished job when its journal entries are still the newest ones.
pub fn undo(q: &Shared, job: u64) -> Result<String, String> {
    let (m, _) = &**q;
    let ids = {
        let g = m.lock().unwrap();
        let j = g.jobs.iter().find(|j| j.id == job).ok_or("no such job")?;
        if j.state != State::Done {
            return Err("only a finished job can be undone".into());
        }
        j.journal.clone()
    };
    let log = fmcore::journal::load();
    let newest: Vec<u64> = log.iter().rev().take(ids.len()).map(|o| o.id).collect();
    let mut want = ids.clone();
    want.reverse();
    if ids.is_empty() || newest != want {
        return Err("newer changes came after this one; undo them first (Ctrl+Z)".into());
    }
    let mut last = String::new();
    for _ in &ids {
        last = fmcore::journal::undo_last()?.summary();
    }
    let mut g = m.lock().unwrap();
    if let Some(j) = g.jobs.iter_mut().find(|j| j.id == job) {
        j.state = State::Undone;
    }
    Ok(last)
}

fn same_file(a: &Path, b: &Path) -> Option<(bool, u64, u64, u64, u64)> {
    let ma = std::fs::metadata(a).ok()?;
    let mb = std::fs::metadata(b).ok()?;
    let ft = |m: &std::fs::Metadata| m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let (ta, tb) = (ft(&ma), ft(&mb));
    // same size and the exact same time (a copy keeps it) = the same file; within 2 s (FAT keeps 2 s steps) only
    // when the bytes match too, so two different files saved in the same second are never skipped silently
    let exact = ma.modified().ok() == mb.modified().ok();
    let same_bytes = || ma.len() <= 64 << 20 && std::fs::read(a).ok().zip(std::fs::read(b).ok()).map(|(x, y)| x == y).unwrap_or(false);
    let identical = ma.is_file() && mb.is_file() && ma.len() == mb.len() && (exact || (ta.abs_diff(tb) <= 2 && same_bytes()));
    let to_ft = |s: u64| fmcore::util::unix_ft(s as i64);
    Some((identical, ma.len(), to_ft(ta), mb.len(), to_ft(tb)))
}

/// Start the worker thread. `notify()` runs after every visible change.
pub fn start(q: Shared, notify: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let _com = fmcore::util::com();
        loop {
            // next waiting job
            let job = {
                let (m, cv) = &*q;
                let mut g = m.lock().unwrap();
                loop {
                    if let Some(j) = g.jobs.iter_mut().find(|j| j.state == State::Waiting) {
                        j.state = State::Running;
                        j.started = Some(Instant::now());
                        break j.clone();
                    }
                    g = cv.wait(g).unwrap();
                }
            };
            notify();
            run_job(&q, job, &notify);
            notify();
        }
    });
}

fn set<F: FnOnce(&mut Job)>(q: &Shared, id: u64, f: F) {
    let (m, _) = &**q;
    let mut g = m.lock().unwrap();
    if let Some(j) = g.jobs.iter_mut().find(|j| j.id == id) {
        f(j);
    }
}

fn run_job(q: &Shared, job: Job, notify: &impl Fn()) {
    let id = job.id;
    let dest = job.dest.clone();
    // 1. find clashes
    let mut plain = vec![];
    let mut clashes = vec![];
    let mut identical = 0;
    let mut total = 0u64;
    for s in &job.srcs {
        let Some(name) = s.file_name() else { continue };
        let t = dest.join(name);
        let same_place = fmcore::util::path_eq(s, &t);
        if same_place && job.kind == Kind::Move {
            identical += 1;
            continue;
        }
        if same_place {
            // copy into its own folder: always a second copy next to it
            clashes.push(Clash { src: s.clone(), target: t, src_size: 0, src_time: 0, dst_size: 0, dst_time: 0, choice: Some(Conflict::KeepBoth) });
            total += ops::tree_size(s).0;
            continue;
        }
        if fmcore::journal::exists(&t) {
            match same_file(s, &t) {
                Some((true, ..)) => {
                    identical += 1;
                    continue;
                }
                Some((false, ss, st, ds, dt)) => {
                    total += ops::tree_size(s).0;
                    clashes.push(Clash { src: s.clone(), target: t, src_size: ss, src_time: st, dst_size: ds, dst_time: dt, choice: None });
                    continue;
                }
                None => {}
            }
        }
        if job.kind == Kind::Copy || !fmcore::util::same_volume(s, &dest) {
            total += ops::tree_size(s).0;
        }
        plain.push(s.clone());
    }
    set(q, id, |j| {
        j.total = total;
        j.identical = identical;
        j.clashes = clashes.clone();
    });
    notify();
    let mut base = 0u64;
    let mut t_last = Instant::now();
    let mut done_last = 0u64;
    let mut speed = 0.0f64;
    let mut run_part = |srcs: &[PathBuf], policy: Conflict| -> Result<ops::Report, String> {
        let ctl = job.ctl.clone();
        let mut on = |p: &Progress| {
            let done = base + p.bytes_done;
            let dt = t_last.elapsed().as_secs_f64();
            if dt > 0.25 {
                let inst = (done.saturating_sub(done_last)) as f64 / dt;
                speed = if speed == 0.0 { inst } else { speed * 0.85 + inst * 0.15 };
                t_last = Instant::now();
                done_last = done;
            }
            let sp = speed;
            set(q, id, |j| {
                j.done = done.min(j.total.max(done));
                j.files_done = p.files_done;
                j.files_total = p.files_total;
                j.speed = sp;
                j.eta = if sp > 1.0 { Some((j.total.saturating_sub(done) as f64 / sp) as u64) } else { None };
            });
            notify();
        };
        let r = match job.kind {
            Kind::Copy => ops::copy(srcs, &dest, policy, &ctl, &mut on),
            Kind::Move => ops::move_items(srcs, &dest, policy, &ctl, &mut on),
        };
        if let Ok(rep) = &r {
            base += rep.bytes;
        }
        r
    };
    let mut journal = vec![];
    let mut errors = vec![];
    let mut results = vec![];
    let mut cancelled = false;
    let mut take = |r: Result<ops::Report, String>, journal: &mut Vec<u64>, errors: &mut Vec<String>, results: &mut Vec<PathBuf>| -> bool {
        match r {
            Ok(rep) => {
                if rep.journal_id != 0 {
                    journal.push(rep.journal_id);
                }
                errors.extend(rep.errors);
                results.extend(rep.done.into_iter().map(|d| d.1));
                rep.cancelled
            }
            Err(e) => {
                errors.push(e);
                false
            }
        }
    };
    // 2. everything without a clash first
    if !plain.is_empty() {
        cancelled |= take(run_part(&plain, Conflict::Fail), &mut journal, &mut errors, &mut results);
    }
    // 3. ask about the clashes (unless already decided), then do them grouped by choice
    if !cancelled && !clashes.is_empty() {
        let (m, cv) = &**q;
        let mut g = m.lock().unwrap();
        loop {
            let Some(j) = g.jobs.iter_mut().find(|j| j.id == id) else { return };
            if j.state == State::Cancelled || j.ctl.cancelled() {
                cancelled = true;
                break;
            }
            if j.open_clashes() == 0 {
                j.state = State::Running;
                clashes = j.clashes.clone();
                break;
            }
            j.state = State::Asking;
            drop(g);
            notify();
            g = cv.wait(m.lock().unwrap()).unwrap();
        }
        drop(g);
        notify();
        if !cancelled {
            for policy in [Conflict::Replace, Conflict::KeepBoth] {
                let srcs: Vec<PathBuf> = clashes.iter().filter(|c| c.choice == Some(policy)).map(|c| c.src.clone()).collect();
                if !srcs.is_empty() {
                    cancelled |= take(run_part(&srcs, policy), &mut journal, &mut errors, &mut results);
                }
            }
        }
    }
    set(q, id, |j| {
        j.journal = journal;
        j.results = results;
        j.done = j.total;
        j.eta = None;
        j.state = if cancelled || j.ctl.cancelled() {
            State::Cancelled
        } else if !errors.is_empty() && j.results.is_empty() {
            State::Failed(errors.first().cloned().unwrap_or_default())
        } else {
            State::Done
        };
        j.errors = errors;
    });
}
