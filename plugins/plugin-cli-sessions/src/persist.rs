use std::path::Path;

use crate::registry::SessionState;

pub fn load(path: &Path) -> Vec<SessionState> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, sessions: &[SessionState]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(sessions) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                eprintln!("[cli-sessions] persist write failed: {e}");
            }
        }
        Err(e) => eprintln!("[cli-sessions] persist serialize failed: {e}"),
    }
}
