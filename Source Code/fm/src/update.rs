//! Built-in updates (the same file is in the Task Manager's app/src/update.rs).
//! - Reads one small manifest, `MANIFEST`: { "<app key>": { "version": "0.2.10", "url": "https://...-setup.exe",
//!   "size": 3711316, "notes": ["...", "..."] }, ... }. App keys: "fm" (Rust File Explorer), "tm" (Rust Task Manager).
//! - Only a newer version is offered. "Later" hides it for a day; nothing is downloaded until the user clicks Update.
//! - Update downloads the setup to %TEMP%, checks its size and that it is an exe, then runs it silently (/S /RUN:
//!   the setup starts the app again when it is done); the app itself closes right after.
//! - Honours HTTPS_PROXY / https_proxy, else the system proxy (WinHTTP / Internet Options).
//! - Builds made with WT_NO_UPDATE set at compile time (a local, personal build) never check.
//! Tests: WT_UPDATE_URL (another https manifest) and WT_UPDATE_AS (pretend to be that version) at run time.
//! State: %LOCALAPPDATA%\WinTools\update-<key>.json { "snooze_until": unix seconds, "snooze_ver": "0.2.10" }.
use serde_json::{Value, json};
use std::ffi::c_void;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Networking::WinHttp::*;
use windows::core::PCWSTR;

pub const MANIFEST: &str = "https://sajjad-majdi.ource--ode.workers.dev/dl/wintools.json";
/// How long "Later" hides an offered version.
const SNOOZE_SECS: u64 = 24 * 3600;
/// A local build (WT_NO_UPDATE at compile time) never checks.
pub const ENABLED: bool = option_env!("WT_NO_UPDATE").is_none();

#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub notes: Vec<String>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn state_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("WinTools").join(format!("update-{key}.json"))
}

fn state(key: &str) -> Value {
    std::fs::read_to_string(state_path(key)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

/// "0.2.10" -> [0, 2, 10]; anything after a '-' or '+' is ignored.
fn parse(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// `a` is a newer version than `b`.
pub fn newer(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (parse(a), parse(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x > y
}

/// The manifest entry for `key` when it is newer than `current` (None: up to date). Pure, for tests.
pub fn offer_from(manifest: &str, key: &str, current: &str) -> Result<Option<Offer>, String> {
    let v: Value = serde_json::from_str(manifest).map_err(|_| "the update list is not readable".to_string())?;
    let e = &v[key];
    let Some(version) = e["version"].as_str() else { return Ok(None) };
    let url = e["url"].as_str().unwrap_or("");
    if !url.starts_with("https://") || !newer(version, current) {
        return Ok(None);
    }
    let notes = e["notes"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Ok(Some(Offer { version: version.to_string(), url: url.to_string(), size: e["size"].as_u64().unwrap_or(0), notes }))
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Ask the server for a newer version than `current` (the app's own version). `force` (asked for by hand)
/// ignores "Later" and a local build.
pub fn check(key: &str, current: &str, force: bool) -> Result<Option<Offer>, String> {
    if !ENABLED && !force {
        return Ok(None);
    }
    let current = env("WT_UPDATE_AS").unwrap_or_else(|| current.to_string());
    let current = current.as_str();
    let url = env("WT_UPDATE_URL").filter(|u| u.starts_with("https://")).unwrap_or_else(|| MANIFEST.to_string());
    let body = get(&format!("{url}?v={current}"), None, &|_, _| true)?;
    let o = offer_from(&String::from_utf8_lossy(&body), key, current)?;
    if !force && let Some(o) = &o {
        let s = state(key);
        if s["snooze_ver"].as_str() == Some(o.version.as_str()) && s["snooze_until"].as_u64().unwrap_or(0) > now() {
            return Ok(None);
        }
    }
    Ok(o)
}

/// "Later": hide this version for a day.
pub fn snooze(key: &str, version: &str) {
    let p = state_path(key);
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(Path::new(".")));
    let _ = std::fs::write(p, json!({ "snooze_until": now() + SNOOZE_SECS, "snooze_ver": version }).to_string());
}

/// Download the setup to %TEMP%. `progress(done, total)` returns false to cancel.
pub fn download(o: &Offer, progress: &dyn Fn(u64, u64) -> bool) -> Result<PathBuf, String> {
    let name = o.url.rsplit('/').next().filter(|n| n.to_lowercase().ends_with(".exe") && !n.contains(['\\', ':'])).unwrap_or("WinTools-setup.exe");
    let path = std::env::temp_dir().join(name);
    let part = path.with_extension("part");
    let mut f = std::fs::File::create(&part).map_err(|e| format!("cannot save the update: {e}"))?;
    let r = get(&o.url, Some(&mut f), &|d, t| progress(d, if t > 0 { t } else { o.size }));
    drop(f);
    let fail = |e: String| {
        let _ = std::fs::remove_file(&part);
        Err(e)
    };
    if let Err(e) = r {
        return fail(e);
    }
    let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let mz = std::fs::read(&part).map(|b| b.starts_with(b"MZ")).unwrap_or(false);
    if (o.size > 0 && len != o.size) || !mz {
        return fail("the download was incomplete, try again".into());
    }
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&part, &path).map_err(|e| format!("cannot save the update: {e}"))?;
    Ok(path)
}

/// Run the downloaded setup silently; its /RUN starts the app again when the files are in place. The caller
/// closes the app right after (the setup moves the running exe aside, so it never waits on it).
pub fn install(setup: &Path) -> Result<(), String> {
    std::process::Command::new(setup).args(["/S", "/RUN"]).spawn().map(|_| ()).map_err(|e| format!("cannot start the update: {e}"))
}

// ------------------------------------------------------------------------------------------------ HTTP GET (WinHTTP)
struct Handles([*mut c_void; 3]);
impl Drop for Handles {
    fn drop(&mut self) {
        for h in self.0.iter().rev() {
            if !h.is_null() {
                unsafe {
                    let _ = WinHttpCloseHandle(*h);
                }
            }
        }
    }
}

fn env_proxy() -> Option<String> {
    let p = ["HTTPS_PROXY", "https_proxy"].iter().find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))?;
    let p = p.trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');
    Some(p.rsplit('@').next().unwrap_or(p).to_string())
}

/// GET `url` over HTTPS (redirects followed). With `out`, the body is written there and an empty Vec returned.
fn get(url: &str, mut out: Option<&mut std::fs::File>, progress: &dyn Fn(u64, u64) -> bool) -> Result<Vec<u8>, String> {
    const UNREACHABLE: &str = "cannot reach the update server (check the internet, VPN or proxy)";
    unsafe {
        let wurl = wide(url);
        let mut host = [0u16; 256];
        let mut path = [0u16; 2048];
        let mut uc = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            lpszHostName: windows::core::PWSTR(host.as_mut_ptr()),
            dwHostNameLength: host.len() as u32,
            lpszUrlPath: windows::core::PWSTR(path.as_mut_ptr()),
            dwUrlPathLength: path.len() as u32,
            ..Default::default()
        };
        WinHttpCrackUrl(&wurl[..wurl.len() - 1], 0, &mut uc).map_err(|_| "bad update address".to_string())?;
        let agent = wide(concat!("WinToolsUpdate/", env!("CARGO_PKG_VERSION")));
        let proxy = env_proxy().map(|p| wide(&p));
        let s = match &proxy {
            Some(p) => WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_NAMED_PROXY, PCWSTR(p.as_ptr()), PCWSTR::null(), 0),
            None => WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0),
        };
        let mut h = Handles([s, std::ptr::null_mut(), std::ptr::null_mut()]);
        if s.is_null() {
            return Err(UNREACHABLE.into());
        }
        let _ = WinHttpSetTimeouts(s, 8000, 10000, 15000, 30000);
        h.0[1] = WinHttpConnect(s, PCWSTR(host.as_ptr()), uc.nPort, 0);
        if h.0[1].is_null() {
            return Err(UNREACHABLE.into());
        }
        let verb = wide("GET");
        h.0[2] = WinHttpOpenRequest(h.0[1], PCWSTR(verb.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE);
        let r = h.0[2];
        if r.is_null() || WinHttpSendRequest(r, None, None, 0, 0, 0).is_err() || WinHttpReceiveResponse(r, std::ptr::null_mut()).is_err() {
            return Err(UNREACHABLE.into());
        }
        let mut status = 0u32;
        let mut sz = 4u32;
        let _ = WinHttpQueryHeaders(r, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut _), &mut sz, std::ptr::null_mut());
        if status != 200 {
            return Err(format!("the update server answered {status}"));
        }
        let mut total = 0u32;
        let mut sz = 4u32;
        let _ = WinHttpQueryHeaders(r, WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut total as *mut u32 as *mut _), &mut sz, std::ptr::null_mut());
        let mut body = vec![];
        let mut done = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut got = 0u32;
            if WinHttpReadData(r, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got).is_err() {
                return Err(UNREACHABLE.into());
            }
            if got == 0 {
                break;
            }
            let chunk = &buf[..got as usize];
            match out.as_mut() {
                Some(f) => f.write_all(chunk).map_err(|e| format!("cannot save the update: {e}"))?,
                None if body.len() > 1 << 20 => return Err("the update list is too large".into()),
                None => body.extend_from_slice(chunk),
            }
            done += got as u64;
            if !progress(done, total as u64) {
                return Err("update cancelled".into());
            }
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(newer("0.2.10", "0.2.9"));
        assert!(newer("0.3", "0.2.99"));
        assert!(!newer("0.2.9", "0.2.9"));
        assert!(!newer("0.2.8", "0.2.9"));
        assert!(newer("v1.0.0", "0.9.9"));
    }

    #[test]
    fn offer_only_for_newer_https_entries() {
        let m = r#"{"fm":{"version":"0.2.10","url":"https://x.dev/dl/a-setup.exe","size":5,"notes":["One","Two"]},
                   "tm":{"version":"0.2.9","url":"https://x.dev/dl/b.exe"},"xx":{"version":"9.0","url":"http://x/b.exe"}}"#;
        let o = offer_from(m, "fm", "0.2.9").unwrap().unwrap();
        assert_eq!((o.version.as_str(), o.size, o.notes.len()), ("0.2.10", 5, 2));
        assert_eq!(offer_from(m, "tm", "0.2.9").unwrap(), None);
        assert_eq!(offer_from(m, "xx", "0.2.9").unwrap(), None);
        assert_eq!(offer_from(m, "none", "0.2.9").unwrap(), None);
        assert!(offer_from("not json", "fm", "0.2.9").is_err());
    }
}
