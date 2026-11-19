//! Instant search through Everything's command line client (es.exe). es.exe runs hidden (CREATE_NO_WINDOW) and
//! answers from Everything's in-memory index, usually in well under 100 ms.
use crate::util::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// What to look for. Every filter is optional; `words` follow Everything's syntax (space = AND, `|` = OR, `!` = NOT,
/// wildcards `*` and `?`).
#[derive(Clone, Debug, Default)]
pub struct Query {
    pub words: Vec<String>,
    /// Only below this folder.
    pub in_dir: Option<PathBuf>,
    /// Extensions without dots, e.g. ["pdf", "docx"].
    pub exts: Vec<String>,
    /// Size filter in Everything syntax after "size:", e.g. ">10mb", "<1kb", "1mb..5mb".
    pub size: Option<String>,
    /// Modified within this window: "7d", "12h", "2w", "3mo", "1y" (or Everything syntax like "today").
    pub modified: Option<String>,
    /// Only folders (Some(true)), only files (Some(false)) or both (None).
    pub folders: Option<bool>,
    /// Max results (0 = 100).
    pub top: usize,
    /// Everything sort name: "name", "path", "size", "date-modified", ... with optional "-descending".
    pub sort: Option<String>,
    /// Treat `words` as one regular expression.
    pub regex: bool,
    /// Match words against the full path, not only the name.
    pub match_path: bool,
    /// Which engine answers (default: Everything, else the folder walk).
    pub engine: Engine,
    /// Time limit of the folder walk in ms (0 = 5000).
    pub walk_limit_ms: u64,
}

/// Search engine choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// Everything when it runs, else a walk of the folder tree.
    #[default]
    Auto,
    Everything,
    /// Walk `in_dir` (or the current folder) with a time limit; no index needed.
    Walk,
}

/// Search results and which engine answered.
#[derive(Clone, Debug)]
pub struct Found {
    pub hits: Vec<Hit>,
    /// "everything" or "walk".
    pub engine: &'static str,
    /// false when the walk hit its time limit (more results may exist).
    pub complete: bool,
    /// Why the walk was used, when Everything was wanted.
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    /// None when Everything does not index folder sizes.
    pub size: Option<u64>,
    pub modified: u64,
    pub created: u64,
    pub attrs: u32,
}

/// Where es.exe is: $WFM_ES, "es_path" in fm.json, next to wfm.exe, %USERPROFILE%\tools\es\es.exe, then PATH.
pub fn es_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WFM_ES") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = crate::pins::config().get("es_path").and_then(|v| v.as_str()).map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(me) = std::env::current_exe() {
        let p = me.with_file_name("es.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(home).join(r"tools\es\es.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            let p = d.join("es.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Run es.exe hidden with these arguments; returns stdout. Error 8 from es.exe means Everything is not running.
pub fn run_es(args: &[String]) -> Res<String> {
    // Test hook: behave as if Everything were not running.
    if std::env::var_os("WFM_NO_EVERYTHING").is_some() {
        return Err("Everything is not running (WFM_NO_EVERYTHING is set)".into());
    }
    let es = es_path().ok_or("es.exe (Everything command line) not found; set WFM_ES or put es.exe next to wfm.exe")?;
    let out = hidden_command(&es).args(args).output().map_err(|e| format!("cannot run es.exe: {e}"))?;
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        if code == 8 {
            return Err("Everything is not running (start Everything, then search again)".into());
        }
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if msg.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { msg };
        return Err(format!("es.exe failed (code {code}): {msg}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Is Everything running and answering?
pub fn everything_ready() -> bool {
    run_es(&["-get-everything-version".into()]).is_ok()
}

/// "7d" -> "last7days", "12h" -> "last12hours", "2w" -> "last2weeks", "3mo" -> "last3months", "1y" -> "last1years".
fn modified_term(s: &str) -> String {
    let t = s.trim().to_ascii_lowercase();
    let idx = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (n, u) = t.split_at(idx);
    if n.is_empty() {
        return t; // already Everything syntax ("today", "thisweek", ">2026-01-01", ...)
    }
    let unit = match u {
        "" | "d" | "day" | "days" => "days",
        "h" | "hour" | "hours" => "hours",
        "m" | "min" | "mins" | "minutes" => "minutes",
        "w" | "week" | "weeks" => "weeks",
        "mo" | "month" | "months" => "months",
        "y" | "year" | "years" => "years",
        _ => return t,
    };
    format!("last{n}{unit}")
}

/// Build the es.exe argument list for a query (public so the GUI can show it in a tooltip).
pub fn es_args(q: &Query) -> Vec<String> {
    let mut a: Vec<String> = ["-json", "-size", "-dm", "-dc", "-attrib", "-date-format", "2", "-size-format", "0"].iter().map(|s| s.to_string()).collect();
    a.push("-n".into());
    a.push(if q.top == 0 { "100".into() } else { q.top.to_string() });
    if let Some(d) = &q.in_dir {
        a.push("-path".into());
        a.push(d.to_string_lossy().into_owned());
    }
    if let Some(s) = &q.sort {
        a.push("-sort".into());
        a.push(s.clone());
    }
    if q.match_path {
        a.push("-p".into());
    }
    if q.regex {
        a.push("-r".into());
        a.push(q.words.join(" "));
    } else {
        a.extend(q.words.iter().cloned());
    }
    if !q.exts.is_empty() {
        a.push(format!("ext:{}", q.exts.iter().map(|e| e.trim_start_matches('.')).collect::<Vec<_>>().join(";")));
    }
    if let Some(s) = &q.size {
        a.push(format!("size:{}", s.replace(' ', "")));
    }
    if let Some(m) = &q.modified {
        a.push(format!("dm:{}", modified_term(m)));
    }
    match q.folders {
        Some(true) => a.push("folder:".into()),
        Some(false) => a.push("file:".into()),
        None => {}
    }
    a
}

/// Search through Everything only (the whole PC or one folder).
pub fn search_everything(q: &Query) -> Res<Vec<Hit>> {
    let out = run_es(&es_args(q))?;
    parse_es_json(&out)
}

/// Search: Everything when it runs (instant, whole PC), else a time-limited walk of `in_dir` (or the current
/// folder). `Found::engine` says which one answered.
pub fn search(q: &Query) -> Res<Found> {
    if q.engine != Engine::Walk {
        match search_everything(q) {
            Ok(hits) => return Ok(Found { hits, engine: "everything", complete: true, note: String::new() }),
            Err(e) if q.engine == Engine::Everything => return Err(e),
            Err(e) if e.contains("not running") || e.contains("not found") || e.contains("cannot run") => {
                let mut f = walk_search(q)?;
                f.note = format!("{e}; searched by walking {}", f.note);
                return Ok(f);
            }
            Err(e) => return Err(e),
        }
    }
    walk_search(q)
}

/// Wildcard match (`*`, `?`), case-insensitive, whole string.
fn wild(p: &[char], s: &[char]) -> bool {
    let (mut pi, mut si, mut star, mut mark) = (0, 0, usize::MAX, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            pi += 1;
            mark = si;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// One word of a walk query: substring, or wildcard pattern when it has * or ?; "!" negates; "a|b" = either.
fn word_matches(word: &str, hay: &str) -> bool {
    let (neg, w) = match word.strip_prefix('!') {
        Some(r) => (true, r),
        None => (false, word),
    };
    let any = w.split('|').filter(|a| !a.is_empty()).any(|alt| {
        let a = alt.to_lowercase();
        if a.contains('*') || a.contains('?') {
            let pc: Vec<char> = a.chars().collect();
            let sc: Vec<char> = hay.chars().collect();
            wild(&pc, &sc)
        } else {
            hay.contains(&a)
        }
    });
    any != neg
}

/// Size filter: ">10mb", ">=1gb", "<1kb", "=0", "1mb..5mb".
fn size_ok(f: &str, n: u64) -> bool {
    let f = f.replace(' ', "");
    if let Some((a, b)) = f.split_once("..") {
        return parse_size(a).is_none_or(|a| n >= a) && parse_size(b).is_none_or(|b| n <= b);
    }
    for (op, cmp) in [(">=", 0), ("<=", 1), (">", 2), ("<", 3), ("=", 4)] {
        if let Some(v) = f.strip_prefix(op).and_then(parse_size) {
            return match cmp {
                0 => n >= v,
                1 => n <= v,
                2 => n > v,
                3 => n < v,
                _ => n == v,
            };
        }
    }
    parse_size(&f).is_none_or(|v| n == v)
}

/// "7d" / "12h" / "2w" / "3mo" / "1y" / "30m" -> oldest allowed FILETIME (None when not understood).
fn modified_cutoff(s: &str) -> Option<u64> {
    let t = s.trim().to_ascii_lowercase();
    if t == "today" {
        return Some(now_ft().saturating_sub(24 * 3600 * 10_000_000));
    }
    let idx = t.find(|c: char| !c.is_ascii_digit())?;
    let n: u64 = t[..idx].parse().ok()?;
    let secs = match &t[idx..] {
        "d" | "day" | "days" => 86_400,
        "h" | "hour" | "hours" => 3_600,
        "m" | "min" | "mins" | "minutes" => 60,
        "w" | "week" | "weeks" => 7 * 86_400,
        "mo" | "month" | "months" => 30 * 86_400,
        "y" | "year" | "years" => 365 * 86_400,
        _ => return None,
    };
    Some(now_ft().saturating_sub(n * secs * 10_000_000))
}

/// Search by walking the folder tree (no index). Breadth first, so shallow matches come first.
pub fn walk_search(q: &Query) -> Res<Found> {
    let root = q.in_dir.clone().map(abs).unwrap_or_else(|| abs("."));
    if !root.is_dir() {
        return Err(format!("folder not found: {}", root.display()));
    }
    let re = if q.regex { Some(crate::rx::Regex::new(&q.words.join(" "))?) } else { None };
    let words: Vec<String> = if q.regex { vec![] } else { q.words.iter().filter(|w| !w.is_empty()).cloned().collect() };
    let exts: Vec<String> = q.exts.iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect();
    let cutoff = q.modified.as_deref().and_then(modified_cutoff);
    let top = if q.top == 0 { 100 } else { q.top };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(if q.walk_limit_ms == 0 { 5000 } else { q.walk_limit_ms });
    let mut hits = vec![];
    let mut complete = true;
    let mut queue = std::collections::VecDeque::from([root.clone()]);
    'walk: while let Some(d) = queue.pop_front() {
        if std::time::Instant::now() > deadline {
            complete = false;
            break;
        }
        let Ok(list) = crate::list::list_dir(&d, true) else { continue };
        for e in list {
            let path = d.join(&e.name);
            if e.is_dir() && e.attrs & ATTR_REPARSE == 0 {
                queue.push_back(path.clone());
            }
            if let Some(f) = q.folders {
                if f != e.is_dir() {
                    continue;
                }
            }
            if !exts.is_empty() && (e.is_dir() || !exts.contains(&e.ext())) {
                continue;
            }
            if let Some(s) = &q.size {
                if e.is_dir() || !size_ok(s, e.size) {
                    continue;
                }
            }
            if cutoff.is_some_and(|c| e.modified < c) {
                continue;
            }
            let hay = if q.match_path { path.to_string_lossy().to_lowercase() } else { e.name.to_lowercase() };
            if let Some(r) = &re {
                if !r.is_match(&hay) {
                    continue;
                }
            } else if !words.iter().all(|w| word_matches(w, &hay)) {
                continue;
            }
            hits.push(Hit {
                path: path.to_string_lossy().into_owned(),
                name: e.name.clone(),
                is_dir: e.is_dir(),
                size: if e.is_dir() { None } else { Some(e.size) },
                modified: e.modified,
                created: e.created,
                attrs: e.attrs,
            });
            if hits.len() >= top {
                complete = false; // stopped at --top; more may exist
                break 'walk;
            }
        }
    }
    Ok(Found { hits, engine: "walk", complete, note: root.to_string_lossy().into_owned() })
}

fn parse_es_json(out: &str) -> Res<Vec<Hit>> {
    let t = out.trim();
    if t.is_empty() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_str(t).map_err(|e| format!("cannot read es.exe output: {e}"))?;
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut hits = Vec::with_capacity(arr.len());
    for it in arr {
        let raw = it["filename"].as_str().unwrap_or("").to_string();
        let attrs = it["attributes"].as_u64().unwrap_or(0) as u32;
        let is_dir = raw.ends_with('\\') || attrs & ATTR_DIRECTORY != 0;
        let path = if raw.len() > 3 { raw.trim_end_matches('\\').to_string() } else { raw };
        let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        hits.push(Hit {
            name,
            is_dir,
            size: it["size"].as_u64(),
            modified: it["date_modified"].as_u64().unwrap_or(0),
            created: it["date_created"].as_u64().unwrap_or(0),
            attrs,
            path,
        });
    }
    Ok(hits)
}

/// Folder size from the Everything index: (bytes, file count). None when Everything is not running or the folder
/// is not indexed. Index results can lag a second or two behind very fresh changes.
pub fn indexed_folder_size(dir: &Path) -> Option<(u64, u64)> {
    let d = dir.to_string_lossy().into_owned();
    // Two questions to Everything, asked at the same time.
    let (bytes, files) = std::thread::scope(|s| {
        let t = s.spawn(|| run_es(&["-get-result-count".into(), "-path".into(), d.clone(), "file:".into()]));
        let b = run_es(&["-get-total-size".into(), "-path".into(), d.clone(), "file:".into()]);
        (b, t.join().unwrap_or_else(|_| Err("es thread failed".into())))
    });
    let bytes: u64 = bytes.ok()?.trim().parse().ok()?;
    let files: u64 = files.ok()?.trim().parse().ok()?;
    if files == 0 {
        // An empty answer for a non-empty folder means the folder is outside the index.
        let has_any = std::fs::read_dir(dir).ok()?.next().is_some();
        if has_any {
            return None;
        }
    }
    Some((bytes, files))
}
