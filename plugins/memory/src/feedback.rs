use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use serde_json::Value;

pub const FEEDBACK_LOG_CAP: u64 = crate::retrieval_log::RETRIEVAL_LOG_CAP;
pub const FEEDBACK_LOG_TAIL: u64 = crate::retrieval_log::RETRIEVAL_LOG_TAIL;

pub fn append_vote(root: &Path, norm: &str, key: &str, vote: i64) {
    let line = serde_json::json!({
        "norm": norm,
        "key": key,
        "vote": vote,
        "ts": crate::text::now_iso()
    });
    let path = root.join("feedback.jsonl");
    crate::retrieval_log::rotate_if_needed(&path, FEEDBACK_LOG_CAP, FEEDBACK_LOG_TAIL);
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{line}"));
    if let Err(error) = result {
        eprintln!("qol-memory: feedback append failed: {error}");
    }
}

pub fn disliked_by_norm(root: &Path) -> HashMap<String, HashSet<String>> {
    let raw = match std::fs::read(root.join("feedback.jsonl")) {
        Ok(raw) => raw,
        Err(_) => return HashMap::new(),
    };
    let mut disliked: HashMap<String, HashSet<String>> = HashMap::new();
    for line in String::from_utf8_lossy(&raw).lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("vote").and_then(Value::as_i64).unwrap_or(0) >= 0 {
            continue;
        }
        let (Some(norm), Some(key)) = (
            value.get("norm").and_then(Value::as_str),
            value.get("key").and_then(Value::as_str),
        ) else {
            continue;
        };
        disliked
            .entry(norm.to_string())
            .or_default()
            .insert(key.to_string());
    }
    disliked
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-feedback-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn append_vote_rotates_feedback_log_and_keeps_the_tail() {
        let dir = TempDir::new("rotate");
        let path = dir.0.join("feedback.jsonl");
        let body: Vec<String> = (0..12)
            .map(|i| {
                serde_json::json!({
                    "norm": "n",
                    "key": format!("k-{i}"),
                    "vote": 0,
                    "pad": "x".repeat(1024 * 1024)
                })
                .to_string()
            })
            .collect();
        std::fs::write(&path, body.join("\n") + "\n").unwrap();

        append_vote(dir.0.as_path(), "dock position", "unit-1", -1);

        assert!(std::fs::metadata(&path).unwrap().len() <= FEEDBACK_LOG_CAP);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("k-0"));
        assert!(raw.contains("k-11"));
        let last: Value = serde_json::from_str(raw.lines().last().unwrap()).unwrap();
        assert_eq!(last["norm"], "dock position");
        assert_eq!(last["key"], "unit-1");
        assert_eq!(last["vote"], -1);
        let disliked = disliked_by_norm(dir.0.as_path());
        assert_eq!(disliked.get("dock position").unwrap().len(), 1);
    }
}
