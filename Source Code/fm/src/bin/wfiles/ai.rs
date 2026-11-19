//! (Copied from the Task Manager's app/src/ai.rs; the file questions are at the end.)
//! AI explanations: online only, Google Gemini with the USER's own API key (Sajjad, 2026-11-19).
//! - OFF by default. The Settings switch AND a saved key are both needed; with no key no AI button shows.
//! - The key is stored encrypted for the current Windows user (DPAPI) in %LOCALAPPDATA%\WinTools\gemini.key,
//!   never in ai.json or logs, never printed.
//! - Only what Settings lists is sent: for a process its name, path, file description, publisher, window title and
//!   command line (first 400 chars) plus the local checks; nothing else.
//! - Honours HTTPS_PROXY / https_proxy, else the system (WinHTTP / Internet Options) proxy, so it works behind a VPN.
//! - Model fixed: gemini-3.5-flash-lite (free tier, ~1.7 s per answer; Sajjad 2026-11-19: fast + free plan), thinking
//!   "minimal", through generateContent (model id and thinking levels checked in the Gemini API docs 2026-11-19).
//! Settings file %LOCALAPPDATA%\WinTools\ai.json: { "enabled": false, "lang": "en" | "fa" }.
use fmcore::util::wide;
use serde_json::{Value, json};
use windows::Win32::Networking::WinHttp::*;
use windows::core::PCWSTR;

pub const MODEL: &str = "gemini-3.5-flash-lite";
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash-lite:generateContent";
pub const GET_KEY_URL: &str = "https://aistudio.google.com/apikey";
/// shown next to the switch in Settings
pub const SENT_NOTE: &str = "When AI is on, \"What is this file?\" sends the file name, path, type, size and the first lines of a text file, and \"Suggest names\" sends only the file names, to Google Gemini with your key. It is asked only for file types the app does not know. Nothing is sent while AI is off.";

pub fn config_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("WinTools")
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn settings() -> Value {
    std::fs::read_to_string(config_dir().join("ai.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

pub fn lang() -> String {
    env("WT_AI_LANG").unwrap_or_else(|| settings()["lang"].as_str().unwrap_or("en").to_string())
}

pub fn set_lang(l: &str) -> Result<(), String> {
    let mut v = settings();
    v["lang"] = json!(if l == "fa" { "fa" } else { "en" });
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    std::fs::write(config_dir().join("ai.json"), serde_json::to_string_pretty(&v).unwrap()).map_err(|e| e.to_string())
}

/// The Settings switch only (no key check).
pub fn switch_on() -> bool {
    crate::models::switch_on(crate::models::Section::Ai)
}

/// AI usable: switch on and a key saved. Gate every AI button, spinner and call on this.
pub fn enabled() -> bool {
    switch_on() && has_key()
}

pub fn available() -> bool {
    enabled()
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    if on && !has_key() {
        return Err("add your Gemini API key first (Settings > AI features > API key, or: wtm ai key set)".into());
    }
    crate::models::set_switch(crate::models::Section::Ai, on)
}

// ------------------------------------------------------------------------------------------------ key (DPAPI)
fn key_path() -> std::path::PathBuf {
    config_dir().join("gemini.key")
}

pub fn has_key() -> bool {
    load_key().is_some()
}

pub fn set_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.len() < 20 || key.contains(char::is_whitespace) {
        return Err("that does not look like a Gemini API key".into());
    }
    let blob = dpapi(key.as_bytes(), true)?;
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    std::fs::write(key_path(), blob).map_err(|e| e.to_string())
}

pub fn clear_key() -> Result<(), String> {
    let _ = std::fs::remove_file(key_path());
    let _ = crate::models::set_switch(crate::models::Section::Ai, false);
    Ok(())
}

fn load_key() -> Option<String> {
    let blob = std::fs::read(key_path()).ok()?;
    let plain = dpapi(&blob, false).ok()?;
    String::from_utf8(plain).ok().filter(|k| !k.is_empty())
}

/// CryptProtectData / CryptUnprotectData for the current user.
fn dpapi(data: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::*;
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let r = if protect {
            CryptProtectData(&input, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        r.map_err(|e| e.message())?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Ok(v)
    }
}

// ------------------------------------------------------------------------------------------------ HTTP (WinHTTP)
/// HTTPS_PROXY / https_proxy as "host:port" (scheme and credentials stripped), if set.
fn env_proxy() -> Option<String> {
    let p = env("HTTPS_PROXY").or_else(|| env("https_proxy"))?;
    let p = p.trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');
    Some(p.rsplit('@').next().unwrap_or(p).to_string())
}

fn http_post(url: &str, headers: &str, body: &str, timeout_ms: i32) -> Result<(u32, String), String> {
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
        WinHttpCrackUrl(&wurl[..wurl.len() - 1], 0, &mut uc).map_err(|e| e.message())?;
        let agent = wide("WinTools/0.1");
        let proxy = env_proxy().map(|p| wide(&p));
        let s = match &proxy {
            Some(p) => WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_NAMED_PROXY, PCWSTR(p.as_ptr()), PCWSTR::null(), 0),
            // system proxy: WinHTTP's own setting, else Internet Options (Windows 8.1+)
            None => WinHttpOpen(PCWSTR(agent.as_ptr()), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0),
        };
        if s.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        let _ = WinHttpSetTimeouts(s, 8000, 10000, timeout_ms, timeout_ms);
        let c = WinHttpConnect(s, PCWSTR(host.as_ptr()), uc.nPort, 0);
        let verb = wide("POST");
        let r = WinHttpOpenRequest(c, PCWSTR(verb.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE);
        let close = || {
            let _ = WinHttpCloseHandle(r);
            let _ = WinHttpCloseHandle(c);
            let _ = WinHttpCloseHandle(s);
        };
        let wh: Vec<u16> = headers.encode_utf16().collect();
        let b = body.as_bytes();
        if WinHttpSendRequest(r, Some(&wh), Some(b.as_ptr() as *const _), b.len() as u32, b.len() as u32, 0).is_err() || WinHttpReceiveResponse(r, std::ptr::null_mut()).is_err() {
            close();
            return Err("cannot reach Google Gemini (check the internet, VPN or proxy)".into());
        }
        let mut status = 0u32;
        let mut sz = 4u32;
        let _ = WinHttpQueryHeaders(r, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut _), &mut sz, std::ptr::null_mut());
        let mut out = vec![];
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(r, &mut avail).is_err() || avail == 0 {
                break;
            }
            let mut chunk = vec![0u8; avail as usize];
            let mut got = 0u32;
            if WinHttpReadData(r, chunk.as_mut_ptr() as *mut _, avail, &mut got).is_err() {
                break;
            }
            out.extend_from_slice(&chunk[..got as usize]);
        }
        close();
        Ok((status, String::from_utf8_lossy(&out).into_owned()))
    }
}

fn gemini(key: &str, system: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let body = json!({
        "systemInstruction": {"parts": [{"text": system}]},
        "contents": [{"role": "user", "parts": [{"text": prompt}]}],
        "generationConfig": {"responseMimeType": "application/json", "temperature": 0.2, "maxOutputTokens": max_tokens,
            "thinkingConfig": {"thinkingLevel": "minimal"}}
    })
    .to_string();
    let headers = format!("Content-Type: application/json\r\nx-goog-api-key: {key}\r\n");
    let (code, text) = http_post(ENDPOINT, &headers, &body, 40000)?;
    if code != 200 {
        let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string())).unwrap_or_default();
        return Err(match code {
            400 if msg.to_lowercase().contains("api key") => "the API key is not valid".into(),
            403 => format!("Gemini refused the request (HTTP 403{})", if msg.is_empty() { String::new() } else { format!(": {msg}") }),
            429 => "Free quota reached, try later.".into(),
            _ => format!("Gemini error HTTP {code}{}", if msg.is_empty() { String::new() } else { format!(": {msg}") }),
        });
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let parts = v["candidates"][0]["content"]["parts"].as_array().cloned().unwrap_or_default();
    let out: String = parts.iter().filter(|p| !p["thought"].as_bool().unwrap_or(false)).filter_map(|p| p["text"].as_str()).collect();
    if out.is_empty() { Err("empty answer".into()) } else { Ok(out) }
}

/// "Test key" button: one tiny request. Ok(()) = the key works from this PC (and its proxy).
pub fn test_key(key: Option<&str>) -> Result<(), String> {
    let k = match key {
        Some(k) => k.trim().to_string(),
        None => load_key().ok_or("no API key saved")?,
    };
    gemini(&k, "Reply only with JSON {\"ok\": true}.", "ping", 20).map(|_| ())
}

/// One completion (JSON asked for in `system`). Fails when AI is off or there is no key.
pub fn complete(system: &str, prompt: &str) -> Result<String, String> {
    if !enabled() {
        return Err("AI features are off (Settings > AI features)".into());
    }
    let key = load_key().ok_or("no API key saved")?;
    gemini(&key, system, prompt, 400)
}

// ------------------------------------------------------------------------------------------------ file cache
fn cache_key(path: &std::path::Path, lang: &str) -> String {
    let (size, mtime) = std::fs::metadata(path).map(|m| (m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0))).unwrap_or((0, 0));
    format!("{}|{size}|{mtime}|{lang}", path.to_string_lossy().to_lowercase())
}

fn cache_file() -> std::path::PathBuf {
    config_dir().join("ai-cache-files.json")
}

fn cache_get(key: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(cache_file()).ok()?).ok()?;
    v.get(key).cloned()
}

fn cache_put(key: &str, val: Value) {
    let mut v: Value = std::fs::read_to_string(cache_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
    v[key] = val;
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(cache_file(), v.to_string());
}

fn clean_json(text: &str) -> Value {
    let clean = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    serde_json::from_str(clean).unwrap_or_else(|_| json!({"what": clean}))
}

/// What "What is this file?" sends (as listed in Settings): file name, path, type, size and the first lines of a
/// text file. Nothing else.
fn facts(path: &std::path::Path) -> Value {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let md = std::fs::metadata(path).ok();
    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let mut f = json!({"name": name, "path": path.display().to_string(), "type": fmcore::info::type_name(path, is_dir),
        "size_bytes": md.as_ref().map(|m| m.len()).unwrap_or(0)});
    if is_dir {
        return f;
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let texty = matches!(ext.as_str(), "txt" | "md" | "json" | "jsonl" | "py" | "rs" | "js" | "ts" | "toml" | "yml" | "yaml" | "csv" | "log" | "ini" | "cfg" | "xml" | "html" | "ps1" | "bat" | "cmd" | "sh" | "ipynb" | "srt" | "conf");
    if texty && let Ok(mut fh) = std::fs::File::open(path) {
        use std::io::Read;
        let mut buf = vec![0u8; 1500];
        let n = fh.read(&mut buf).unwrap_or(0);
        let head: String = String::from_utf8_lossy(&buf[..n]).lines().take(20).collect::<Vec<_>>().join("
");
        f["first_lines"] = json!(head);
    }
    f
}

/// "What is this file?": a short answer in the chosen language, plus up to two fitting actions.
#[derive(Clone, Debug, Default)]
pub struct FileAnswer {
    pub what: String,
    pub actions: Vec<String>,
    pub lang: String,
}

pub fn explain_file(path: &std::path::Path) -> Result<FileAnswer, String> {
    let lang = lang();
    let key = cache_key(path, &lang);
    let v = match cache_get(&key) {
        Some(v) => v,
        None => {
            let language = if lang == "fa" { "short standard written Persian (no first person)" } else { "short plain English" };
            let system = format!(
                "You explain one file or folder on a Windows PC to a non-expert. Reply ONLY with JSON: {{\"what\": \"two or three short sentences in {language}: what it is, whether it is still needed, what to do with it\", \"actions\": up to two of [\"open\", \"extract\", \"delete\", \"run\"]}}. Never invent facts that are not in the data."
            );
            let text = complete(&system, &format!("File facts: {}", facts(path)))?;
            let a = clean_json(&text);
            cache_put(&key, a.clone());
            a
        }
    };
    Ok(FileAnswer {
        what: v["what"].as_str().unwrap_or("").to_string(),
        actions: v["actions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default(),
        lang,
    })
}

/// "Suggest names" for a batch: one short new stem per file (extension kept by the caller).
pub fn suggest_names(paths: &[std::path::PathBuf], style: &str) -> Result<Vec<String>, String> {
    // only the names are sent
    let files: Vec<Value> = paths.iter().take(40).map(|p| json!(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())).collect();
    let system = format!(
        "You rename files. Reply ONLY with JSON: {{\"names\": [one new name per file, same order, WITHOUT the extension]}}. Style: {style}. Names are short (2 to 5 words), describe the file better than the old name, are unique, and use only letters, digits and - or _."
    );
    let text = complete(&system, &format!("Files: {}", Value::Array(files)))?;
    let v = clean_json(&text);
    let names: Vec<String> = v["names"].as_array().map(|a| a.iter().map(|x| x.as_str().unwrap_or("").trim().to_string()).collect()).unwrap_or_default();
    if names.len() != paths.len().min(40) {
        return Err("the answer did not have one name per file".into());
    }
    Ok(names)
}
