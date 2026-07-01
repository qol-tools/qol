use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::filters::ViewFilters;
use super::TraceRate;

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ConsoleState {
    #[serde(default)]
    pub(super) filters: ViewFilters,
    #[serde(default)]
    pub(super) trace_details: bool,
    #[serde(default)]
    pub(super) trace_rate: TraceRate,
    #[serde(default)]
    pub(super) keys_hidden: bool,
    #[serde(default)]
    pub(super) feature_flags: Vec<String>,
}

fn console_state_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("dev/console.json"))
}

pub(super) fn load_console_state() -> ConsoleState {
    let Some(path) = console_state_path() else {
        return ConsoleState::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return ConsoleState::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub(super) fn save_console_state(state: &ConsoleState) {
    let Some(path) = console_state_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if fs::write(&tmp, json).is_err() {
        return;
    }
    let _ = fs::rename(&tmp, &path);
}
