//! Optional downloads for the AI features. Everything AI is OFF by default (Sajjad, 2026-11-19): the user turns a
//! switch on in Settings and presses "Download model". Files live outside the exe in `models_dir`
//! (default %LOCALAPPDATA%\WinTools\models). Shared config: %LOCALAPPDATA%\WinTools\ai.json
//! {
//!   "enabled": false,                         AI features switch ("What is this?", name suggestions)
//!   "models_dir": "",                         empty = %LOCALAPPDATA%\WinTools\models
//!   "semsearch": { "enabled": false, "cmd": "" }   meaning search, its own switch (cmd = an existing install)
//! }
//! What is downloaded is built into the exe (AI_ITEMS / SEMSEARCH_ITEMS): the official Hugging Face URL only, no mirror,
//! not editable. Downloads resume from `<file>.part` with an HTTP Range header and are checked by size and SHA-256.
use crate::sys::wide;
use serde_json::{Value, json};
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Networking::WinHttp::*;
use windows::core::PCWSTR;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    /// "What is this?", name suggestions (local LLM)
    Ai,
    /// meaning search (semsearch: embedding model + index)
    Semsearch,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub file: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// no download URL configured yet
    NotAvailable,
    NotDownloaded,
    /// part file present (bytes so far)
    Partial,
    Ready,
}

#[derive(Clone, Debug)]
pub struct ItemStatus {
    pub item: Item,
    pub state: State,
    pub have: u64,
    pub path: PathBuf,
}

pub fn config_path() -> PathBuf {
    crate::ai::config_dir().join("ai.json")
}

pub fn load() -> Value {
    std::fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

pub fn save(v: &Value) -> Result<(), String> {
    std::fs::create_dir_all(crate::ai::config_dir()).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), serde_json::to_string_pretty(v).unwrap()).map_err(|e| e.to_string())
}

pub fn models_dir() -> PathBuf {
    let v = load();
    match v["models_dir"].as_str().filter(|s| !s.is_empty()) {
        Some(d) => PathBuf::from(d.replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())),
        None => crate::ai::config_dir().join("models"),
    }
}


/// The downloads are BUILT IN: only the model's official Hugging Face URL, no mirror, not editable in Settings or
/// ai.json, and every file is checked by SHA-256 (Sajjad, 2026-11-19). Model Trainer names the model; until then the
/// entries stay empty and Settings shows "not available yet".
/// (id, name, official URL, size in bytes, sha256, file name)
const AI_ITEMS: &[(&str, &str, &str, u64, &str, &str)] = &[
    // ("runtime", "llama.cpp server", "https://huggingface.co/<org>/<repo>/resolve/<commit>/<file>.zip", <size>, "<sha256>", "llama-server.zip"),
    // ("model", "<model name>", "https://huggingface.co/<org>/<repo>/resolve/<commit>/<file>.gguf", <size>, "<sha256>", "<file>.gguf"),
];
const SEMSEARCH_ITEMS: &[(&str, &str, &str, u64, &str, &str)] = &[
    // ("model", "<embedding model>", "https://huggingface.co/...", <size>, "<sha256>", "<file>"),
];

pub fn items(s: Section) -> Vec<Item> {
    let list = match s {
        Section::Ai => AI_ITEMS,
        Section::Semsearch => SEMSEARCH_ITEMS,
    };
    list.iter()
        .filter(|i| i.2.starts_with("https://huggingface.co/") && !i.4.is_empty())
        .map(|i| Item { id: i.0.into(), name: i.1.into(), url: i.2.into(), size: i.3, sha256: i.4.to_lowercase(), file: i.5.into() })
        .collect()
}

pub fn item_path(i: &Item) -> PathBuf {
    models_dir().join(&i.file)
}

pub fn status(s: Section) -> Vec<ItemStatus> {
    items(s)
        .into_iter()
        .map(|i| {
            let path = item_path(&i);
            let part = path.with_extension(format!("{}.part", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
            let full = std::fs::metadata(&path).map(|m| m.len()).ok();
            let partial = std::fs::metadata(&part).map(|m| m.len()).ok();
            let (state, have) = match (full, partial) {
                (Some(n), _) if i.size == 0 || n == i.size => (State::Ready, n),
                (_, Some(n)) => (State::Partial, n),
                _ if i.url.is_empty() => (State::NotAvailable, 0),
                _ => (State::NotDownloaded, 0),
            };
            ItemStatus { item: i, state, have, path }
        })
        .collect()
}

/// All items downloaded (and at least one configured).
pub fn ready(s: Section) -> bool {
    let st = status(s);
    !st.is_empty() && st.iter().all(|x| x.state == State::Ready)
}

pub fn set_switch(s: Section, on: bool) -> Result<(), String> {
    let mut v = load();
    match s {
        Section::Ai => v["enabled"] = json!(on),
        Section::Semsearch => {
            if !v["semsearch"].is_object() {
                v["semsearch"] = json!({});
            }
            v["semsearch"]["enabled"] = json!(on);
        }
    }
    save(&v)
}

pub fn switch_on(s: Section) -> bool {
    let v = load();
    match s {
        Section::Ai => v["enabled"].as_bool().unwrap_or(false),
        Section::Semsearch => v["semsearch"]["enabled"].as_bool().unwrap_or(false),
    }
}

// ------------------------------------------------------------------------------------------------ semsearch
/// An existing semsearch install (Sajjad's own PC has one): config "cmd", else the known default location.
pub fn semsearch_detect() -> Option<PathBuf> {
    let v = load();
    let mut cands = vec![];
    if let Some(c) = v["semsearch"]["cmd"].as_str().filter(|s| !s.is_empty()) {
        cands.push(PathBuf::from(c));
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        cands.push(PathBuf::from(home).join(r"tools\semindex\semsearch.cmd"));
    }
    cands.into_iter().find(|p| p.is_file() && p.parent().map(|d| d.join("index").is_dir()).unwrap_or(false))
}

/// Meaning search usable: its switch is on and either an install exists or its items are downloaded.
pub fn semsearch_enabled() -> bool {
    switch_on(Section::Semsearch) && (semsearch_detect().is_some() || ready(Section::Semsearch))
}

// ------------------------------------------------------------------------------------------------ download
/// Download every missing item of a section. Resumes `.part` files; `progress(id, have, total)`; stops when
/// `cancel` is set (the part file stays for a later resume).
pub fn download(s: Section, progress: &dyn Fn(&str, u64, u64), cancel: &AtomicBool) -> Result<(), String> {
    std::fs::create_dir_all(models_dir()).map_err(|e| e.to_string())?;
    for st in status(s) {
        if st.state == State::Ready {
            continue;
        }
        let i = &st.item;
        if i.url.is_empty() {
            return Err(format!("no download URL for '{}' yet", i.name));
        }
        let part = st.path.with_extension(format!("{}.part", st.path.extension().and_then(|e| e.to_str()).unwrap_or("")));
        get_resumable(&i.url, &part, i.size, &|have, total| progress(&i.id, have, total), cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err("paused".into());
        }
        let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if i.size > 0 && got != i.size {
            return Err(format!("'{}': got {got} bytes, expected {}", i.name, i.size));
        }
        if !i.sha256.is_empty() {
            let h = sha256_file(&part)?;
            if h != i.sha256 {
                let _ = std::fs::remove_file(&part);
                return Err(format!("'{}': checksum mismatch (file removed, download again)", i.name));
            }
        }
        std::fs::rename(&part, &st.path).map_err(|e| e.to_string())?;
        // a zipped runtime is unpacked next to it (Windows' own tar, hidden)
        if st.path.extension().and_then(|e| e.to_str()) == Some("zip") {
            let dir = models_dir().join(&i.id);
            let _ = std::fs::create_dir_all(&dir);
            let ok = std::process::Command::new("tar")
                .args(["-xf", &st.path.display().to_string(), "-C", &dir.display().to_string()])
                .creation_flags_hidden()
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err(format!("could not unpack {}", st.path.display()));
            }
        }
    }
    Ok(())
}

trait Hidden {
    fn creation_flags_hidden(&mut self) -> &mut Self;
}

impl Hidden for std::process::Command {
    fn creation_flags_hidden(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    }
}

pub fn delete(s: Section) -> Result<u64, String> {
    let mut freed = 0;
    for st in status(s) {
        for p in [st.path.clone(), st.path.with_extension(format!("{}.part", st.path.extension().and_then(|e| e.to_str()).unwrap_or("")))] {
            if let Ok(m) = std::fs::metadata(&p) {
                freed += m.len();
                std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            }
        }
        let unpacked = models_dir().join(&st.item.id);
        if unpacked.is_dir() {
            let _ = std::fs::remove_dir_all(&unpacked);
        }
    }
    if s == Section::Ai {
        let _ = set_switch(s, false);
    }
    Ok(freed)
}

/// Locate a file inside the models dir by name (e.g. llama-server.exe inside the unpacked runtime).
pub fn find_file(name: &str) -> Option<PathBuf> {
    let mut stack = vec![models_dir()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false) {
                return Some(p);
            }
        }
    }
    None
}

fn get_resumable(url: &str, part: &PathBuf, size: u64, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
    let mut have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if size > 0 && have >= size {
        return Ok(());
    }
    unsafe {
        let wurl = wide(url);
        let mut host = [0u16; 256];
        let mut path = [0u16; 4096];
        let mut uc = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            lpszHostName: windows::core::PWSTR(host.as_mut_ptr()),
            dwHostNameLength: host.len() as u32,
            lpszUrlPath: windows::core::PWSTR(path.as_mut_ptr()),
            dwUrlPathLength: path.len() as u32,
            ..Default::default()
        };
        WinHttpCrackUrl(&wurl[..wurl.len() - 1], 0, &mut uc).map_err(|e| e.message())?;
        let agent = wide("WinTools/0.1");
        let s = WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if s.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        let _ = WinHttpSetTimeouts(s, 10000, 15000, 30000, 60000);
        let c = WinHttpConnect(s, PCWSTR(host.as_ptr()), uc.nPort, 0);
        let verb = wide("GET");
        let r = WinHttpOpenRequest(c, PCWSTR(verb.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), if uc.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) });
        let close = || {
            let _ = WinHttpCloseHandle(r);
            let _ = WinHttpCloseHandle(c);
            let _ = WinHttpCloseHandle(s);
        };
        let hdr: Vec<u16> = if have > 0 { format!("Range: bytes={have}-\r\n").encode_utf16().collect() } else { vec![] };
        if WinHttpSendRequest(r, if hdr.is_empty() { None } else { Some(&hdr) }, None, 0, 0, 0).is_err() || WinHttpReceiveResponse(r, std::ptr::null_mut()).is_err() {
            close();
            return Err("network error (download server unreachable)".into());
        }
        let mut status = 0u32;
        let mut sz = 4u32;
        let _ = WinHttpQueryHeaders(r, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut _), &mut sz, std::ptr::null_mut());
        let mut f = match status {
            206 => std::fs::OpenOptions::new().append(true).open(part).map_err(|e| e.to_string())?,
            200 => {
                have = 0; // server ignored the range: start over
                std::fs::File::create(part).map_err(|e| e.to_string())?
            }
            416 if size > 0 && have >= size => {
                close();
                return Ok(());
            }
            other => {
                close();
                return Err(format!("download failed: HTTP {other}"));
            }
        };
        let total = if size > 0 { size } else { 0 };
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut got = 0u32;
            if WinHttpReadData(r, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got).is_err() {
                close();
                return Err("connection dropped (press Download again to resume)".into());
            }
            if got == 0 {
                break;
            }
            f.write_all(&buf[..got as usize]).map_err(|e| e.to_string())?;
            have += got as u64;
            progress(have, total);
        }
        let _ = f.flush();
        close();
        Ok(())
    }
}

/// SHA-256 of a file as lower-case hex (Windows CNG).
pub fn sha256_file(p: &PathBuf) -> Result<String, String> {
    use windows::Win32::Security::Cryptography::*;
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let _ = f.seek(std::io::SeekFrom::Start(0));
    unsafe {
        let mut h = BCRYPT_HASH_HANDLE::default();
        if BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut h, None, None, 0).is_err() {
            return Err("BCryptCreateHash failed".into());
        }
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            let _ = BCryptHashData(h, &buf[..n], 0);
        }
        let mut out = [0u8; 32];
        let _ = BCryptFinishHash(h, &mut out, 0);
        let _ = BCryptDestroyHash(h);
        Ok(out.iter().map(|b| format!("{b:02x}")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the stub server: python scratchpad/range_server.py (127.0.0.1:8098, 3 MB file with Range support).
    /// Run: cargo test --release --offline --lib models -- --ignored
    #[test]
    #[ignore]
    fn download_pauses_resumes_and_hashes() {
        let dir = std::env::temp_dir().join("wt-model-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let part = dir.join("model.gguf.part");
        let cancel = AtomicBool::new(false);
        // first run: stop after ~1 MB
        get_resumable("http://127.0.0.1:8098/model.gguf?slow=1", &part, 3 << 20, &|have, _| if have >= 1 << 20 { cancel.store(true, Ordering::Relaxed) }, &cancel).unwrap();
        let first = std::fs::metadata(&part).unwrap().len();
        assert!(first >= 1 << 20 && first < 3 << 20, "paused part has {first} bytes");
        // second run resumes with a Range request and completes
        let cancel2 = AtomicBool::new(false);
        get_resumable("http://127.0.0.1:8098/model.gguf", &part, 3 << 20, &|_, _| {}, &cancel2).unwrap();
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 3 << 20);
        let want: String = {
            let data: Vec<u8> = (0..3 * 1024 * 1024).map(|i: u32| ((i as u64 * 7 + 3) % 251) as u8).collect();
            std::fs::write(dir.join("ref.bin"), &data).unwrap();
            sha256_file(&dir.join("ref.bin")).unwrap()
        };
        assert_eq!(sha256_file(&part).unwrap(), want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn built_in_items_are_official_huggingface_only() {
        for s in [Section::Ai, Section::Semsearch] {
            for i in items(s) {
                assert!(i.url.starts_with("https://huggingface.co/") && i.sha256.len() == 64);
            }
        }
        assert!(!switch_on(Section::Ai) || ready(Section::Ai), "AI switch can only be on with a ready model");
    }
}
