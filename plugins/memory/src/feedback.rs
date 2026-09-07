use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use serde_json::Value;

pub fn append_vote(root: &Path, norm: &str, key: &str, vote: i64) {
    let line = serde_json::json!({
        "norm": norm,
        "key": key,
        "vote": vote,
        "ts": crate::text::now_iso()
    });
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("feedback.jsonl"))
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
