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
mod rank;
mod sicons;
mod theme;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let p = std::env::temp_dir().join("wfiles-crash.log");
        let _ = std::fs::write(p, format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));
    fmcore::util::quiet_errors();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let mut path = None;
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
        path = Some(a.clone());
    }
    // one window: a second start opens its folder as a tab in the running window
    if !flag("--new-window") && std::env::var("WFILES_TEST").is_err() && fmcore::pipe::exists() {
        let p = path.clone().map(|p| {
            let p = p.trim_matches('"').trim_start_matches("/select,");
            // "::{CLSID}" / "shell:..." names of This PC and the Recycle Bin pass through as they are
            if model::special_loc(p).is_some() { p.to_string() } else { fmcore::util::abs(p).display().to_string() }
        });
        let cmd = match p {
            Some(p) => format!("tab new {p}"),
            None => "tab new".into(),
        };
        if fmcore::pipe::send(&cmd).is_ok() {
            return;
        }
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
