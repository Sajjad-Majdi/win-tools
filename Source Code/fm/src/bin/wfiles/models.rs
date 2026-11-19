//! The opt-in AI switch, kept in %LOCALAPPDATA%\WinTools\ai.json ("enabled"; shared with the Task Manager).
//! OFF by default. AI is online only (the user's own Gemini key, see ai.rs). The meaning search (semsearch) was
//! removed on 2026-11-20: AI Search finds things by meaning, fast, without a local index.
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Ai,
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
    }
    save(&v)
}

pub fn switch_on(s: Section) -> bool {
    let v = load();
    match s {
        Section::Ai => v["enabled"].as_bool().unwrap_or(false),
    }
}
