use std::path::Path;

use crate::session::registry::SessionState;

pub fn load(path: &Path) -> Vec<SessionState> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, sessions: &[SessionState]) {
    match serde_json::to_string_pretty(sessions) {
        Ok(json) => {
            if let Err(e) = qol_fs::atomic_write(path, json.as_bytes()) {
                eprintln!("[cli-sessions] persist write failed: {e}");
            }
        }
        Err(e) => eprintln!("[cli-sessions] persist serialize failed: {e}"),
    }
}
