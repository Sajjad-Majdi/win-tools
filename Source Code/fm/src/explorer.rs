//! Folders opened from inside a Windows File Explorer window stay in that window.
//!
//! While Rust File Explorer is the default for folders, Explorer runs our "open" verb for every folder double-clicked
//! in its own list too. [`hand_back`] tells that case apart at the very start of RustFileExplorer.exe: the request
//! came from Explorer (our parent is explorer.exe), an Explorer window is in front, and the folder is what its active
//! tab has selected (or lies in the folder that tab shows). That tab is then sent to the folder itself
//! (`IWebBrowser2::Navigate2`, the same as typing it into its address bar) and our window never opens. Anything else
//! (desktop, Run, other apps, Win+E) opens here as before.
use crate::util::*;
use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, CoCreateInstance, IDispatch, IServiceProvider};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{Folder2, IShellBrowser, IShellFolderViewDual, IShellWindows, IWebBrowser2, SID_STopLevelBrowser, ShellWindows};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetClassNameW, GetForegroundWindow};
use windows::core::{Interface, w};

/// One Explorer tab (each tab of a Windows 11 Explorer window is its own entry).
#[derive(Clone, Debug, Default)]
pub struct ExplorerTab {
    /// the top-level window (CabinetWClass)
    pub hwnd: isize,
    /// the tab's own window (ShellTabWindowClass), 0 when unknown
    pub tab: isize,
    /// the tab in front in its window
    pub active: bool,
    /// the folder it shows (file system path; empty for virtual places like Home)
    pub location: String,
    /// the selected items' paths (at most 64)
    pub selected: Vec<String>,
    /// the place as the shell names it ("::{GUID}" for virtual places, else the path)
    pub raw: String,
}

/// "c:\users\me" style key: lower case, no trailing backslash ("c:" for a drive root).
pub fn norm(p: &str) -> String {
    p.trim().trim_matches('"').trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

/// Is `target` the folder this tab is opening? It is selected there (or a selected shortcut points at it), or it lies
/// right inside the folder the tab shows. `links` resolves a selected .lnk to its target.
pub fn wanted_by(tab: &ExplorerTab, target: &str, links: impl Fn(&str) -> Option<String>) -> bool {
    let t = norm(target);
    if t.is_empty() {
        return false;
    }
    if tab.selected.iter().any(|s| norm(s) == t) {
        return true;
    }
    if tab.selected.iter().filter(|s| s.to_lowercase().ends_with(".lnk")).any(|s| links(s).map(|l| norm(&l) == t).unwrap_or(false)) {
        return true;
    }
    let parent = Path::new(&t).parent().map(|p| norm(&p.to_string_lossy())).unwrap_or_default();
    !tab.location.is_empty() && !parent.is_empty() && parent == norm(&tab.location)
}

fn class_of(h: HWND) -> String {
    let mut b = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut b) };
    String::from_utf16_lossy(&b[..n.max(0) as usize])
}

/// Our parent process is explorer.exe (Explorer started us: its folder windows, the desktop, Run all live there).
fn parent_is_explorer() -> bool {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let me = GetCurrentProcessId();
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut procs = vec![];
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            procs.push((e.th32ProcessID, e.th32ParentProcessID, from_wide(&e.szExeFile)));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        let parent = procs.iter().find(|p| p.0 == me).map(|p| p.1);
        parent.and_then(|pp| procs.iter().find(|p| p.0 == pp)).map(|p| p.2.eq_ignore_ascii_case("explorer.exe")).unwrap_or(false)
    }
}

fn int_variant(i: i32) -> VARIANT {
    let mut v = VARIANT::default();
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = i;
    }
    v
}

/// Location and selection of one tab (through Explorer's automation objects; works from another process).
fn tab_of(wb: &IWebBrowser2) -> ExplorerTab {
    let mut t = ExplorerTab { hwnd: unsafe { wb.HWND() }.map(|h| h.0).unwrap_or(0), ..Default::default() };
    unsafe {
        if let Ok(sb) = wb.cast::<IServiceProvider>().and_then(|sp| sp.QueryService::<IShellBrowser>(&SID_STopLevelBrowser))
            && let Ok(h) = sb.GetWindow()
        {
            t.tab = h.0 as isize;
        }
        let Ok(doc) = wb.Document().and_then(|d| d.cast::<IShellFolderViewDual>()) else { return t };
        if let Ok(f) = doc.Folder().and_then(|f| f.cast::<Folder2>())
            && let Ok(p) = f.Self_().and_then(|s| s.Path())
        {
            let p = p.to_string();
            t.raw = p.clone();
            // virtual places (Home, This PC) give "::{GUID}" names: no folder there
            if !p.starts_with("::") {
                t.location = p;
            }
        }
        if let Ok(items) = doc.SelectedItems() {
            let n = items.Count().unwrap_or(0).clamp(0, 64);
            for i in 0..n {
                if let Ok(p) = items.Item(&int_variant(i)).and_then(|it| it.Path()) {
                    t.selected.push(p.to_string());
                }
            }
        }
    }
    t
}

/// Every open Explorer tab (only those of window `only` when given), with its automation object.
fn tabs(only: Option<isize>) -> Res<Vec<(IWebBrowser2, ExplorerTab)>> {
    let sw: IShellWindows = unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }.map_err(|e| win_err("no Explorer windows list", &e))?;
    let n = unsafe { sw.Count() }.unwrap_or(0);
    let mut out = vec![];
    for i in 0..n {
        let Ok(d): windows::core::Result<IDispatch> = (unsafe { sw.Item(&int_variant(i)) }) else { continue };
        let Ok(wb) = d.cast::<IWebBrowser2>() else { continue };
        let h = unsafe { wb.HWND() }.map(|h| h.0).unwrap_or(0);
        if only.is_some_and(|o| o != h) {
            continue;
        }
        // only File Explorer windows (not other hosts of the shell browser)
        if class_of(HWND(h as *mut _)) != "CabinetWClass" {
            continue;
        }
        let t = tab_of(&wb);
        out.push((wb, t));
    }
    // the tab in front of each window is its first tab child in z-order
    let mut fronts: std::collections::HashMap<isize, isize> = std::collections::HashMap::new();
    for (_, t) in &out {
        fronts.entry(t.hwnd).or_insert_with(|| unsafe { FindWindowExW(Some(HWND(t.hwnd as *mut _)), None, w!("ShellTabWindowClass"), None) }.map(|h| h.0 as isize).unwrap_or(0));
    }
    let count = |h: isize| out.iter().filter(|(_, t)| t.hwnd == h).count();
    let single: Vec<bool> = out.iter().map(|(_, t)| count(t.hwnd) == 1).collect();
    for (i, (_, t)) in out.iter_mut().enumerate() {
        t.active = single[i] || (t.tab != 0 && fronts.get(&t.hwnd) == Some(&t.tab));
    }
    Ok(out)
}

/// Every open Explorer tab with its automation object (the probe closes the windows it opened through it).
pub fn tabs_with_browser() -> Res<Vec<(IWebBrowser2, ExplorerTab)>> {
    tabs(None)
}

/// The open Explorer tabs (read only; `wfm explorer` shows them).
pub fn list() -> Res<Vec<ExplorerTab>> {
    let _c = com();
    Ok(tabs(None)?.into_iter().map(|(_, t)| t).collect())
}

fn link_target(p: &str) -> Option<String> {
    crate::info::read_shortcut(Path::new(p)).map(|l| l.target).filter(|t| !t.is_empty())
}

/// Why a request does or does not go back to Explorer (`wfm explorer check` prints it; `hand_back` acts on it).
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// open here: the reason
    Here(&'static str),
    /// navigate this Explorer tab (window, tab) in place
    Navigate(isize, isize),
    /// several folders were opened from Explorer at once: each opens in a new Explorer window, as Explorer does
    NewWindow,
}

fn decide(target: &str, check_parent: bool) -> (Verdict, Option<IWebBrowser2>) {
    let t = target.trim().trim_matches('"');
    if t.starts_with("::") || t.starts_with('/') || t.starts_with("shell:") || !Path::new(t).is_dir() {
        return (Verdict::Here("not a folder path"), None);
    }
    let fg = unsafe { GetForegroundWindow() };
    if class_of(fg) != "CabinetWClass" {
        return (Verdict::Here("no Explorer window in front"), None);
    }
    if check_parent && !parent_is_explorer() {
        return (Verdict::Here("not started by Explorer"), None);
    }
    let Ok(all) = tabs(Some(fg.0 as isize)) else { return (Verdict::Here("Explorer did not answer"), None) };
    let Some((wb, tab)) = all.into_iter().find(|(_, t)| t.active) else { return (Verdict::Here("no active Explorer tab"), None) };
    if !wanted_by(&tab, t, link_target) {
        return (Verdict::Here("the folder is not what that Explorer window opened"), None);
    }
    if tab.selected.len() > 1 && tab.selected.iter().any(|s| norm(s) == norm(t)) {
        return (Verdict::NewWindow, None);
    }
    (Verdict::Navigate(tab.hwnd, tab.tab), Some(wb))
}

/// What `hand_back` would do for `target` right now, without doing it (`check_parent` false: from a terminal).
pub fn check(target: &str, check_parent: bool) -> Verdict {
    let _c = com();
    decide(target, check_parent).0
}

/// Called first thing by RustFileExplorer.exe with the folder it was asked to open: when the request came from an
/// Explorer window, that window opens the folder itself and this returns true (the caller then exits). Explorer gets
/// 1.5 s to answer; a hung Explorer never keeps the folder from opening here.
pub fn hand_back(target: &str) -> bool {
    // cheap checks first: almost every start leaves here without any COM call
    let fg = unsafe { GetForegroundWindow() };
    if class_of(fg) != "CabinetWClass" {
        return false;
    }
    let target = target.trim().trim_matches('"').to_string();
    // a loop guard: the same folder handed back a moment ago comes here instead (should Explorer ever answer a
    // navigation with our verb again, it ends after one round)
    let mark = std::env::temp_dir().join("wfm-handback.txt");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    if let Ok(s) = std::fs::read_to_string(&mark)
        && let Some((k, t)) = s.split_once('\n')
        && k == norm(&target)
        && now.saturating_sub(t.trim().parse::<u128>().unwrap_or(0)) < 1500
    {
        return false;
    }
    let key = norm(&target);
    let gave_up = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let g = gave_up.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _c = com();
        let (v, wb) = decide(&target, true);
        let done = match v {
            _ if g.load(std::sync::atomic::Ordering::SeqCst) => false,
            Verdict::Navigate(..) => wb.map(|wb| unsafe { wb.Navigate2(&VARIANT::from(target.as_str()), None, None, None, None) }.is_ok()).unwrap_or(false),
            // Explorer's own verb, kept next to ours by `defaultfm::set` (never ours, so no loop)
            Verdict::NewWindow => crate::shell::open_verb(Path::new(&target), "openinexplorer", "").is_ok(),
            Verdict::Here(_) => false,
        };
        let _ = tx.send(done);
    });
    match rx.recv_timeout(std::time::Duration::from_millis(1500)) {
        Ok(done) => {
            if done {
                let _ = std::fs::write(&mark, format!("{key}\n{now}"));
            }
            done
        }
        Err(_) => {
            gave_up.store(true, std::sync::atomic::Ordering::SeqCst);
            false
        }
    }
}

// ------------------------------------------------------------------ the start argument of RustFileExplorer.exe

/// What RustFileExplorer.exe does with the argument it was started with (see [`classify`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    /// no argument: the start page
    Start,
    /// a real folder or drive: opens here (after [`hand_back`])
    Open(std::path::PathBuf),
    /// a file (or "/select,<item>"): its folder opens here with the item selected
    Reveal(std::path::PathBuf),
    /// This PC / Recycle Bin ("::{CLSID}" or "shell:" name): our own pages
    Special(String),
    /// a file system path that does not exist: opens here (the window says "Not found", as before)
    Missing(std::path::PathBuf),
    /// anything else ("::{CLSID}" places, other "shell:" names, phones, libraries): Explorer opens it, never through
    /// our own `open` / `explore` verbs
    Explorer(String),
}

/// What a path is on disk (injected so [`classify`] is testable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Dir,
    File,
    Missing,
}

/// This PC and the Recycle Bin, as "::{CLSID}" or "shell:" names: pages of our own window.
pub fn is_our_special(s: &str) -> bool {
    let s = s.trim().trim_matches('"').to_ascii_lowercase();
    let s = s.strip_prefix("shell:").unwrap_or(&s);
    let s = s.trim_start_matches(':');
    matches!(s, "{20d04fe0-3aea-1069-a2d8-08002b30309d}" | "mycomputerfolder" | "{645ff040-5081-101b-9f08-00aa002f954e}" | "recyclebinfolder")
}

/// Explorer's command-line switches in front of a path: "/select,<item>", "/e,<dir>", "/n,", "/separate,",
/// "/root,<dir>", in any combination ("/e,/select,C:\x"). Returns (select, the path left).
pub fn parse_switches(arg: &str) -> (bool, String) {
    let mut s = arg.trim().trim_matches('"').trim();
    let mut select = false;
    let mut root = String::new();
    while let Some(rest) = s.strip_prefix('/') {
        let (sw, after) = rest.split_once(',').unwrap_or((rest, ""));
        let sw = sw.trim().to_ascii_lowercase();
        if !matches!(sw.as_str(), "select" | "e" | "n" | "separate" | "root") {
            break;
        }
        s = after.trim().trim_matches('"').trim();
        match sw.as_str() {
            "select" => select = true,
            "root" => {
                // "/root,<dir>[,<path>]": the root is what opens when nothing follows
                let (r, more) = s.split_once(',').unwrap_or((s, ""));
                root = r.trim().trim_matches('"').to_string();
                s = more.trim().trim_matches('"').trim();
            }
            _ => {}
        }
    }
    let path = if s.is_empty() { root } else { s.to_string() };
    (select, path)
}

/// A drive-letter or UNC share path ("C:\x", "C:", "\\server\share\x"); a bare "\\server" is a network computer
/// (virtual: Explorer).
fn fs_like(p: &str) -> bool {
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return b.len() == 2 || b[2] == b'\\' || b[2] == b'/';
    }
    if let Some(rest) = p.strip_prefix(r"\\").or_else(|| p.strip_prefix("//")) {
        return !rest.starts_with('?') && !rest.starts_with('.') && rest.trim_end_matches(['\\', '/']).contains(['\\', '/']);
    }
    false
}

/// Sort out the start argument. `kind` looks at the disk, `resolve` turns a shell name ("shell:Downloads",
/// "::{...}\C:\x", "file:///...") into a file system path when it is one.
pub fn classify(arg: &str, kind: impl Fn(&str) -> Kind, resolve: impl Fn(&str) -> Option<String>) -> Launch {
    let (select, p) = parse_switches(arg);
    let p = p.trim().to_string();
    if p.is_empty() {
        return Launch::Start;
    }
    if is_our_special(&p) {
        return Launch::Special(p);
    }
    // "C:" alone means the drive, never "the current folder on C:"
    let p = if p.len() == 2 && p.ends_with(':') { format!(r"{p}\") } else { p };
    let on_disk = |q: &str| match kind(q) {
        Kind::Dir if select => Some(Launch::Reveal(q.into())),
        Kind::Dir => Some(Launch::Open(q.into())),
        Kind::File => Some(Launch::Reveal(q.into())),
        Kind::Missing => None,
    };
    if fs_like(&p) {
        return on_disk(&p).unwrap_or_else(|| Launch::Missing(p.into()));
    }
    if let Some(q) = resolve(&p)
        && let Some(l) = on_disk(&q)
    {
        return l;
    }
    Launch::Explorer(p)
}

/// [`classify`] against the real disk and shell.
pub fn classify_here(arg: &str) -> Launch {
    classify(
        arg,
        |p| {
            let p = Path::new(p);
            if p.is_dir() {
                Kind::Dir
            } else if p.exists() {
                Kind::File
            } else {
                Kind::Missing
            }
        },
        shell_path,
    )
}

/// The file system path behind a shell name, if it has one.
pub fn shell_path(name: &str) -> Option<String> {
    use windows::Win32::UI::Shell::{ILFree, SHGetPathFromIDListW, SHParseDisplayName};
    let _c = com();
    let w = wide(name);
    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(windows::core::PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).ok()?;
        let mut buf = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, &mut buf).as_bool();
        ILFree(Some(pidl));
        let p = from_wide(&buf);
        (ok && !p.is_empty()).then_some(p)
    }
}

/// Loop guard: the same argument handed to Explorer twice within 2 s (Explorer gave it back to us) stops there.
/// `now` in ms; the mark is "key\nms" in `mark`. True = stop.
pub fn seen_twice(mark: &Path, key: &str, now: u128) -> bool {
    let key = key.trim().to_lowercase();
    let again = std::fs::read_to_string(mark)
        .ok()
        .and_then(|s| {
            let (k, t) = s.split_once('\n')?;
            Some(k == key && now.saturating_sub(t.trim().parse::<u128>().ok()?) < 2000)
        })
        .unwrap_or(false);
    let _ = std::fs::write(mark, format!("{key}\n{now}"));
    again
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Give a place to Windows' Explorer: Explorer's own `openinexplorer` delegate verb on the item (never our `open` /
/// `explore`), else Explorer's delegate object directly, else explorer.exe. Stops (false) when the same place came
/// back within 2 s.
pub fn to_explorer(place: &str) -> bool {
    if seen_twice(&std::env::temp_dir().join("wfm-to-explorer.txt"), place, now_ms()) {
        probe_note(place, "loop guard: stopped");
        return false;
    }
    let _c = com();
    let how = if open_in_explorer_verb(place) {
        "Explorer (openinexplorer verb)"
    } else if open_in_explorer_delegate(place) {
        "Explorer (delegate)"
    } else if std::process::Command::new(explorer_exe()).arg(place).spawn().is_ok() {
        "Explorer (explorer.exe)"
    } else {
        "failed"
    };
    probe_note(place, how);
    how != "failed"
}

fn explorer_exe() -> String {
    format!(r"{}\explorer.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
}

fn parse_pidl(place: &str) -> Option<*mut windows::Win32::UI::Shell::Common::ITEMIDLIST> {
    let w = wide(place);
    let mut pidl = std::ptr::null_mut();
    unsafe { windows::Win32::UI::Shell::SHParseDisplayName(windows::core::PCWSTR(w.as_ptr()), None, &mut pidl, 0, None) }.ok()?;
    (!pidl.is_null()).then_some(pidl)
}

/// ShellExecuteEx(verb "openinexplorer", the item's ID list): HKCU Directory / Drive / Folder carry it while we
/// are the default.
fn open_in_explorer_verb(place: &str) -> bool {
    use windows::Win32::UI::Shell::*;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let Some(pidl) = parse_pidl(place) else { return false };
    let verb = wide("openinexplorer");
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_IDLIST | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: windows::core::PCWSTR(verb.as_ptr()),
        lpIDList: pidl as *mut _,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    let ok = unsafe { ShellExecuteExW(&mut sei) }.is_ok();
    unsafe { ILFree(Some(pidl)) };
    ok
}

/// Explorer's delegate object ({11dbb47c-...}, "ExecuteFolder", the one its own Folder\shell\open names) run on
/// the item.
fn open_in_explorer_delegate(place: &str) -> bool {
    use windows::Win32::System::Com::CLSCTX_INPROC_SERVER;
    use windows::Win32::UI::Shell::*;
    let Some(pidl) = parse_pidl(place) else { return false };
    let r = (|| -> windows::core::Result<()> {
        unsafe {
            let clsid = windows::core::GUID::from_u128(0x11dbb47c_a525_400b_9e80_a54615a090c0);
            let cmd: IExecuteCommand = CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER)?;
            let arr = SHCreateShellItemArrayFromIDLists(&[pidl as *const _])?;
            cmd.cast::<IObjectWithSelection>()?.SetSelection(&arr)?;
            let _ = cmd.SetShowWindow(windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0);
            cmd.Execute()
        }
    })();
    unsafe { ILFree(Some(pidl)) };
    r.is_ok()
}

/// "Open in Windows File Explorer" from our own menu: always Windows' Explorer, whatever the default is.
pub fn open_in_explorer(place: &str) -> Res<()> {
    let _c = com();
    if open_in_explorer_verb(place) || open_in_explorer_delegate(place) || std::process::Command::new(explorer_exe()).arg(place).spawn().is_ok() {
        Ok(())
    } else {
        Err(format!("File Explorer could not open {place}"))
    }
}

/// `wfm default probe` asks every RustFileExplorer.exe it reaches to note what it got: while the marker
/// %TEMP%\wfm-probe-active is younger than 10 minutes, one line per start goes to %TEMP%\wfm-probe-log.txt.
pub fn probe_note(arg: &str, what: &str) {
    let dir = std::env::temp_dir();
    let fresh = std::fs::metadata(dir.join("wfm-probe-active")).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() < 600);
    if !fresh {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("wfm-probe-log.txt")) {
        let _ = writeln!(f, "{}\t{}\t{}\t{}", now_ms(), std::process::id(), arg, what);
    }
}

/// Diagnostics (`wfm explorer find <folder>`): which shell window the ShellWindows list gives for a folder, per
/// window kind, and whether it answers the way "Show in folder" asks (IServiceProvider -> IShellBrowser ->
/// IShellView).
pub fn find_shell_window(place: &str) -> Vec<String> {
    use windows::Win32::System::Variant::InitVariantFromBuffer;
    use windows::Win32::UI::Shell::{ILFree, ILGetSize, IShellView, SHParseDisplayName, SWFO_NEEDDISPATCH, ShellWindowFindWindowOptions, ShellWindowTypeConstants};
    let _c = com();
    let mut out = vec![];
    let Ok(sw): windows::core::Result<IShellWindows> = (unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }) else { return vec!["no ShellWindows".into()] };
    out.push(format!("count {}", unsafe { sw.Count() }.unwrap_or(-1)));
    let w = wide(place);
    let v = unsafe {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(windows::core::PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_err() {
            return vec!["cannot parse".into()];
        }
        let v = InitVariantFromBuffer(pidl as *const _, ILGetSize(Some(pidl)));
        ILFree(Some(pidl));
        v
    };
    let Ok(v) = v else { return vec!["no variant".into()] };
    for kind in [0, 1, 2, 8] {
        let t = std::time::Instant::now();
        let mut hwnd = 0i32;
        let r = unsafe { sw.FindWindowSW(&v, &VARIANT::default(), ShellWindowTypeConstants(kind), &mut hwnd, ShellWindowFindWindowOptions(SWFO_NEEDDISPATCH.0)) };
        let mut line = format!("class {kind}: {} hwnd {hwnd:#x} ({} ms)", if r.is_ok() { "found" } else { "none" }, t.elapsed().as_millis());
        if let Ok(d) = r {
            let chain = d.cast::<IServiceProvider>().and_then(|sp| unsafe { sp.QueryService::<IShellBrowser>(&SID_STopLevelBrowser) }).and_then(|sb| unsafe { sb.QueryActiveShellView() });
            line += &format!("; view: {}", match chain {
                Ok(_v) => {
                    let _: IShellView = _v;
                    "ok".to_string()
                }
                Err(e) => format!("{e}"),
            });
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(loc: &str, sel: &[&str]) -> ExplorerTab {
        ExplorerTab { location: loc.into(), selected: sel.iter().map(|s| s.to_string()).collect(), active: true, ..Default::default() }
    }

    #[test]
    fn norm_folds_case_slashes_and_quotes() {
        assert_eq!(norm(r#""C:\Users\Me\""#), r"c:\users\me");
        assert_eq!(norm(r"C:\"), "c:");
        assert_eq!(norm("D:/Music/"), r"d:\music");
    }

    #[test]
    fn selected_folder_goes_back_to_explorer() {
        let t = tab(r"C:\Users\Me", &[r"C:\Users\Me\Music"]);
        assert!(wanted_by(&t, r"c:\users\me\music\", |_| None));
        // a folder in another place, not selected there: opens here
        let t = tab(r"C:\Users\Me", &[r"C:\Users\Me\Music"]);
        assert!(!wanted_by(&t, r"D:\Games", |_| None));
    }

    #[test]
    fn search_results_and_home_match_by_selection() {
        // Home / search results: no file system location, the selected item decides
        let t = tab("", &[r"E:\Work\Project"]);
        assert!(wanted_by(&t, r"E:\Work\Project", |_| None));
        assert!(!wanted_by(&tab("", &[]), r"E:\Work\Project", |_| None));
    }

    #[test]
    fn child_of_the_shown_folder_counts_without_selection() {
        // "Open" from the folder tree or a selection Explorer did not report yet
        let t = tab(r"C:\Data", &[]);
        assert!(wanted_by(&t, r"C:\Data\Photos", |_| None));
        assert!(!wanted_by(&t, r"C:\Data", |_| None));
        assert!(!wanted_by(&t, r"C:\Other\Photos", |_| None));
        // drive root shown: its folders
        assert!(wanted_by(&tab(r"D:\", &[]), r"D:\Games", |_| None));
    }

    #[test]
    fn selected_shortcut_to_the_folder_counts() {
        let t = tab(r"C:\Users\Me\Desktop", &[r"C:\Users\Me\Desktop\Music.lnk"]);
        assert!(wanted_by(&t, r"D:\Music", |p| (p.ends_with("Music.lnk")).then(|| r"D:\Music".to_string())));
        assert!(!wanted_by(&t, r"D:\Video", |p| (p.ends_with("Music.lnk")).then(|| r"D:\Music".to_string())));
    }

    fn disk(p: &str) -> Kind {
        match p.to_lowercase().as_str() {
            r"c:\data" | r"c:\" | r"d:\" | r"\\srv\share\docs" | r"c:\users\me\downloads" => Kind::Dir,
            r"c:\data\a b.txt" => Kind::File,
            _ => Kind::Missing,
        }
    }

    fn shell(p: &str) -> Option<String> {
        match p.to_lowercase().as_str() {
            "shell:downloads" => Some(r"C:\Users\Me\Downloads".into()),
            "file:///c:/data" => Some(r"C:\Data".into()),
            _ => None,
        }
    }

    fn c(arg: &str) -> Launch {
        classify(arg, disk, shell)
    }

    #[test]
    fn folders_and_drives_open_here() {
        assert_eq!(c(r"C:\Data"), Launch::Open(r"C:\Data".into()));
        assert_eq!(c(r#""C:\Data""#), Launch::Open(r"C:\Data".into()));
        assert_eq!(c("D:"), Launch::Open(r"D:\".into()));
        assert_eq!(c(r"C:\"), Launch::Open(r"C:\".into()));
        assert_eq!(c(r"\\srv\share\docs"), Launch::Open(r"\\srv\share\docs".into()));
        assert_eq!(c(""), Launch::Start);
        assert_eq!(c(r"C:\Data\a b.txt"), Launch::Reveal(r"C:\Data\a b.txt".into()));
    }

    #[test]
    fn explorer_switches_select_and_open() {
        assert_eq!(c(r"/select,C:\Data\a b.txt"), Launch::Reveal(r"C:\Data\a b.txt".into()));
        assert_eq!(c(r#"/select,"C:\Data\a b.txt""#), Launch::Reveal(r"C:\Data\a b.txt".into()));
        assert_eq!(c(r"/select, C:\Data\a b.txt"), Launch::Reveal(r"C:\Data\a b.txt".into()));
        assert_eq!(c(r"/e,C:\Data"), Launch::Open(r"C:\Data".into()));
        assert_eq!(c(r"/n,/e,C:\Data"), Launch::Open(r"C:\Data".into()));
        assert_eq!(c(r"/e,/select,C:\Data\a b.txt"), Launch::Reveal(r"C:\Data\a b.txt".into()));
        // "/select,<folder>" selects the folder in its parent
        assert_eq!(c(r"/select,C:\Data"), Launch::Reveal(r"C:\Data".into()));
        assert_eq!(c(r"/root,C:\Data"), Launch::Open(r"C:\Data".into()));
        assert_eq!(c("/e,"), Launch::Start);
        assert_eq!(parse_switches(r"/x,C:\a"), (false, r"/x,C:\a".into()), "unknown switches are left alone");
    }

    #[test]
    fn virtual_places_go_to_explorer() {
        let cp = "::{26EE0668-A00A-44D7-9371-BEB064C98683}";
        assert_eq!(c(cp), Launch::Explorer(cp.into()));
        assert_eq!(c("shell:ControlPanelFolder"), Launch::Explorer("shell:ControlPanelFolder".into()));
        assert_eq!(c("::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}"), Launch::Explorer("::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}".into()));
        // a phone (MTP) below This PC is not This PC itself
        let phone = r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\\?\usb#vid_04e8&pid_6860#r58m";
        assert_eq!(c(phone), Launch::Explorer(phone.into()));
        assert_eq!(c(r"\\srv"), Launch::Explorer(r"\\srv".into()), "a network computer");
        assert_eq!(c("/select,::{645FF040-5081-101B-9F08-00AA002F954E}"), Launch::Special("::{645FF040-5081-101B-9F08-00AA002F954E}".into()));
    }

    #[test]
    fn our_pages_and_shell_names_with_a_folder() {
        assert_eq!(c("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"), Launch::Special("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}".into()));
        assert_eq!(c("shell:MyComputerFolder"), Launch::Special("shell:MyComputerFolder".into()));
        assert_eq!(c("shell:RecycleBinFolder"), Launch::Special("shell:RecycleBinFolder".into()));
        assert_eq!(c("shell:Downloads"), Launch::Open(r"C:\Users\Me\Downloads".into()));
        assert_eq!(c("file:///C:/Data"), Launch::Open(r"C:\Data".into()));
        assert_eq!(c(r"C:\Gone\Away"), Launch::Missing(r"C:\Gone\Away".into()));
    }

    #[test]
    fn loop_guard_stops_the_second_round() {
        let mark = std::env::temp_dir().join(format!("wfm-test-loop-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&mark);
        assert!(!seen_twice(&mark, "::{X}", 10_000));
        assert!(seen_twice(&mark, "::{x}", 11_500), "same place within 2 s");
        assert!(!seen_twice(&mark, "::{x}", 14_000), "later is fine again");
        assert!(!seen_twice(&mark, "::{Y}", 14_100), "another place");
        let _ = std::fs::remove_file(&mark);
    }
}
