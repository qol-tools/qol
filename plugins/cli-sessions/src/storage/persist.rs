use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::session::registry::SessionState;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnfoldedPanel {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn load(path: &Path) -> Vec<SessionState> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, sessions: &[SessionState]) -> bool {
    match serde_json::to_string_pretty(sessions) {
        Ok(json) => {
            if let Err(e) = qol_fs::atomic_write(path, json.as_bytes()) {
                log::warn!("persist write failed: {e}");
                return false;
            }
            true
        }
        Err(e) => {
            log::warn!("persist serialize failed: {e}");
            false
        }
    }
}

pub fn save_unfolded_panel(path: &Path, panel: UnfoldedPanel) {
    let json = match serde_json::to_vec(&panel) {
        Ok(json) => json,
        Err(error) => {
            log::warn!("[cli-sessions] unfolded panel encode failed: {error}");
            return;
        }
    };
    if let Err(error) = qol_fs::atomic_write(path, &json) {
        log::warn!("[cli-sessions] unfolded panel write failed: {error}");
    }
}

pub fn load_unfolded_panel(path: &Path) -> Option<UnfoldedPanel> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn clear_unfolded_panel(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("[cli-sessions] unfolded panel clear failed: {error}"),
    }
}
