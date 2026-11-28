//! The live drive list: This PC, the sidebar, the "Copy to / Move to" drives and Cleanup follow drives that come and
//! go from any cause (a mounted ISO or VHD, a USB stick, a disc, a mapped network drive, a new drive letter, a
//! BitLocker unlock), at once and without a restart, as Explorer does. Any one of these sources is enough:
//! - WM_DEVICECHANGE: the volume broadcasts (arrival / removal, media in / out, subst and network drives) and the
//!   volume interface notices registered for this window (delivered to it on any desktop);
//! - the window becoming active: the drive letters are compared, drives without media are asked again, and free
//!   space and labels are read again when the last read is older than 30 s.
//! The shell's own change notices for This PC (SHChangeNotifyRegister) are NOT used: binding This PC loads ten
//! shell DLLs (+7 MB working set, ~50 handles) and the sources above already cover every drive that comes or goes.
//! A burst of notices is one re-read 250 ms after the last one (at most 300 ms after the first), plus one more
//! 1.6 s later for letters Windows assigns late.
//!
//! A pane inside a drive that went away goes to the nearest folder that still exists, else This PC. Folder
//! watchers register their handle, so a drive being removed safely (by us, Explorer or the tray) is let go of
//! instead of being reported "in use" because of this window.
//!
//! Eject (mounted ISO / VHD, USB stick or disk, DVD, card) and mounting a disc image run on worker threads.
use crate::app::*;
use crate::model::*;
use fmcore::shell::Drive;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// one-shot timers: the re-read after a burst of notices, and the late one
pub const T_DRIVES: usize = 15;
pub const T_DRIVES2: usize = 16;
/// one-shot: watch folders again that were let go of for a volume lock or a removal that did not happen
pub const T_HOLD: usize = 17;
/// one-shot: start listening for drives, right after the first frame
pub const T_WATCH: usize = 18;

/// Registrations and state of the live drive list (one per window).
#[derive(Default)]
pub struct DriveWatch {
    /// the volume interface notice (HDEVNOTIFY)
    dev: isize,
    /// drive letters (GetLogicalDrives bits) of the list shown
    pub mask: u32,
    /// re-reads asked / the newest one applied (an older answer that comes late is dropped)
    seq: u64,
    applied: u64,
    /// drives being ejected now (root -> the name shown)
    pub ejecting: std::collections::HashMap<String, String>,
    /// panes that let go of their folder for a removal (tab id, pane), watched again if it is cancelled
    released: Vec<(u64, usize)>,
    /// disc images being mounted now
    mounting: std::collections::HashSet<PathBuf>,
    hold_tries: u32,
    /// re-reads asked because a pane was in a drive the list lacked (capped)
    stale_reads: u32,
    /// the notices are registered (once per window)
    listening: bool,
    /// the first notice of the burst whose re-read is pending (the re-read waits at most 300 ms from it)
    burst: Option<std::time::Instant>,
    /// when the shown list was read (free space and labels are read again on activation after 30 s)
    read_at: Option<std::time::Instant>,
}

/// A folder watcher plus its device notice: Windows asks this window to let go of the folder when its drive is
/// removed safely (DBT_DEVICEQUERYREMOVE). Dropping it unregisters the notice first, then stops the watcher.
pub struct Watch {
    w: Option<fmcore::watch::Watcher>,
    note: isize,
}

impl Watch {
    pub fn new(w: fmcore::watch::Watcher, hwnd: HWND) -> Watch {
        let t0 = std::time::Instant::now();
        let f = DEV_BROADCAST_HANDLE {
            dbch_size: size_of::<DEV_BROADCAST_HANDLE>() as u32,
            dbch_devicetype: DBT_DEVTYP_HANDLE.0,
            dbch_handle: HANDLE(w.handle() as *mut _),
            ..Default::default()
        };
        let note = unsafe { RegisterDeviceNotificationW(HANDLE(hwnd.0), &f as *const _ as *const _, DEVICE_NOTIFY_WINDOW_HANDLE) }.map(|h| h.0 as isize).unwrap_or(0);
        trace(&format!("folder notice {} in {:.2} ms", if note != 0 { "registered" } else { "refused" }, t0.elapsed().as_secs_f64() * 1000.0));
        Watch { w: Some(w), note }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if self.note != 0 {
            unsafe {
                let _ = UnregisterDeviceNotification(HDEVNOTIFY(self.note as *mut _));
            }
        }
        drop(self.w.take());
    }
}

/// Names of the drives as last read (letter -> "Label (E:)"), for titles and the address bar: no disk access
/// while painting.
static NAMES: Mutex<Vec<(char, String)>> = Mutex::new(Vec::new());

/// "Main (D:)", "Local Disk (C:)", "USB Drive (E:)", "DVD Drive (G:)", "share (\\server) (Z:)": Explorer's names.
pub fn drive_name(d: &Drive) -> String {
    let letter = d.root.trim_end_matches('\\');
    if !d.label.is_empty() {
        return format!("{} ({letter})", d.label);
    }
    format!("{} ({letter})", generic_name(d.kind, &d.remote))
}

pub fn generic_name(kind: &str, remote: &str) -> String {
    match kind {
        "removable" => "USB Drive".into(),
        "cdrom" => "DVD Drive".into(),
        "ramdisk" => "RAM Disk".into(),
        "network" => match remote.trim_start_matches('\\').split_once('\\') {
            Some((server, share)) if !share.is_empty() => format!("{share} (\\\\{server})"),
            _ => "Network Drive".into(),
        },
        _ => "Local Disk".into(),
    }
}

/// The name of a drive from the last read ("C:" or "C:\"), if it was there.
pub fn cached_name(letter: &str) -> Option<String> {
    let c = letter.chars().next()?.to_ascii_uppercase();
    NAMES.lock().ok()?.iter().find(|(l, _)| *l == c).map(|(_, n)| n.clone())
}

fn letter(root: &str) -> Option<char> {
    let b = root.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| (b[0] as char).to_ascii_uppercase())
}

/// "G:\" for a path on a lettered drive.
fn root_of(p: &Path) -> Option<String> {
    letter(&p.to_string_lossy()).map(|l| format!("{l}:\\"))
}

fn mask_of(drives: &[Drive]) -> u32 {
    drives.iter().filter_map(|d| letter(&d.root)).fold(0, |m, l| m | 1 << (l as u8 - b'A'))
}

/// The drive of this folder is not there any more (unplugged, ejected, no media, unmapped). Network drives are
/// never judged here (a slow share is not a gone one). Instant: one bit test and, for local drives, one attribute
/// read of the root.
pub fn root_gone(dir: &Path) -> bool {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    let Some(l) = letter(&dir.to_string_lossy()) else { return false };
    if letter_gone(l) {
        return true;
    }
    if is_network_root(&format!("{l}:\\")) {
        return false;
    }
    let w = fmcore::util::wide(format!("{l}:\\"));
    unsafe { GetFileAttributesW(windows::core::PCWSTR(w.as_ptr())) == INVALID_FILE_ATTRIBUTES }
}

/// The drive letter is not assigned any more (one bit test, no disk access).
fn letter_gone(l: char) -> bool {
    unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() & (1 << (l as u8 - b'A')) == 0 }
}

/// A mapped network drive (asks no server).
fn is_network_root(root: &str) -> bool {
    let w = fmcore::util::wide(root);
    unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(w.as_ptr())) == 4 }
}

/// Where a pane in `dir` goes when its folder is gone: the nearest parent that still exists, else This PC.
fn nearest(dir: &Path) -> Loc {
    if root_gone(dir) {
        return Loc::ThisPc;
    }
    let mut p = dir.parent();
    while let Some(d) = p {
        if d.as_os_str().is_empty() {
            break;
        }
        if d.is_dir() {
            return Loc::Folder(d.to_path_buf());
        }
        p = d.parent();
    }
    Loc::ThisPc
}

impl App {
    /// Start listening for drives that come and go (once, when the window exists).
    pub fn drives_watch_start(&mut self) {
        if std::mem::replace(&mut self.dw.listening, true) {
            return;
        }
        let t0 = std::time::Instant::now();
        unsafe {
            // volumes appearing and going (directed to this window: also reaches a window on another desktop)
            let f = DEV_BROADCAST_DEVICEINTERFACE_W {
                dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
                dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
                dbcc_classguid: windows::Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME,
                ..Default::default()
            };
            self.dw.dev = RegisterDeviceNotificationW(HANDLE(self.hwnd.0), &f as *const _ as *const _, DEVICE_NOTIFY_WINDOW_HANDLE).map(|h| h.0 as isize).unwrap_or(0);
        }
        trace(&format!("drive notices: volume {} ({:.2} ms)", self.dw.dev != 0, t0.elapsed().as_secs_f64() * 1000.0));
    }

    /// Stop listening (the window is closing).
    pub fn drives_watch_stop(&mut self) {
        if self.dw.dev != 0 {
            unsafe {
                let _ = UnregisterDeviceNotification(HDEVNOTIFY(self.dw.dev as *mut _));
            }
            self.dw.dev = 0;
        }
    }

    /// Something about the drives changed: read them again soon (a burst of notices is one read), and once more a
    /// bit later for a letter Windows assigns after the volume itself appeared.
    pub fn drives_changed(&mut self) {
        self.drives_soon();
        self.timer(T_DRIVES2, 1600);
    }

    /// One re-read for a burst of notices: 250 ms after the last one, but never later than 300 ms after the first
    /// (four drives ejected one after another must not keep pushing the read back while the gone ones stay listed).
    fn drives_soon(&mut self) {
        let first = *self.dw.burst.get_or_insert_with(std::time::Instant::now);
        let left = 300u32.saturating_sub(first.elapsed().as_millis() as u32);
        self.timer(T_DRIVES, left.clamp(1, 250));
    }

    pub fn drives_timer(&mut self, id: usize) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(self.hwnd), id);
        }
        if id == T_DRIVES {
            self.dw.burst = None;
        }
        self.refresh_drives();
    }

    /// Read the drives now, off the UI thread (a dead network share answers within 1.5 s as "no answer").
    pub fn refresh_drives(&mut self) {
        self.dw.seq += 1;
        let seq = self.dw.seq;
        self.spawn(move || Msg::Drives(seq, fmcore::shell::drives_within(Duration::from_millis(1500))));
    }

    /// The window became active: drive letters that differ from the list, or drives without media (a card or
    /// disc put in, a BitLocker drive unlocked meanwhile), are read again.
    pub fn drives_on_activate(&mut self) {
        if self.dw.applied == 0 {
            // the first read is still on its way
            return;
        }
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let old = self.dw.read_at.is_none_or(|t| t.elapsed() > Duration::from_secs(30));
        if mask != self.dw.mask || old || self.drives.iter().any(|d| !d.ready && d.kind != "network") {
            self.drives_soon();
        }
    }

    /// A drive list arrived (`seq` = which read).
    pub fn drives_arrived(&mut self, seq: u64, d: Vec<Drive>) {
        if seq <= self.dw.applied {
            return;
        }
        self.dw.applied = seq;
        self.dw.read_at = Some(std::time::Instant::now());
        self.set_drives(d);
    }

    fn set_drives(&mut self, d: Vec<Drive>) {
        let changed = d.len() != self.drives.len()
            || d.iter().zip(&self.drives).any(|(a, b)| a.root != b.root || a.label != b.label || a.ready != b.ready || a.free != b.free || a.total != b.total || a.eject != b.eject || a.kind != b.kind);
        self.drives = d;
        self.dw.mask = mask_of(&self.drives);
        if let Ok(mut n) = NAMES.lock() {
            *n = self.drives.iter().filter_map(|d| Some((letter(&d.root)?, drive_name(d)))).collect();
        }
        self.cleanup_drives_changed();
        self.cleanup_drives_ready();
        self.leave_gone_drives();
        self.close_gone_views();
        if changed {
            for t in 0..self.tabs.len() {
                for p in 0..self.tabs[t].panes.len() {
                    if self.tabs[t].panes[p].loc == Loc::ThisPc {
                        self.load(t, p, true);
                    }
                }
            }
        }
        self.invalidate();
    }

    /// Panes inside a drive that is gone (or lost its media, or whose folder is gone with it) move to the nearest
    /// folder that still exists, or This PC, as in Explorer. Network drives are left alone (slow is not gone).
    fn leave_gone_drives(&mut self) {
        let mut stale = false;
        for t in 0..self.tabs.len() {
            for p in 0..self.tabs[t].panes.len() {
                let Loc::Folder(dir) = self.tabs[t].panes[p].loc.clone() else { continue };
                let Some(root) = root_of(&dir) else { continue };
                let listed = self.drives.iter().find(|d| d.root.eq_ignore_ascii_case(&root));
                // the list is only a hint: a read that started before a drive arrived can come in after a pane
                // opened it (a just mounted ISO); the drive itself decides, right now. A network drive that is
                // still mapped is never judged here (a slow share is not a gone one, and asking it could hang)
                // (a local drive that did not answer in time, a disk spinning up, is not asked on this thread either)
                let to = if letter(&root).is_some_and(letter_gone) {
                    Some(Loc::ThisPc)
                } else if listed.is_some_and(|d| d.timed_out) || is_network_root(&root) {
                    None
                } else if root_gone(&dir) {
                    Some(Loc::ThisPc)
                } else if !dir.is_dir() {
                    Some(nearest(&dir))
                } else {
                    stale |= listed.is_none_or(|d| !d.ready);
                    None
                };
                if let Some(to) = to {
                    trace(&format!("drive gone: {} -> {}", dir.display(), to.display()));
                    self.leave_pane(t, p, to);
                }
            }
        }
        // a pane is in a drive the list lacks (or calls empty): that list was read too early, read again (a few
        // times at most: a volume that answers no questions must not keep the reads going)
        if stale && self.dw.stale_reads < 3 {
            self.dw.stale_reads += 1;
            self.timer(T_DRIVES, 400);
        } else if !stale {
            self.dw.stale_reads = 0;
        }
    }

    /// The Space preview and the picture viewer let go of files whose drive went away (ejected, unplugged, no
    /// media): the preview closes; the viewer drops those pictures and closes when none is left.
    fn close_gone_views(&mut self) {
        let gone = |p: &Path| root_of(p).is_some_and(|r| letter(&r).is_some_and(letter_gone) || root_gone(p));
        if self.pv.as_ref().is_some_and(|pv| gone(&pv.path)) {
            trace("drive gone: preview closed");
            self.pv = None;
        }
        if let Some(v) = crate::viewer::get()
            && v.list.iter().any(|p| gone(p))
        {
            let cur = v.list.get(v.idx).cloned();
            let keep: Vec<PathBuf> = v.list.iter().filter(|p| !gone(p)).cloned().collect();
            if keep.is_empty() || cur.as_deref().is_some_and(gone) {
                trace("drive gone: picture viewer closed");
                crate::viewer::close();
            } else {
                let idx = cur.and_then(|c| keep.iter().position(|p| *p == c)).unwrap_or(0);
                v.set_list(keep, idx);
            }
        }
    }

    fn leave_pane(&mut self, t: usize, p: usize, to: Loc) {
        self.tabs[t].panes[p].watch.clear();
        self.tabs[t].visit(p, to, true);
        self.after_move(t, p, None);
    }

    /// A pane's folder could not be listed: when its drive is gone, the pane moves away at once (no error page) and
    /// the drive list is read again. Returns true when it moved.
    pub fn left_gone_folder(&mut self, t: usize, p: usize) -> bool {
        let Loc::Folder(dir) = self.tabs[t].panes[p].loc.clone() else { return false };
        // local drives only (a slow network share is not a gone one); a folder that exists failed for another
        // reason (no access) and keeps its error
        let local = root_of(&dir).is_some_and(|r| !self.is_network(&r));
        if self.tabs[t].panes[p].searching() || !local || !(root_gone(&dir) || !dir.exists()) {
            return false;
        }
        let to = nearest(&dir);
        trace(&format!("folder gone: {} -> {}", dir.display(), to.display()));
        self.leave_pane(t, p, to);
        self.drives_changed();
        true
    }

    fn is_network(&self, root: &str) -> bool {
        self.drives.iter().any(|d| d.root.eq_ignore_ascii_case(root) && d.kind == "network")
    }

    /// WM_DEVICECHANGE.
    pub fn on_device_change(&mut self, wp: usize, lp: isize) -> LRESULT {
        const ARRIVAL: usize = 0x8000;
        const QUERYREMOVE: usize = 0x8001;
        const QUERYREMOVEFAILED: usize = 0x8002;
        const REMOVEPENDING: usize = 0x8003;
        const REMOVECOMPLETE: usize = 0x8004;
        const CUSTOMEVENT: usize = 0x8006;
        let kind = if lp != 0 { unsafe { (*(lp as *const DEV_BROADCAST_HDR)).dbch_devicetype.0 } } else { 0 };
        if kind == DBT_DEVTYP_HANDLE.0 {
            // one of our folder watchers: its drive is being removed, locked or dismounted
            let h = unsafe { &*(lp as *const DEV_BROADCAST_HANDLE) };
            let note = h.dbch_hdevnotify.0 as isize;
            match wp {
                QUERYREMOVE => {
                    // the notice is unregistered with the watcher, so a removal that another program vetoes later
                    // (QUERYREMOVEFAILED) never reaches us: look again soon either way (a drive that went is dropped)
                    self.let_go(note);
                    self.hold_again_soon();
                }
                QUERYREMOVEFAILED => self.hold_again_soon(),
                REMOVEPENDING | REMOVECOMPLETE => {
                    self.let_go(note);
                    self.drives_changed();
                }
                CUSTOMEVENT => {
                    let g = h.dbch_eventguid;
                    if g == GUID_IO_VOLUME_LOCK || g == GUID_IO_VOLUME_DISMOUNT {
                        // another program locks or dismounts the volume (an eject, Format, a disk check): our open
                        // folder must not make it fail; it is watched again once the volume is back
                        self.let_go(note);
                        self.hold_again_soon();
                    } else if g == GUID_IO_MEDIA_REMOVAL || g == GUID_IO_MEDIA_ARRIVAL {
                        self.drives_changed();
                    }
                }
                _ => {}
            }
            trace(&format!("device notice {wp:#x} for a watched folder (event {:?})", h.dbch_eventguid));
            return LRESULT(1);
        }
        if matches!(wp, ARRIVAL | REMOVECOMPLETE | CUSTOMEVENT) {
            trace(&format!("device notice {wp:#x} type {kind}"));
            self.drives_changed();
        }
        LRESULT(1)
    }

    /// Let go of the folder whose watcher has the notice `note` (Windows asks before removing its drive).
    fn let_go(&mut self, note: isize) {
        for t in 0..self.tabs.len() {
            let id = self.tabs[t].id;
            for p in 0..self.tabs[t].panes.len() {
                if self.tabs[t].panes[p].watch.iter().any(|w| w.note == note) {
                    self.tabs[t].panes[p].watch.clear();
                    if !self.dw.released.contains(&(id, p)) {
                        self.dw.released.push((id, p));
                    }
                }
            }
        }
    }

    /// Watch the folders that were let go of again, a bit later (the volume lock or removal may still be going on).
    fn hold_again_soon(&mut self) {
        self.dw.hold_tries = 0;
        self.timer(T_HOLD, 800);
    }

    pub fn hold_timer(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_HOLD);
        }
        // a volume still locked (a disk check) is asked again for a while; a drive that went away is left to the
        // drive list, which moves its panes
        if self.hold_again() && self.dw.hold_tries < 20 {
            self.dw.hold_tries += 1;
            self.timer(T_HOLD, 1000);
        }
    }

    /// Watch the folders that were let go of again (the removal was cancelled, the volume is unlocked). Returns
    /// true when some folder could not be reached yet while its drive is still there.
    fn hold_again(&mut self) -> bool {
        let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
        let mut waiting = vec![];
        for (id, p) in std::mem::take(&mut self.dw.released) {
            let Some(t) = self.tabs.iter().position(|t| t.id == id) else { continue };
            if p >= self.tabs[t].panes.len() || !self.tabs[t].panes[p].watch.is_empty() {
                continue;
            }
            let Loc::Folder(dir) = self.tabs[t].panes[p].loc.clone() else { continue };
            if dir.is_dir() {
                self.load(t, p, true);
            } else if letter(&dir.to_string_lossy()).is_some_and(|l| mask & (1 << (l as u8 - b'A')) != 0) {
                waiting.push((id, p));
            }
        }
        let more = !waiting.is_empty();
        self.dw.released = waiting;
        more
    }

    /// Whether this drive can be ejected now ("" = no Eject in its menu).
    pub fn eject_kind_of(&self, root: &Path) -> &'static str {
        self.drives.iter().find(|d| fmcore::util::path_eq(Path::new(&d.root), root)).map(|d| d.eject).unwrap_or("")
    }

    /// A DVD / CD drive or a mounted ISO (read-only media).
    pub fn is_disc(&self, root: &Path) -> bool {
        self.drives.iter().any(|d| fmcore::util::path_eq(Path::new(&d.root), root) && d.kind == "cdrom")
    }

    /// A folder on a drive Everything does not index: a DVD or mounted ISO, or a FAT / exFAT / UDF volume (it
    /// indexes NTFS and ReFS). Network drives are not judged (Everything may index a share as a folder).
    pub fn unindexed(&self, p: &Path) -> bool {
        let Some(r) = root_of(p) else { return false };
        self.drives.iter().any(|d| {
            d.root.eq_ignore_ascii_case(&r) && d.kind != "network" && (d.kind == "cdrom" || (d.ready && !d.fs.is_empty() && !d.fs.eq_ignore_ascii_case("NTFS") && !d.fs.eq_ignore_ascii_case("ReFS")))
        })
    }

    /// A path on a drive nothing can be written to (a DVD, a mounted ISO, a write-protected card), from the list.
    pub fn read_only_at(&self, p: &Path) -> bool {
        root_of(p).is_some_and(|r| self.drives.iter().any(|d| d.root.eq_ignore_ascii_case(&r) && d.read_only))
    }

    pub fn is_ejecting(&self, root: &Path) -> bool {
        self.dw.ejecting.keys().any(|r| fmcore::util::path_eq(Path::new(r), root))
    }

    /// Eject a drive (menu "Eject"): the panes inside it let go of it first, the eject runs on a worker thread, and
    /// on success the drive leaves the list at once.
    pub fn eject_drive(&mut self, root: &Path) {
        let Some(root) = root_of(root) else { return };
        if self.is_ejecting(Path::new(&root)) {
            return;
        }
        let name = self.drives.iter().find(|d| d.root.eq_ignore_ascii_case(&root)).map(drive_name).unwrap_or_else(|| root.clone());
        self.dw.ejecting.insert(root.clone(), name.clone());
        for t in 0..self.tabs.len() {
            let id = self.tabs[t].id;
            for p in 0..self.tabs[t].panes.len() {
                let inside = self.tabs[t].panes[p].loc.path().and_then(root_of).is_some_and(|r| r.eq_ignore_ascii_case(&root));
                if inside && !self.tabs[t].panes[p].watch.is_empty() {
                    self.tabs[t].panes[p].watch.clear();
                    self.dw.released.push((id, p));
                }
            }
        }
        self.toast("Ejecting ", &name, false);
        let r = root.clone();
        self.spawn(move || Msg::Ejected { res: fmcore::volume::eject(&r), root: r });
    }

    pub fn ejected(&mut self, root: String, res: Result<fmcore::volume::Eject, String>) {
        use fmcore::volume::Eject;
        let name = self.dw.ejecting.remove(&root).unwrap_or_else(|| root.clone());
        match res {
            Ok(how) => {
                self.dw.released.clear();
                let mut d = self.drives.clone();
                if matches!(how, Eject::Media) {
                    // the drive stays, empty
                    for x in d.iter_mut().filter(|x| x.root.eq_ignore_ascii_case(&root)) {
                        x.ready = false;
                        x.label.clear();
                        x.fs.clear();
                        x.free = 0;
                        x.total = 0;
                    }
                } else {
                    d.retain(|x| !x.root.eq_ignore_ascii_case(&root));
                }
                // reads asked before the eject may still list it: they are dropped
                self.dw.applied = self.dw.seq;
                self.set_drives(d);
                self.drives_changed();
                match how {
                    Eject::Device => self.toast("Safe to remove ", &name, false),
                    _ => self.toast("Ejected ", &name, false),
                }
            }
            Err(e) => {
                if self.hold_again() {
                    self.hold_again_soon();
                }
                // "RFETEST (G:) is in use ..." reads better than the bare root
                self.toast_err(&e.replace(&root, &name));
            }
        }
    }

    /// Open a disc image with Windows' own Mount: mount it off the UI thread, then show the new drive in a new tab.
    pub fn mount_and_open(&mut self, path: PathBuf) {
        // a second double-click while it mounts does not mount it twice
        if !self.dw.mounting.insert(path.clone()) {
            return;
        }
        self.spawn(move || Msg::Mounted { res: fmcore::volume::mount_image(&path), path });
    }

    pub fn mounted(&mut self, path: PathBuf, res: Result<String, String>) {
        self.dw.mounting.remove(&path);
        match res {
            Ok(root) => {
                self.refresh_drives();
                // already open (Enter on a mounted image again): that tab comes to the front, no second copy;
                // a tab at the drive's top first, else one somewhere inside it
                let on = |l: &Loc, exact: bool| l.path().is_some_and(|p| if exact { fmcore::util::path_eq(p, Path::new(&root)) } else { root_of(p).is_some_and(|r| r.eq_ignore_ascii_case(&root)) });
                let open = self.tabs.iter().position(|t| on(&t.pane().loc, true)).or_else(|| self.tabs.iter().position(|t| on(&t.pane().loc, false)));
                match open {
                    Some(i) => {
                        self.select_tab(i);
                        self.invalidate();
                    }
                    None => self.new_tab(Loc::Folder(PathBuf::from(root))),
                }
            }
            Err(e) => {
                // not mountable here: Windows' own default action decides (it shows its own message)
                trace(&format!("mount failed ({e}); the shell opens it"));
                if std::env::var("WFILES_TEST").is_ok() {
                    // a test window never starts Windows' own handler (its window would open on the owner's screen)
                    self.toast_err(&e);
                } else if let Err(e) = fmcore::shell::open_file(&path) {
                    self.toast_err(&e);
                }
            }
        }
    }

    /// The drives for `wfm gui state`.
    pub fn drives_json(&self) -> serde_json::Value {
        serde_json::json!(self.drives.iter().map(|d| serde_json::json!({"root": d.root, "name": drive_name(d), "kind": d.kind, "eject": d.eject, "ready": d.ready, "total": d.total, "free": d.free})).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(root: &str, kind: &'static str, label: &str, remote: &str) -> Drive {
        Drive { root: root.into(), label: label.into(), kind, fs: String::new(), free: 0, total: 0, ready: true, remote: remote.into(), timed_out: false, eject: "", read_only: kind == "cdrom" }
    }

    #[test]
    fn explorer_style_names() {
        assert_eq!(drive_name(&drive("C:\\", "fixed", "", "")), "Local Disk (C:)");
        assert_eq!(drive_name(&drive("D:\\", "fixed", "Main", "")), "Main (D:)");
        assert_eq!(drive_name(&drive("E:\\", "removable", "", "")), "USB Drive (E:)");
        assert_eq!(drive_name(&drive("G:\\", "cdrom", "", "")), "DVD Drive (G:)");
        assert_eq!(drive_name(&drive("G:\\", "cdrom", "RFETEST", "")), "RFETEST (G:)");
        assert_eq!(drive_name(&drive("Z:\\", "network", "", r"\\nas\media")), r"media (\\nas) (Z:)");
        assert_eq!(drive_name(&drive("Z:\\", "network", "", "")), "Network Drive (Z:)");
    }

    #[test]
    fn letters_and_masks() {
        assert_eq!(root_of(Path::new(r"g:\dir\x")), Some("G:\\".into()));
        assert_eq!(root_of(Path::new(r"\\server\share\x")), None);
        assert_eq!(mask_of(&[drive("C:\\", "fixed", "", ""), drive("G:\\", "cdrom", "", "")]), 1 << 2 | 1 << 6);
    }

    #[test]
    fn a_missing_drive_is_gone_and_leads_to_this_pc() {
        use windows::Win32::Storage::FileSystem::GetLogicalDrives;
        let mask = unsafe { GetLogicalDrives() };
        let Some(free) = (b'D'..=b'Z').rev().find(|l| mask & (1 << (l - b'A')) == 0) else { return };
        let dir = PathBuf::from(format!("{}:\\some\\folder", free as char));
        assert!(root_gone(&dir));
        assert!(nearest(&dir) == Loc::ThisPc);
        // the Windows drive is there; a missing folder on it leads to its nearest existing parent
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(!root_gone(Path::new(&sys)));
        let gone = Path::new(&sys).join("no such folder 7f3a").join("deeper");
        assert!(nearest(&gone) == Loc::Folder(PathBuf::from(&sys)));
        // network paths (UNC) are never judged gone
        assert!(!root_gone(Path::new(r"\\server\share\x")));
    }
}
