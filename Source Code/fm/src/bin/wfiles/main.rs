#![windows_subsystem = "windows"]
#![allow(non_snake_case)]
//! RustFileExplorer.exe: the Rust File Explorer window (Win32 + Direct2D, no webview).
//! Args: [folder or file] [--theme light|dark] [--size WxH] [--gpu] [--no-mica] [--new-window] [--offscreen]
//! --offscreen (tests): the real window on the real desktop, but far off the screen, without a taskbar button, never
//! activated; `wfm gui shot-real` then captures it as Windows composes it (caption buttons, backdrop).
//! A second start sends its folder to the open window as a new tab (unless --new-window).
//! A crash writes %TEMP%\wfiles-crash.log (there is no console to print to).
use fmcore::{ai, models};
mod app;
mod ask;
mod cleanup_page;
mod drives;
mod dnd;
mod ficons;
mod fmt;
mod gfx;
mod input;
mod jobs;
mod known;
mod look;
mod model;
mod motion;
mod outproc;
mod paint;
mod preview;
mod quick;
mod rank;
mod sicons;
mod theme;
mod upd;
mod viewer;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let p = std::env::temp_dir().join("wfiles-crash.log");
        let _ = std::fs::write(p, format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));
    fmcore::util::quiet_errors();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let flag = |name: &str| args.iter().any(|a| a == name);
    // the positional words: Explorer-style switches may arrive split at spaces ("/select," "C:\a b\x.txt")
    let mut words: Vec<String> = vec![];
    let mut skip = false;
    for a in &args {
        if skip {
            skip = false;
            continue;
        }
        if a == "--theme" || a == "--size" {
            skip = true;
            continue;
        }
        if a.starts_with("--") {
            continue;
        }
        words.push(a.clone());
    }
    let raw = if words.first().is_some_and(|w| w.starts_with('/')) { Some(words.join(" ")) } else { words.last().cloned() };
    // the argument decides: a folder or drive opens here, "/select,<file>" opens its folder with the file selected,
    // This PC / Recycle Bin are our own pages, anything else (Control Panel, Network, phones ...) goes to Explorer
    let launch = raw.as_deref().map(fmcore::explorer::classify_here).unwrap_or(fmcore::explorer::Launch::Start);
    fmcore::explorer::probe_note(raw.as_deref().unwrap_or(""), &format!("{launch:?}"));
    // keys left pointing at an exe that is gone: Explorer is put back (this exe may be a copy elsewhere)
    if std::env::var("WFILES_TEST").is_err() {
        let _ = fmcore::defaultfm::guard_missing_exe();
    }
    use fmcore::explorer::Launch;
    let path: Option<String> = match &launch {
        Launch::Start => None,
        Launch::Explorer(place) => {
            let _ = fmcore::explorer::to_explorer(place);
            return;
        }
        Launch::Open(p) | Launch::Missing(p) => Some(p.display().to_string()),
        // the window opens the item's folder with the item selected (a folder too: selected in its parent)
        Launch::Reveal(p) => Some(format!("/select,{}", p.display())),
        Launch::Special(s) => Some(s.clone()),
    };
    // a folder double-clicked inside a Windows Explorer window stays in that window (our "open" verb is the default)
    if let Launch::Open(p) = &launch
        && !flag("--new-window")
        && std::env::var("WFILES_TEST").is_err()
        && fmcore::explorer::hand_back(&p.display().to_string())
    {
        fmcore::explorer::probe_note(&p.display().to_string(), "handed back to the Explorer window");
        return;
    }
    // one window: a second start opens its folder as a tab in the running window (a file: its folder, selected)
    if !flag("--new-window") && std::env::var("WFILES_TEST").is_err() && fmcore::pipe::exists() {
        let p = path.clone().map(|p| {
            // "::{CLSID}" / "shell:..." names of This PC and the Recycle Bin pass through as they are
            if model::special_loc(&p).is_some() {
                p
            } else if let Some(item) = p.strip_prefix("/select,") {
                format!("/select,{}", fmcore::util::abs(item).display())
            } else {
                fmcore::util::abs(&p).display().to_string()
            }
        });
        let cmd = match p {
            Some(p) => format!("tab new {p}"),
            None => "tab new".into(),
        };
        // this process got the foreground right from the click that started it; pass it on to the running window
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
        }
        if fmcore::pipe::send(&cmd).is_ok() {
            let _ = fmcore::pipe::send("activate");
            return;
        }
    }
    // this process is the window: its own taskbar identity, then (in the background) its jump list and the Start
    // menu shortcut's matching ID
    if !flag("--offscreen") && std::env::var("WFILES_TEST").is_err() {
        fmcore::jumplist::set_process_id();
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(3));
            if let Ok(exe) = std::env::current_exe() {
                let _ = fmcore::jumplist::update(&exe);
                let _ = fmcore::jumplist::fix_shortcut(&exe);
            }
        });
    }
    let size = opt("--size").and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    });
    let o = app::Opts {
        // CPU drawing by default: starts ~35 ms sooner and skips the GPU driver memory; --gpu uses Direct3D
        software: !flag("--gpu") && std::env::var("WFILES_GPU").is_err(),
        theme: opt("--theme").and_then(|t| match t.as_str() {
            "light" => Some(false),
            "dark" => Some(true),
            _ => None,
        }),
        size,
        path,
        no_mica: flag("--no-mica"),
        offscreen: flag("--offscreen"),
    };
    let code = app::run(o);
    std::process::exit(code);
}
