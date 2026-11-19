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
pub const SENT_NOTE: &str = "Search in your own words, in any language: type what you need and press Shift+Enter (or AI Search). It sends your search words and the names, folders and dates of up to 60 files it finds to Google Gemini with your own key. Nothing is sent while it is off.";

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
    saved_key().or_else(env_key)
}

fn saved_key() -> Option<String> {
    let blob = std::fs::read(key_path()).ok()?;
    let plain = dpapi(&blob, false).ok()?;
    String::from_utf8(plain).ok().filter(|k| !k.is_empty())
}

// ------------------------------------------------------------------------------------------------ relay
/// A relay that keeps Google's paths, for places where Google is blocked: Settings (ai.json "relay_url",
/// "relay_header"; the secret encrypted like the key in gemini-relay.key), else GEMINI_BASE_URL /
/// GEMINI_RELAY_HEADER / GEMINI_RELAY_SECRET from the environment.
pub struct Relay {
    pub url: String,
    pub header: String,
    pub secret: Option<String>,
}

fn relay_secret_path() -> std::path::PathBuf {
    config_dir().join("gemini-relay.key")
}

/// What Settings shows: (url, header, a secret is saved)
pub fn relay_saved() -> (String, String, bool) {
    let s = settings();
    (s["relay_url"].as_str().unwrap_or("").to_string(), s["relay_header"].as_str().unwrap_or("").to_string(), relay_secret_path().is_file())
}

/// Save the relay (empty url = no relay). `secret`: None keeps the saved one, Some("") removes it.
pub fn set_relay(url: &str, header: &str, secret: Option<&str>) -> Result<(), String> {
    let url = url.trim().trim_end_matches('/');
    if !url.is_empty() && !url.starts_with("https://") {
        return Err("the relay address must start with https://".into());
    }
    let mut s = settings();
    s["relay_url"] = json!(url);
    s["relay_header"] = json!(header.trim());
    std::fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    std::fs::write(config_dir().join("ai.json"), serde_json::to_string_pretty(&s).unwrap()).map_err(|e| e.to_string())?;
    match secret.map(str::trim) {
        Some("") => {
            let _ = std::fs::remove_file(relay_secret_path());
        }
        Some(sec) => std::fs::write(relay_secret_path(), dpapi(sec.as_bytes(), true)?).map_err(|e| e.to_string())?,
        None => {}
    }
    Ok(())
}

fn relay() -> Option<Relay> {
    let (url, header, _) = relay_saved();
    if !url.is_empty() {
        let secret = std::fs::read(relay_secret_path()).ok().and_then(|b| dpapi(&b, false).ok()).and_then(|p| String::from_utf8(p).ok()).filter(|s| !s.is_empty());
        return Some(Relay { url, header: if header.is_empty() { "x-relay-secret".into() } else { header }, secret });
    }
    env("GEMINI_BASE_URL").map(|url| Relay { url, header: env("GEMINI_RELAY_HEADER").unwrap_or_else(|| "x-relay-secret".into()), secret: env("GEMINI_RELAY_SECRET") })
}

/// A key from the environment (GEMINI_API_KEY, else GEMINI_API_KEY_1), used when none is saved in Settings.
fn env_key() -> Option<String> {
    env("GEMINI_API_KEY").or_else(|| env("GEMINI_API_KEY_1"))
}

/// The key comes from the environment, not from Settings (for the Settings status line).
pub fn key_from_env() -> bool {
    saved_key().is_none() && env_key().is_some()
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
    let mut headers = format!("Content-Type: application/json\r\nx-goog-api-key: {key}\r\n");
    // where Google is blocked (Iran): the relay from Settings, else from the environment (never in the code)
    let endpoint = match relay() {
        Some(r) => {
            if let Some(secret) = &r.secret {
                headers += &format!("{}: {secret}\r\n", r.header);
            }
            format!("{}/v1beta/models/{MODEL}:generateContent", r.url.trim_end_matches('/'))
        }
        None => ENDPOINT.to_string(),
    };
    let (mut code, mut text) = http_post(&endpoint, &headers, &body, 40000)?;
    // "high demand" (503) is usually gone a moment later: one more try
    if code == 503 {
        std::thread::sleep(std::time::Duration::from_millis(700));
        (code, text) = http_post(&endpoint, &headers, &body, 40000)?;
    }
    if code != 200 {
        let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string())).unwrap_or_default();
        return Err(match code {
            400 if msg.to_lowercase().contains("api key") => "the API key is not valid".into(),
            // an HTML 403 with no message = Google blocks this region: a relay helps
            403 if msg.is_empty() => "Google blocks Gemini from this region: add a relay in Settings > Search and AI".into(),
            403 => format!("Gemini refused the request (HTTP 403: {msg})"),
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

/// "AI Search" search: a request in plain words (any language) becomes up to three Everything queries that the app runs
/// together. Only the request text, today's date and the user's folder paths are sent.
pub fn search_queries(request: &str) -> Result<(Vec<String>, String), String> {
    search_queries_with(request, None)
}

/// AI Search, second step: the request and the candidates found (name, folder, kind, date) go to Gemini, which
/// returns the numbers of the ones that really match, best first, and a short note. This is what makes the search
/// understand meaning instead of only words.
pub fn pick_results(request: &str, cands: &[(String, String, bool, String)], key: Option<&str>) -> Result<(Vec<usize>, Vec<usize>, String), String> {
    let list: Vec<String> = cands.iter().enumerate().map(|(i, (name, folder, dir, date))| format!("{i}. {}{name}  |  in {folder}  |  {date}", if *dir { "[folder] " } else { "" })).collect();
    let system = r#"You pick files for a request. You get the request and numbered candidates (name | folder | date).
Reply ONLY with JSON: {"best": [numbers of the best matches, best first, up to 15], "drop": [numbers that clearly do NOT fit], "note": "under 8 words, in the request's language"}.
Judge by meaning, not by shared words: the folder tells what a file belongs to, the name tells what it is.
If the exact thing is missing, the closest real things are the best (the program's exe and its folder for a
missing installer, the project folder for its code). Drop only what clearly does not fit; when unsure, keep."#;
    let prompt = format!("Request: {request}\nCandidates:\n{}", list.join("\n"));
    let text = match key {
        Some(k) => gemini(k, system, &prompt, 600)?,
        None => complete(system, &prompt)?,
    };
    let v = clean_json(&text);
    let nums = |k: &str| -> Vec<usize> { v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).filter(|i| *i < cands.len()).collect()).unwrap_or_default() };
    Ok((nums("best"), nums("drop"), v["note"].as_str().unwrap_or("").to_string()))
}

/// `key`: a key for a test run (the saved key and the AI switch are used when None).
pub fn search_queries_with(request: &str, key: Option<&str>) -> Result<(Vec<String>, String), String> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let today = {
        let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay)
    };
    let system = format!(
        r#"You turn a request for files or folders into Everything (voidtools) search queries.
Reply ONLY with JSON: {{"queries": [3 to 5 strings], "core": "core name words", "note": "under 8 words, in the request's language"}}.
Your queries gather CANDIDATES; a second step picks the right ones by meaning, so cast a wide net: synonyms,
both English and Persian words, abbreviations, related names and folder names, each in its own query.
Everything syntax: space = AND, and EVERY word must be part of the file or folder NAME; a|b = OR; !word = NOT;
ext:pdf;docx (types); folder: or file:; size:>10mb; dm:today, dm:thisweek, dm:thismonth, dm:lastmonth, dm:thisyear
(modified); "C:\some\folder\" (quoted, ending in a backslash) = only inside that folder. Never write folder:PATH.
Rules for good queries:
- Use 1 or 2 short distinctive name words per query (a title word, a brand, a project name). Never add generic
  words that names rarely hold (my, the, of, source, code, installer, setup, video, movie, document), unless
  they are part of a likely name ("file manager" is a project name, "file" alone is not).
- Programs and games: their folder has the full title, their .exe often an abbreviation (Gears of War -> gow,
  GoWEDay; Visual Studio Code -> code). Put title|abbreviation in EVERY query (gears|gow).
- Projects and source code: search FOLDER names with the project words (e.g. file manager folder:), since code
  files inside are named by their content, not by the project.
- Express the kind of file with ext: (installers: ext:exe;msi;iso;zip;rar;7z, videos: ext:mp4;mkv;avi;mov;webm,
  source code: ext:rs;py;js;ts;cs;cpp, documents: ext:pdf;docx;doc;txt;md, pictures: ext:jpg;jpeg;png;webp;heic).
- Use | for spelling variants, abbreviations and the other language (gears|gow, film|movie, English|Persian word).
- Query 1: the most precise (topic words + the asked kind). Query 2: the topic with RELATED kinds too (for a
  game or program also ext:exe and folder:, for a film also its folder). Query 3: the BROAD fallback = only the
  core name words, no ext:, no date (e.g. gears war). The user wants the nearest real thing when the exact one
  does not exist (no installer on disk, but the game's exe or folder is there).
- Also give "core": the core name words alone (e.g. "gears war"), used if every query finds nothing.
- With no topic word (e.g. "videos downloaded this month"), use the kind + place + time, and core = "".
Examples:
"the installer of Gears of War" -> {{"queries": ["gears|gow ext:exe;msi;iso;zip;rar;7z", "gears|gow folder:|ext:exe", "gears|gow war"], "core": "gears war"}}
"my rust file manager source code" -> {{"queries": ["\"file manager\" folder:", "\"file manager\" ext:rs;toml", "\"file manager\""], "core": "file manager"}}
Today is {today}. User folders: Desktop {home}\Desktop, Downloads {home}\Downloads, Documents {home}\Documents,
Pictures {home}\Pictures, Videos {home}\Videos, Music {home}\Music."#
    );
    let text = match key {
        Some(k) => gemini(k, &system, request, 400)?,
        None => complete(&system, request)?,
    };
    let v = clean_json(&text);
    let qs: Vec<String> = v["queries"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).take(5).collect()).unwrap_or_default();
    if qs.is_empty() {
        return Err("the AI gave no search".into());
    }
    // the core name words go last: the caller runs them alone when every query finds nothing
    let mut qs = qs;
    let core: String = v["core"].as_str().unwrap_or("").split_whitespace().filter(|w| !w.contains(':')).collect::<Vec<_>>().join(" ");
    qs.push(core);
    Ok((qs, v["note"].as_str().unwrap_or("").to_string()))
}
