#![windows_subsystem = "windows"]
//! wfiles.exe: the Win Tools Files window (Win32 + Direct2D, no webview).
//! Args: [folder or file] [--theme light|dark] [--size WxH] [--gpu] [--no-mica] [--new-window]
//! A second start sends its folder to the open window as a new tab (unless --new-window).
//! A crash writes %TEMP%\wfiles-crash.log (there is no console to print to).
mod ai;
mod app;
mod dnd;
mod ficons;
mod fmt;
mod gfx;
mod input;
mod jobs;
mod known;
mod model;
mod models;
mod paint;
mod preview;
mod theme;

/// Plain HTTP GET to a local server (semsearch on 127.0.0.1), short timeout.
pub fn ai_http_get(url: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").ok_or("http only")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let addr: std::net::SocketAddr = host.parse().map_err(|_| "bad host".to_string())?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(60)));
    write!(s, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").map_err(|e| e.to_string())?;
    let mut buf = vec![];
    s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("bad reply")?;
    if !head.starts_with("HTTP/1.") || !head.split_whitespace().nth(1).map(|c| c == "200").unwrap_or(false) {
        return Err("HTTP error".into());
    }
    Ok(body.to_string())
}

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
        let p = path.clone().map(|p| fmcore::util::abs(p.trim_matches('"').trim_start_matches("/select,")).display().to_string());
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
    };
    let code = app::run(o);
    std::process::exit(code);
}
