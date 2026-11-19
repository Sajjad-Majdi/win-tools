//! Folder watching with ReadDirectoryChangesW (overlapped I/O + a stop event, so it ends at once on request).
use crate::util::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACTION, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_REMOVED, FILE_ACTION_RENAMED_NEW_NAME, FILE_ACTION_RENAMED_OLD_NAME,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects};
use windows::core::PCWSTR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Renamed,
    /// Too many changes at once: the GUI should re-list the folder.
    Overflow,
}

impl ChangeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Modified => "modified",
            ChangeKind::Renamed => "renamed",
            ChangeKind::Overflow => "overflow",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Change {
    pub kind: ChangeKind,
    pub path: PathBuf,
    /// The old path of a rename.
    pub old_path: Option<PathBuf>,
}

fn open_dir(dir: &Path) -> Res<HANDLE> {
    let w = wide_path(dir);
    unsafe {
        CreateFileW(
            PCWSTR(w.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .map_err(|e| win_err(&format!("cannot watch {}", dir.display()), &e))
}

/// The watch loop. Ends when `stop` is signalled, the deadline passes, or `cb` returns false.
fn run(dir: &Path, h: HANDLE, stop: HANDLE, recursive: bool, deadline: Option<Instant>, cb: &mut dyn FnMut(&Change) -> bool) -> Res<()> {
    let ev = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| win_err("CreateEvent", &e))?;
    let mut buf = vec![0u32; 16 * 1024]; // 64 KB, DWORD aligned (the limit for network shares)
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_CREATION | FILE_NOTIFY_CHANGE_ATTRIBUTES;
    let mut result = Ok(());
    'outer: loop {
        let mut ov = OVERLAPPED { hEvent: ev, ..Default::default() };
        if let Err(e) = unsafe { ReadDirectoryChangesW(h, buf.as_mut_ptr() as *mut _, (buf.len() * 4) as u32, recursive, filter, None, Some(&mut ov), None) } {
            result = Err(win_err(&format!("watch of {} stopped", dir.display()), &e));
            break;
        }
        let wait_ms = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128 - 1) as u32,
            None => u32::MAX, // INFINITE
        };
        let w = unsafe { WaitForMultipleObjects(&[ev, stop], false, wait_ms) };
        if w != WAIT_OBJECT_0 {
            unsafe {
                let _ = CancelIoEx(h, Some(&ov));
                let mut n = 0u32;
                let _ = GetOverlappedResult(h, &ov, &mut n, true);
            }
            break;
        }
        let mut n = 0u32;
        if let Err(e) = unsafe { GetOverlappedResult(h, &ov, &mut n, false) } {
            result = Err(win_err(&format!("watch of {} stopped", dir.display()), &e));
            break;
        }
        if n == 0 {
            if !cb(&Change { kind: ChangeKind::Overflow, path: dir.to_path_buf(), old_path: None }) {
                break;
            }
            continue;
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, n as usize) };
        let mut off = 0usize;
        let mut old: Option<PathBuf> = None;
        loop {
            if off + 12 > bytes.len() {
                break;
            }
            let next = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
            let action = FILE_ACTION(u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()));
            let len = u32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()) as usize;
            let name_bytes = &bytes[(off + 12).min(bytes.len())..(off + 12 + len).min(bytes.len())];
            let name: Vec<u16> = name_bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let path = dir.join(String::from_utf16_lossy(&name));
            let change = match action {
                FILE_ACTION_ADDED => Some(Change { kind: ChangeKind::Added, path, old_path: None }),
                FILE_ACTION_REMOVED => Some(Change { kind: ChangeKind::Removed, path, old_path: None }),
                FILE_ACTION_MODIFIED => Some(Change { kind: ChangeKind::Modified, path, old_path: None }),
                FILE_ACTION_RENAMED_OLD_NAME => {
                    old = Some(path);
                    None
                }
                FILE_ACTION_RENAMED_NEW_NAME => Some(Change { kind: ChangeKind::Renamed, path, old_path: old.take() }),
                _ => None,
            };
            if let Some(c) = change {
                if !cb(&c) {
                    break 'outer;
                }
            }
            if next == 0 {
                break;
            }
            off += next;
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            break;
        }
    }
    unsafe {
        let _ = CloseHandle(ev);
    }
    result
}

/// Watch on the calling thread until `cb` returns false or `timeout` passes (None = for ever).
pub fn watch_blocking(dir: &Path, recursive: bool, timeout: Option<Duration>, mut cb: impl FnMut(&Change) -> bool) -> Res<()> {
    let dir = abs(dir);
    let h = open_dir(&dir)?;
    let stop = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| win_err("CreateEvent", &e))?;
    let r = run(&dir, h, stop, recursive, timeout.map(|t| Instant::now() + t), &mut cb);
    unsafe {
        let _ = CloseHandle(stop);
        let _ = CloseHandle(h);
    }
    r
}

/// A background watch for the GUI. Changes arrive on the watch thread; dropping the Watcher stops it.
pub struct Watcher {
    stop: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    /// Start watching `dir`. `cb` runs on the watch thread for every change.
    pub fn start(dir: &Path, recursive: bool, mut cb: impl FnMut(&Change) + Send + 'static) -> Res<Watcher> {
        let dir = abs(dir);
        let h = open_dir(&dir)?.0 as isize;
        let stop = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| win_err("CreateEvent", &e))?.0 as isize;
        let thread = std::thread::spawn(move || {
            let (h, stop_h) = (HANDLE(h as *mut _), HANDLE(stop as *mut _));
            let _ = run(&dir, h, stop_h, recursive, None, &mut |c| {
                cb(c);
                true
            });
            unsafe {
                let _ = CloseHandle(h);
            }
        });
        Ok(Watcher { stop, thread: Some(thread) })
    }

    /// Stop and wait for the watch thread.
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        if let Some(t) = self.thread.take() {
            unsafe {
                let _ = SetEvent(HANDLE(self.stop as *mut _));
            }
            let _ = t.join();
            unsafe {
                let _ = CloseHandle(HANDLE(self.stop as *mut _));
            }
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.halt();
    }
}
