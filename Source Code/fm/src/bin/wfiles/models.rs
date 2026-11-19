//! Opt-in switches kept in %LOCALAPPDATA%\WinTools\ai.json (shared with the Task Manager; reduced copy of its
//! app/src/models.rs): "enabled" (AI features) and "semsearch": {"enabled", "cmd"} (meaning search). Both OFF by
//! default. AI is online only (the user's own Gemini key, see ai.rs); meaning search uses an existing semsearch
//! install, nothing is downloaded by this app.
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Ai,
    Semsearch,
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

/// An existing semsearch install: config "cmd", else the known default location (with its index folder).
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

/// Meaning search usable: its switch is on and an install exists.
pub fn semsearch_enabled() -> bool {
    switch_on(Section::Semsearch) && semsearch_detect().is_some()
}
