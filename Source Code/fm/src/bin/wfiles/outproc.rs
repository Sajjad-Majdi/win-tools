//! Shell work that loads other people's code (context-menu extensions: 7-Zip, Git, VS Code, Defender ...; "Open
//! with" handlers) runs in a short-lived `wfm.exe` next to this exe, never in the window: those DLLs, their threads
//! and their heaps (about 20 MB) would otherwise stay in the window's memory for the rest of the session.
//! When wfm.exe is missing the same work runs in-process (the old way).
use fmcore::menu::{Icon, MenuItem};
use fmcore::shell::Handler;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub type Apps = Vec<(Handler, Option<Icon>)>;

fn wfm_exe() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.with_file_name("wfm.exe");
    p.is_file().then_some(p)
}

/// "W,H,hex" (premultiplied BGRA) -> icon.
fn icon(s: &str) -> Option<Icon> {
    let mut it = s.splitn(3, ',');
    let w: u32 = it.next()?.parse().ok()?;
    let h: u32 = it.next()?.parse().ok()?;
    let hex = it.next()?.as_bytes();
    if hex.len() != (w * h * 4 * 2) as usize {
        return None;
    }
    let val = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => 0,
    };
    let bgra = hex.chunks_exact(2).map(|p| (val(p[0]) << 4) | val(p[1])).collect();
    Some(Icon { width: w, height: h, bgra })
}

/// The menu's shell entries and (for one file) the apps of its type, with their icons.
pub fn menu_data(paths: &[PathBuf], ext: Option<&str>) -> (Vec<MenuItem>, Apps) {
    if let Some(exe) = wfm_exe() {
        let mut c = fmcore::util::hidden_command(&exe);
        c.arg("menu-data").args(paths);
        if let Some(e) = ext {
            c.arg("--apps").arg(e);
        }
        if let Ok(out) = c.output()
            && let Ok(v) = serde_json::from_slice::<Value>(&out.stdout)
            && v["ok"] == true
        {
            let items = v["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|i| MenuItem {
                            id: 0,
                            verb: i["verb"].as_str().unwrap_or("").into(),
                            text: i["text"].as_str().unwrap_or("").into(),
                            parent: i["parent"].as_str().unwrap_or("").into(),
                            enabled: i["enabled"].as_bool().unwrap_or(true),
                            icon: i["icon"].as_str().and_then(icon),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let apps = v["apps"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|h| {
                            let hd = Handler {
                                name: h["name"].as_str().unwrap_or("").into(),
                                ui_name: h["ui_name"].as_str().unwrap_or("").into(),
                                recommended: h["recommended"].as_bool().unwrap_or(false),
                                icon: h["icon_loc"].as_str().unwrap_or("").into(),
                                default: h["default"].as_bool().unwrap_or(false),
                            };
                            (hd, h["icon"].as_str().and_then(icon))
                        })
                        .collect()
                })
                .unwrap_or_default();
            return (items, apps);
        }
    }
    // no wfm.exe: in-process
    let items = fmcore::menu::list(paths, false).unwrap_or_default();
    let apps = match ext {
        Some(e) => fmcore::shell::open_with_list(e, false).unwrap_or_default().into_iter().take(12).map(|h| {
            let i = fmcore::menu::handler_icon(&h);
            (h, i)
        }).collect(),
        None => vec![],
    };
    (items, apps)
}

/// Run one shell entry (by verb or text) on the items, out of process.
pub fn invoke(paths: &[PathBuf], verb: &str) -> Result<(), String> {
    let Some(exe) = wfm_exe() else {
        let _c = fmcore::util::com();
        return fmcore::menu::invoke(paths, verb, false).map(|_| ());
    };
    let out = fmcore::util::hidden_command(&exe).arg("menu").args(paths).arg("--invoke").arg(verb).output().map_err(|e| format!("cannot run wfm.exe: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("error: ").to_string()) }
}

/// Open a file with one app ("Open with" row), out of process.
pub fn open_with(path: &Path, app: &str) -> Result<(), String> {
    let Some(exe) = wfm_exe() else {
        return fmcore::shell::open_with(path, app).map(|_| ());
    };
    let out = fmcore::util::hidden_command(&exe).arg("openwith").arg(path).arg("--app").arg(app).output().map_err(|e| format!("cannot run wfm.exe: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("error: ").to_string()) }
}
