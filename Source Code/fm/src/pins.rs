//! Quick access: pinned folders (kept in %LOCALAPPDATA%\WinTools\fm.json) and recent files (the .lnk files Windows
//! keeps in %APPDATA%\Microsoft\Windows\Recent).
use crate::util::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn config_path() -> PathBuf {
    data_dir().join("fm.json")
}

/// The whole fm.json as JSON (an empty object when missing or broken).
pub fn config() -> Value {
    std::fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).filter(|v: &Value| v.is_object()).unwrap_or_else(|| json!({}))
}

/// Write fm.json atomically (temp file + rename).
pub fn save_config(v: &Value) -> Res<()> {
    let p = config_path();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| format!("cannot create {}: {e}", data_dir().display()))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(v).unwrap()).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("cannot write {}: {e}", p.display()))
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub path: String,
    pub name: String,
    pub exists: bool,
}

pub fn pins() -> Vec<Pin> {
    config()["pins"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let path = p["path"].as_str()?.to_string();
                    let name = p["name"].as_str().map(String::from).unwrap_or_else(|| default_name(Path::new(&path)));
                    Some(Pin { exists: Path::new(&path).exists(), path, name })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn default_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

/// Pin a folder or file (no duplicates). Returns the pin list after the change.
pub fn pin(path: &Path, name: Option<&str>) -> Res<Vec<Pin>> {
    let path = abs(path);
    if !path.exists() {
        return Err(format!("not found: {}", path.display()));
    }
    let mut cfg = config();
    let mut arr = cfg["pins"].as_array().cloned().unwrap_or_default();
    arr.retain(|p| !path_eq(Path::new(p["path"].as_str().unwrap_or("")), &path));
    arr.push(json!({"path": path.to_string_lossy(), "name": name.map(String::from).unwrap_or_else(|| default_name(&path))}));
    cfg["pins"] = Value::Array(arr);
    save_config(&cfg)?;
    Ok(pins())
}

/// Remove a pin (by path, or by pin name). Error when nothing matched.
pub fn unpin(path_or_name: &str) -> Res<Vec<Pin>> {
    let full = abs(path_or_name);
    let mut cfg = config();
    let mut arr = cfg["pins"].as_array().cloned().unwrap_or_default();
    let before = arr.len();
    arr.retain(|p| {
        let pp = p["path"].as_str().unwrap_or("");
        let nm = p["name"].as_str().unwrap_or("");
        !(path_eq(Path::new(pp), &full) || nm.eq_ignore_ascii_case(path_or_name))
    });
    if arr.len() == before {
        return Err(format!("not pinned: {path_or_name}"));
    }
    cfg["pins"] = Value::Array(arr);
    save_config(&cfg)?;
    Ok(pins())
}

#[derive(Clone, Debug)]
pub struct Recent {
    pub target: String,
    pub name: String,
    /// When the shortcut was last updated (= last opened), FILETIME.
    pub used: u64,
    pub is_dir: bool,
    pub exists: bool,
}

/// The Recent folder (%APPDATA%\Microsoft\Windows\Recent).
pub fn recent_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Recent"))
}

/// Recently opened files and folders, newest first. Reads only the newest `top` shortcuts.
pub fn recent(top: usize) -> Res<Vec<Recent>> {
    let dir = recent_dir().ok_or("APPDATA is not set")?;
    recent_in(&dir, top)
}

/// Same as `recent`, from any folder of .lnk files.
pub fn recent_in(dir: &Path, top: usize) -> Res<Vec<Recent>> {
    let mut lnks: Vec<crate::list::Entry> = crate::list::list_dir(dir, true)?.into_iter().filter(|e| !e.is_dir() && e.ext() == "lnk").collect();
    lnks.sort_by(|a, b| b.modified.cmp(&a.modified));
    let _com = com();
    let mut out = vec![];
    for e in lnks {
        if out.len() >= top {
            break;
        }
        let Some(li) = crate::info::read_shortcut(&dir.join(&e.name)) else { continue };
        if li.target.is_empty() {
            continue;
        }
        let tp = Path::new(&li.target);
        let md = std::fs::metadata(tp);
        out.push(Recent {
            name: default_name(tp),
            is_dir: md.as_ref().map(|m| m.is_dir()).unwrap_or(false),
            exists: md.is_ok(),
            used: e.modified,
            target: li.target,
        });
    }
    Ok(out)
}
