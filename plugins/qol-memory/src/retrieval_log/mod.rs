use std::io::Write;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const RETRIEVAL_SCHEMA: &str = "qol-memory-retrieval-v1";
pub const CANDIDATES_SCHEMA: &str = "qol-memory-candidates-v1";
pub const RETRIEVAL_LOG_CAP: u64 = 10 * 1024 * 1024;
pub const RETRIEVAL_LOG_TAIL: u64 = 1024 * 1024;

pub fn normalize_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if matches!(c, 'a'..='z' | '0'..='9') {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        } else {
            pending_space = true;
        }
    }
    out
}

pub fn candidate_key(norm_query: &str) -> String {
    let digest = Sha256::digest(norm_query.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex.truncate(16);
    hex
}

pub fn rotate_if_needed(path: &Path, cap: u64, tail: u64) {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return,
    };
    if size <= cap {
        return;
    }
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(_) => return,
    };
    if raw.len() as u64 <= cap {
        return;
    }
    let cut_at = raw.len().saturating_sub(tail as usize);
    let prefix_len = raw[..cut_at]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |pos| pos + 1);
    let _ = std::fs::write(path, &raw[prefix_len..]);
}

pub fn correctness_of(
    verdict: &str,
    answer_text: Option<&str>,
    fact: Option<&str>,
    source: &str,
) -> Option<String> {
    let fact = match fact {
        Some(value) if !value.is_empty() => value,
        _ => return None,
    };
    if source == "eval" && fact.starts_with("trap:") {
        let outcome = if verdict == "answered" {
            "trapped"
        } else {
            "untrapped"
        };
        return Some(outcome.to_string());
    }
    if verdict != "answered" {
        return Some("unanswered".to_string());
    }
    let norm_fact = normalize_query(fact);
    if norm_fact.is_empty() {
        return Some("correct".to_string());
    }
    let norm_answer = normalize_query(answer_text.unwrap_or(""));
    let outcome = if norm_answer.contains(&norm_fact) {
        "correct"
    } else {
        "wrong"
    };
    Some(outcome.to_string())
}

#[derive(Serialize)]
pub struct RetrievalEvent {
    pub ts: String,
    pub source: String,
    pub session: Option<String>,
    pub cwd: Option<String>,
    pub agent_home: String,
    pub query: String,
    pub verdict: String,
    pub confidence: String,
    pub correctness: Option<String>,
    pub latency_ms: u64,
    pub k: usize,
    pub exclusion: Exclusion,
    pub gates: serde_json::Value,
    pub signals: serde_json::Value,
    pub answer_key: Option<String>,
    pub recalled_keys: Vec<String>,
    pub counts: serde_json::Value,
}

#[derive(Serialize)]
pub struct Exclusion {
    pub exclude_session: bool,
    pub non_default_gates: bool,
}

fn append_inner(root: &Path, event: &RetrievalEvent) -> anyhow::Result<()> {
    std::fs::create_dir_all(root)?;
    let path = root.join("retrievals.jsonl");
    rotate_if_needed(&path, RETRIEVAL_LOG_CAP, RETRIEVAL_LOG_TAIL);
    let mut line = serde_json::to_vec(event)?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(&line)?;
    Ok(())
}

pub fn append(root: &Path, event: &RetrievalEvent) {
    if std::env::var("QOL_MEMORY_RETRIEVAL_LOG_DISABLE").is_ok_and(|value| value == "1") {
        return;
    }
    if let Err(err) = append_inner(root, event) {
        eprintln!("qol-memory: retrieval log append failed: {}", err);
    }
}

pub fn count_pending_candidates(root: &Path) -> usize {
    let raw = match std::fs::read(root.join("candidates.jsonl")) {
        Ok(raw) => raw,
        Err(_) => return 0,
    };
    String::from_utf8_lossy(&raw)
        .split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value.get("status").and_then(|s| s.as_str()) == Some("candidate"))
        .count()
}

pub fn last_event_ts(root: &Path) -> Option<String> {
    let raw = std::fs::read(root.join("retrievals.jsonl")).ok()?;
    String::from_utf8_lossy(&raw)
        .lines()
        .rev()
        .find_map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()?
                .get("ts")?
                .as_str()
                .map(str::to_owned)
        })
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
                "qol-memory-retrieval-log-{}-{}-{}",
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
    fn normalize_query_maps_collapses_and_trims() {
        assert_eq!(normalize_query(""), "");
        assert_eq!(
            normalize_query("  What   is QoL-Tools? "),
            "what is qol tools"
        );
        assert_eq!(normalize_query("\tMIXed\tCASE!!\n"), "mixed case");
        assert_eq!(normalize_query("--...__--"), "");
        assert_eq!(normalize_query("abc123 xyz"), "abc123 xyz");
    }

    #[test]
    fn candidate_key_matches_node_sha256_first16() {
        assert_eq!(
            candidate_key(&normalize_query("hello world")),
            "b94d27b9934d3e08"
        );
        assert_eq!(candidate_key("").len(), 16);
        assert_eq!(candidate_key(""), "e3b0c44298fc1c14",);
    }

    #[test]
    fn rotate_keeps_line_aligned_tail() {
        let dir = TempDir::new("rotate");
        let path = dir.0.join("retrievals.jsonl");
        let fmt_line = |i: usize| format!("line-{i:02}-{}", "x".repeat(15));
        let body: Vec<String> = (0..12).map(&fmt_line).collect();
        std::fs::write(&path, body.join("\n") + "\n").unwrap();

        rotate_if_needed(&path, 100, 60);

        let kept = std::fs::read_to_string(&path).unwrap();
        assert_eq!(kept.len(), 3 * (fmt_line(0).len() + 1));
        for i in 9..12 {
            assert!(kept.contains(&fmt_line(i)));
        }
        assert!(!kept.contains(&fmt_line(8)));

        let before = std::fs::read_to_string(&path).unwrap();
        rotate_if_needed(&path, 100, 60);
        assert_eq!(before, std::fs::read_to_string(&path).unwrap());

        let missing = dir.0.join("missing.jsonl");
        rotate_if_needed(&missing, 10, 5);
        assert!(!missing.exists());
    }

    #[test]
    fn correctness_table_matches_js() {
        assert_eq!(correctness_of("answered", Some("v"), None, "ask-cli"), None);
        assert_eq!(
            correctness_of("answered", Some("v"), Some(""), "ask-cli"),
            None
        );
        assert_eq!(
            correctness_of("answered", Some("42"), Some("trap: what version"), "eval"),
            Some("trapped".to_string())
        );
        assert_eq!(
            correctness_of("no-memory", None, Some("trap: what version"), "eval"),
            Some("untrapped".to_string())
        );
        assert_eq!(
            correctness_of("no-memory", None, Some("fact"), "ask-cli"),
            Some("unanswered".to_string())
        );
        assert_eq!(
            correctness_of("answered", Some("anything"), Some("!@#"), "ask-cli"),
            Some("correct".to_string())
        );

        let contains_case = correctness_of(
            "answered",
            Some("The Dock is at the BOTTOM of the screen."),
            Some("bottom of the screen"),
            "ask-cli",
        );
        assert_eq!(contains_case, Some("correct".to_string()));

        let wrong_case = correctness_of(
            "answered",
            Some("completely unrelated"),
            Some("bottom of the screen"),
            "ask-cli",
        );
        assert_eq!(wrong_case, Some("wrong".to_string()));
    }

    #[test]
    fn append_writes_line_and_honors_disable_env() {
        let dir = TempDir::new("append");
        let log_path = dir.0.join("retrievals.jsonl");
        let event = sample_event("correct");
        append(dir.0.as_path(), &event);
        append(dir.0.as_path(), &sample_event("wrong"));
        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["verdict"], "answered");
        assert_eq!(first["agent_home"], "/home/tester/.claude");
        assert_eq!(first["exclusion"]["exclude_session"], false);
        assert_eq!(first["recalled_keys"][0], "note-key");

        std::env::set_var("QOL_MEMORY_RETRIEVAL_LOG_DISABLE", "1");
        append(dir.0.as_path(), &sample_event("no-memory"));
        std::env::remove_var("QOL_MEMORY_RETRIEVAL_LOG_DISABLE");
        let after = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(after.lines().count(), 2);
    }

    fn sample_event(correctness: &str) -> RetrievalEvent {
        RetrievalEvent {
            ts: crate::text::now_iso(),
            source: "ask-cli".to_string(),
            session: None,
            cwd: None,
            agent_home: "/home/tester/.claude".to_string(),
            query: "dock position".to_string(),
            verdict: "answered".to_string(),
            confidence: "high".to_string(),
            correctness: Some(correctness.to_string()),
            latency_ms: 12,
            k: 5,
            exclusion: Exclusion {
                exclude_session: false,
                non_default_gates: false,
            },
            gates: serde_json::json!({"FLOOR": 1.2}),
            signals: serde_json::json!({"top_unit_score": 3.5}),
            answer_key: Some("unit-key".to_string()),
            recalled_keys: vec!["note-key".to_string()],
            counts: serde_json::json!({"units": 4}),
        }
    }

    #[test]
    fn candidates_and_last_event_ts_read_the_store_files() {
        let dir = TempDir::new("status-reads");
        assert_eq!(count_pending_candidates(dir.0.as_path()), 0);
        assert_eq!(last_event_ts(dir.0.as_path()), None);

        std::fs::write(
            dir.0.join("candidates.jsonl"),
            "{\"status\":\"candidate\"}\n{\"status\":\"promoted\"}\nbroken\n{\"status\":\"candidate\",\"query\":\"q\"}\n",
        )
        .unwrap();
        assert_eq!(count_pending_candidates(dir.0.as_path()), 2);

        std::fs::write(
            dir.0.join("retrievals.jsonl"),
            "{\"ts\":\"2026-08-01T00:00:00.000Z\"}\nnot json\n{\"ts\":\"2026-08-02T00:00:00.000Z\",\"verdict\":\"answered\"}\n{\"verdict\":\"no-memory\"}\n",
        )
        .unwrap();
        assert_eq!(
            last_event_ts(dir.0.as_path()).as_deref(),
            Some("2026-08-02T00:00:00.000Z")
        );
    }
}
