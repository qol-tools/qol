use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::cli::{tail, CliActivityEvidence, CliRuntimeState};
use crate::SessionFacts;

use super::environment::PiEnvironment;
use crate::cli::activity::{quiet_secs, recently_active};

const SESSION_CACHE_TTL: Duration = Duration::from_secs(30);
const REVERSE_READ_CHUNK: u64 = 64 * 1024;

pub(super) struct PiMetadata {
    pub session_name: Option<String>,
    pub external_id: Option<String>,
    pub has_activity: Option<bool>,
    pub runtime: CliRuntimeState,
    pub activity: CliActivityEvidence,
}

pub(super) struct PiMetadataResolver {
    environment: Arc<dyn PiEnvironment>,
    cache: Mutex<PiCache>,
}

#[derive(Default)]
struct PiCache {
    session_files: HashMap<i32, Timed<Option<PathBuf>>>,
    session_file_lists: HashMap<i32, Timed<Vec<PathBuf>>>,
    facts: HashMap<PathBuf, CachedFacts>,
}

struct Timed<T> {
    value: T,
    checked_at: Instant,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct FileSignature {
    modified: Option<SystemTime>,
    length: u64,
}

struct CachedFacts {
    signature: FileSignature,
    scanned_length: u64,
    session_name: Option<String>,
    has_message: bool,
}

impl PiMetadataResolver {
    pub fn new(environment: Arc<dyn PiEnvironment>) -> Self {
        Self {
            environment,
            cache: Mutex::new(PiCache::default()),
        }
    }

    pub fn resolve(&self, session: &SessionFacts) -> PiMetadata {
        let mut cache = self.cache.lock().ok();
        let path = cache
            .as_mut()
            .and_then(|cache| session_file(session, self.environment.as_ref(), cache));
        let external_id = path.as_deref().and_then(id_from_path);
        let facts = cache
            .as_mut()
            .and_then(|cache| path.as_deref().and_then(|path| cached_facts(path, cache)));
        let session_name = facts.as_ref().and_then(|facts| facts.session_name.clone());
        let activity = facts
            .map(|facts| CliActivityEvidence {
                file_fresh: recently_active(facts.signature.modified),
                file_has_work: Some(facts.has_message),
                file_quiet_secs: quiet_secs(facts.signature.modified),
            })
            .unwrap_or_default();
        PiMetadata {
            session_name,
            external_id,
            has_activity: activity.combined(),
            runtime: path
                .as_deref()
                .map(tail_runtime)
                .unwrap_or(CliRuntimeState::Unknown),
            activity,
        }
    }

    pub fn subscription_path(&self, session: &SessionFacts) -> Option<PathBuf> {
        let mut cache = self.cache.lock().ok()?;
        session_file(session, self.environment.as_ref(), &mut cache)
    }

    pub fn subscription_paths(&self, session: &SessionFacts) -> Vec<PathBuf> {
        let Some(mut cache) = self.cache.lock().ok() else {
            return Vec::new();
        };
        session_files(session, self.environment.as_ref(), &mut cache)
    }
}

fn session_file(
    session: &SessionFacts,
    environment: &dyn PiEnvironment,
    cache: &mut PiCache,
) -> Option<PathBuf> {
    session
        .foreground_pids
        .iter()
        .find_map(|pid| cached_session_file(*pid, &session.cwd, environment, cache))
}

fn cached_session_file(
    pid: i32,
    cwd: &str,
    environment: &dyn PiEnvironment,
    cache: &mut PiCache,
) -> Option<PathBuf> {
    if let Some(entry) = cache.session_files.get(&pid) {
        if entry.checked_at.elapsed() < SESSION_CACHE_TTL {
            return entry.value.clone();
        }
    }
    let value = environment.session_file(pid, cwd);
    cache.session_files.insert(
        pid,
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

fn session_files(
    session: &SessionFacts,
    environment: &dyn PiEnvironment,
    cache: &mut PiCache,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = session
        .foreground_pids
        .iter()
        .flat_map(|pid| cached_session_files(*pid, &session.cwd, environment, cache))
        .collect();
    candidates.sort();
    candidates.dedup();
    candidates
}

fn cached_session_files(
    pid: i32,
    cwd: &str,
    environment: &dyn PiEnvironment,
    cache: &mut PiCache,
) -> Vec<PathBuf> {
    if let Some(entry) = cache.session_file_lists.get(&pid) {
        if entry.checked_at.elapsed() < SESSION_CACHE_TTL {
            return entry.value.clone();
        }
    }
    let value = environment.session_files(pid, cwd);
    cache.session_file_lists.insert(
        pid,
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

pub(super) fn id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let (_, id) = stem.rsplit_once('_')?;
    (!id.is_empty()).then(|| id.to_owned())
}

fn cached_facts(path: &Path, cache: &mut PiCache) -> Option<CachedFacts> {
    let signature = file_signature(path)?;
    if let Some(entry) = cache.facts.get(path) {
        if entry.signature == signature {
            return Some(clone_facts(entry));
        }
        if signature.length >= entry.scanned_length && entry.scanned_length > 0 {
            let (name, message) = scan_appended(path, entry.scanned_length);
            let facts = CachedFacts {
                signature,
                scanned_length: tail::last_complete_line(path).map_or(0, |line| line.end),
                session_name: name.or_else(|| entry.session_name.clone()),
                has_message: entry.has_message || message,
            };
            cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
            return Some(facts);
        }
    }
    let facts = CachedFacts {
        signature,
        scanned_length: tail::last_complete_line(path).map_or(0, |line| line.end),
        session_name: latest_session_name(path),
        has_message: any_message(path),
    };
    cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
    Some(facts)
}

fn clone_facts(facts: &CachedFacts) -> CachedFacts {
    CachedFacts {
        signature: facts.signature,
        scanned_length: facts.scanned_length,
        session_name: facts.session_name.clone(),
        has_message: facts.has_message,
    }
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

fn scan_appended(path: &Path, offset: u64) -> (Option<String>, bool) {
    let Ok(mut file) = fs::File::open(path) else {
        return (None, false);
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (None, false);
    }
    let mut appended = Vec::new();
    if file.read_to_end(&mut appended).is_err() {
        return (None, false);
    }
    let mut name = None;
    let mut message = false;
    for line in appended.split(|byte| *byte == b'\n') {
        if let Some(found) = session_name(line) {
            name = Some(found);
        }
        if is_message(line) {
            message = true;
        }
    }
    (name, message)
}

fn latest_session_name(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut cursor = file.metadata().ok()?.len();
    let mut suffix = Vec::new();
    while cursor > 0 {
        let start = cursor.saturating_sub(REVERSE_READ_CHUNK);
        let length = usize::try_from(cursor - start).ok()?;
        let mut chunk = vec![0; length];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&suffix);
        let lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let complete_from = usize::from(start > 0);
        for line in lines[complete_from..].iter().rev() {
            if let Some(name) = session_name(line) {
                return Some(name);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    session_name(&suffix)
}

fn any_message(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    std::io::BufRead::lines(std::io::BufReader::new(file))
        .map_while(Result::ok)
        .any(|line| is_message(line.as_bytes()))
}

fn is_message(line: &[u8]) -> bool {
    memchr_contains(line, b"\"type\":\"message\"")
}

fn session_name(line: &[u8]) -> Option<String> {
    if !memchr_contains(line, b"\"type\":\"session_info\"") {
        return None;
    }
    let value = serde_json::from_slice::<Value>(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("session_info") {
        return None;
    }
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn memchr_contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub(super) fn marker_in_terminal_assistant_text(path: &Path, marker: &str) -> Option<bool> {
    let mut file = fs::File::open(path).ok()?;
    let mut cursor = file.metadata().ok()?.len();
    let mut suffix = Vec::new();
    while cursor > 0 {
        let start = cursor.saturating_sub(REVERSE_READ_CHUNK);
        let length = usize::try_from(cursor - start).ok()?;
        let mut chunk = vec![0; length];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&suffix);
        let lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let complete_from = usize::from(start > 0);
        for line in lines[complete_from..].iter().rev() {
            if let Some(completed) = assistant_text_marker(line, marker) {
                return Some(completed);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    assistant_text_marker(&suffix, marker)
}

fn assistant_text_marker(line: &[u8], marker: &str) -> Option<bool> {
    if !memchr_contains(line, b"\"type\":\"message\"") {
        return None;
    }
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return None;
    };
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let terminal = message
        .get("stopReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| reason != "toolUse");
    let text_contains = message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .any(|text| crate::marker::marker_close_tolerant(text, marker));
    Some(terminal && text_contains)
}

struct TerminalAssistant {
    text: String,
    timestamp_millis: Option<i64>,
}

fn latest_terminal_assistant(path: &Path) -> Option<TerminalAssistant> {
    let mut file = fs::File::open(path).ok()?;
    let mut cursor = file.metadata().ok()?.len();
    let mut suffix = Vec::new();
    while cursor > 0 {
        let start = cursor.saturating_sub(REVERSE_READ_CHUNK);
        let length = usize::try_from(cursor - start).ok()?;
        let mut chunk = vec![0; length];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&suffix);
        let lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let complete_from = usize::from(start > 0);
        for line in lines[complete_from..].iter().rev() {
            if let Some(assistant) = terminal_assistant(line) {
                return Some(assistant);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    terminal_assistant(&suffix)
}

fn terminal_assistant(line: &[u8]) -> Option<TerminalAssistant> {
    if !memchr_contains(line, b"\"type\":\"message\"") {
        return None;
    }
    let value = serde_json::from_slice::<Value>(line).ok()?;
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let terminal = message
        .get("stopReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| reason != "toolUse");
    if !terminal {
        return None;
    }
    let text = message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let timestamp_millis = value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|stamp| stamp.timestamp_millis());
    Some(TerminalAssistant {
        text,
        timestamp_millis,
    })
}

pub(super) fn terminal_report_after(path: &Path, since_millis: i64) -> Option<String> {
    let assistant = latest_terminal_assistant(path)?;
    let stamp = assistant.timestamp_millis?;
    if stamp < since_millis {
        return None;
    }
    (!assistant.text.is_empty()).then_some(assistant.text)
}

pub(super) fn marked_terminal_text(path: &Path, marker: &str) -> Option<String> {
    let assistant = latest_terminal_assistant(path)?;
    crate::marker::marker_close_tolerant(&assistant.text, marker)
        .then_some(assistant.text)
        .filter(|text| !text.is_empty())
}

pub(super) fn transcript_owned_by(path: &Path, marker: &str) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    std::io::BufRead::lines(std::io::BufReader::new(file))
        .map_while(Result::ok)
        .any(|line| user_message_mentions_marker(&line, marker))
}

fn user_message_mentions_marker(line: &str, marker: &str) -> bool {
    if !memchr_contains(line.as_bytes(), b"QOL_BRIDGE_DONE") {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let Some(message) = value.get("message") else {
        return false;
    };
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return false;
    }
    let text = message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    crate::marker::marker_close_tolerant(&text, marker)
}

pub(super) fn transcript_report(
    paths: &[std::path::PathBuf],
    since_millis: i64,
    marker: &str,
) -> Option<String> {
    for path in paths {
        if !transcript_owned_by(path, marker) {
            continue;
        }
        if let Some(text) = terminal_report_after(path, since_millis) {
            return Some(text);
        }
    }
    None
}

pub(super) fn transcript_runtime(
    paths: &[std::path::PathBuf],
    marker: &str,
) -> Option<CliRuntimeState> {
    let path = paths
        .iter()
        .find(|path| transcript_owned_by(path, marker))?;
    Some(tail_runtime(path))
}

pub(super) fn tail_runtime(path: &Path) -> CliRuntimeState {
    let Some(line) = tail::last_complete_line(path) else {
        return CliRuntimeState::Ready;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&line.bytes) else {
        return CliRuntimeState::Working;
    };
    let terminal = value.get("type").and_then(Value::as_str) == Some("message")
        && value
            .get("message")
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            == Some("assistant")
        && value
            .get("message")
            .and_then(|message| message.get("stopReason"))
            .and_then(Value::as_str)
            .is_some_and(|reason| reason != "toolUse");
    if terminal {
        CliRuntimeState::Ready
    } else {
        CliRuntimeState::Working
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::marker_in_terminal_assistant_text;

    fn message(role: &str, stop_reason: Option<&str>, blocks: &[(&str, &str)]) -> String {
        let content = blocks
            .iter()
            .map(|(kind, text)| {
                format!(
                    "{{\"type\":\"{kind}\",\"text\":{}}}",
                    serde_json::to_string(text).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let stop = stop_reason
            .map(|reason| format!(",\"stopReason\":\"{reason}\""))
            .unwrap_or_default();
        format!(
            "{{\"type\":\"message\",\"message\":{{\"role\":\"{role}\",\"content\":[{content}]{stop}}}}}\n"
        )
    }

    fn write(lines: &[String]) -> (TempDir, std::path::PathBuf) {
        let root = TempDir::new().unwrap();
        let path = root.path().join("session.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        for line in lines {
            file.write_all(line.as_bytes()).unwrap();
        }
        (root, path)
    }

    #[test]
    fn marker_in_thinking_block_is_not_a_completion() {
        let (_root, path) = write(&[
            message("user", None, &[("text", "finish with QOL_BRIDGE_DONE_x")]),
            message(
                "assistant",
                Some("end_turn"),
                &[
                    ("thinking", "reconstruct QOL_BRIDGE_DONE_x now"),
                    ("text", "done"),
                ],
            ),
        ]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_x"),
            Some(false)
        );
    }

    #[test]
    fn marker_in_text_with_tool_use_stop_reason_is_not_a_completion() {
        let (_root, path) = write(&[message(
            "assistant",
            Some("toolUse"),
            &[
                ("text", "calling with QOL_BRIDGE_DONE_x"),
                ("thinking", "plan"),
            ],
        )]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_x"),
            Some(false)
        );
    }

    #[test]
    fn marker_in_text_with_terminal_stop_reason_completes() {
        let (_root, path) = write(&[
            message("user", None, &[("text", "go")]),
            message(
                "assistant",
                Some("end_turn"),
                &[
                    ("thinking", "internal"),
                    ("text", "finished QOL_BRIDGE_DONE_y"),
                ],
            ),
        ]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_y"),
            Some(true)
        );
    }

    #[test]
    fn mangled_marker_in_terminal_text_still_completes() {
        let (_root, path) = write(&[message(
            "assistant",
            Some("end_turn"),
            &[("text", "finished QOL_BRIDGE_DONE_4aab0331027f21a7322")],
        )]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_4aab033102f7a21f7322"),
            Some(true)
        );
    }

    #[test]
    fn only_the_latest_assistant_message_counts() {
        let (_root, path) = write(&[
            message(
                "assistant",
                Some("end_turn"),
                &[("text", "earlier QOL_BRIDGE_DONE_z")],
            ),
            message("user", None, &[("text", "keep going")]),
            message(
                "assistant",
                Some("end_turn"),
                &[("text", "finished without the marker")],
            ),
        ]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_z"),
            Some(false)
        );
    }

    #[test]
    fn marker_in_text_without_a_stop_reason_is_not_a_completion() {
        let (_root, path) = write(&[message(
            "assistant",
            None,
            &[("text", "still writing QOL_BRIDGE_DONE_w")],
        )]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_w"),
            Some(false)
        );
    }

    #[test]
    fn missing_or_empty_transcript_has_no_verdict() {
        let root = TempDir::new().unwrap();
        assert_eq!(
            marker_in_terminal_assistant_text(&root.path().join("absent.jsonl"), "MARKER"),
            None
        );
        let (_root, path) = write(&[]);
        assert_eq!(marker_in_terminal_assistant_text(&path, "MARKER"), None);
    }

    #[test]
    fn the_split_fragment_form_counts_in_terminal_assistant_text() {
        let (_root, path) = write(&[message(
            "assistant",
            Some("end_turn"),
            &[(
                "text",
                "Completion fragments: `QOL_BRIDGE_DONE_` and `abc123`.",
            )],
        )]);
        assert_eq!(
            marker_in_terminal_assistant_text(&path, "QOL_BRIDGE_DONE_abc123"),
            Some(true)
        );
    }

    #[test]
    fn marked_terminal_text_returns_the_final_message_for_a_matching_marker() {
        use super::marked_terminal_text;
        let (_root, path) = write(&[
            message("user", None, &[("text", "go")]),
            message(
                "assistant",
                Some("end_turn"),
                &[
                    ("thinking", "internal"),
                    ("text", "the full report"),
                    ("text", "QOL_BRIDGE_DONE_abc123"),
                ],
            ),
        ]);
        assert_eq!(
            marked_terminal_text(&path, "QOL_BRIDGE_DONE_abc123").as_deref(),
            Some("the full report\nQOL_BRIDGE_DONE_abc123")
        );
    }

    #[test]
    fn terminal_report_after_ignores_older_turns_and_rejects_missing_timestamps() {
        use super::terminal_report_after;
        let boundary = chrono::DateTime::parse_from_rfc3339("2026-08-03T09:01:00.000Z")
            .unwrap()
            .timestamp_millis();
        let (_root, path) = write(&[
            "{\"type\":\"message\",\"timestamp\":\"2026-08-03T09:00:00.000Z\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"earlier\"}]}}\n"
                .to_string(),
        ]);
        assert_eq!(terminal_report_after(&path, boundary), None);
        let (_root, path) = write(&[
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"later\"}]}}\n"
                .to_string(),
        ]);
        assert_eq!(terminal_report_after(&path, boundary), None);
        let (_root, path) = write(&[
            "{\"type\":\"message\",\"timestamp\":\"2026-08-03T09:05:00.000Z\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"end_turn\",\"content\":[{\"type\":\"text\",\"text\":\"fresh\"}]}}\n"
                .to_string(),
        ]);
        assert_eq!(
            terminal_report_after(&path, boundary).as_deref(),
            Some("fresh")
        );
    }

    fn stamped(role: &str, stop: Option<&str>, text: &str, stamp: &str) -> String {
        let stop = stop
            .map(|reason| format!(",\"stopReason\":\"{reason}\""))
            .unwrap_or_default();
        format!(
            "{{\"type\":\"message\",\"timestamp\":\"{stamp}\",\"message\":{{\"role\":\"{role}\"{stop},\"content\":[{{\"type\":\"text\",\"text\":{}}}]}}}}\n",
            serde_json::to_string(text).unwrap()
        )
    }

    #[test]
    fn a_fresh_sibling_transcript_is_never_captured_for_this_lane() {
        use super::{transcript_owned_by, transcript_report};
        let root = tempfile::TempDir::new().unwrap();
        let sibling = root.path().join("2026-08-23T19-52-50-000Z_a.jsonl");
        let own = root.path().join("2026-08-23T19-52-51-000Z_b.jsonl");
        std::fs::write(
            &sibling,
            format!(
                "{} {}",
                stamped(
                    "user",
                    None,
                    "task QOL_BRIDGE_DONE_sibling",
                    "2026-08-23T19:52:50.000Z"
                ),
                stamped(
                    "assistant",
                    Some("end_turn"),
                    "SIBLING REPORT QOL_BRIDGE_DONE_sibling",
                    "2026-08-23T19:55:00.000Z"
                ),
            ),
        )
        .unwrap();
        std::fs::write(
            &own,
            format!(
                "{} {}",
                stamped(
                    "user",
                    None,
                    "task QOL_BRIDGE_DONE_own",
                    "2026-08-23T19:52:51.000Z"
                ),
                stamped(
                    "assistant",
                    Some("end_turn"),
                    "OWN REPORT QOL_BRIDGE_DONE_own",
                    "2026-08-23T19:56:00.000Z"
                ),
            ),
        )
        .unwrap();
        let paths = vec![sibling.clone(), own.clone()];
        assert!(!transcript_owned_by(&sibling, "QOL_BRIDGE_DONE_own"));
        assert!(transcript_owned_by(&own, "QOL_BRIDGE_DONE_own"));
        let report = transcript_report(&paths, 0, "QOL_BRIDGE_DONE_own")
            .expect("the own transcript must produce the report");
        assert!(
            report.contains("OWN REPORT"),
            "the capture must be the lane's own report: {report}"
        );
        assert!(
            !report.contains("SIBLING"),
            "a sibling report must never be captured: {report}"
        );
    }

    #[test]
    fn the_runtime_tail_follows_the_owned_transcript_not_a_ready_sibling() {
        use super::transcript_runtime;
        use crate::cli::CliRuntimeState;
        let root = tempfile::TempDir::new().unwrap();
        let sibling = root.path().join("2026-08-23T19-52-50-000Z_a.jsonl");
        let own = root.path().join("2026-08-23T19-52-51-000Z_b.jsonl");
        std::fs::write(
            &sibling,
            format!(
                "{} {}",
                stamped(
                    "user",
                    None,
                    "task QOL_BRIDGE_DONE_sibling",
                    "2026-08-23T19:52:50.000Z"
                ),
                stamped(
                    "assistant",
                    Some("end_turn"),
                    "SIBLING DONE QOL_BRIDGE_DONE_sibling",
                    "2026-08-23T19:55:00.000Z"
                ),
            ),
        )
        .unwrap();
        std::fs::write(
            &own,
            format!(
                "{} {}",
                stamped("user", None, "task QOL_BRIDGE_DONE_own", "2026-08-23T19:52:51.000Z"),
                "{\"type\":\"message\",\"timestamp\":\"2026-08-23T19:56:00.000Z\",\"message\":{\"role\":\"tool\",\"content\":[{\"type\":\"toolResult\",\"toolUseId\":\"x\",\"content\":\"still working\"}]}}\n",
            ),
        )
        .unwrap();
        let paths = vec![sibling.clone(), own.clone()];
        assert_eq!(
            transcript_runtime(&paths, "QOL_BRIDGE_DONE_own"),
            Some(CliRuntimeState::Working),
            "the runtime must follow the lane's own tail, not the ready sibling"
        );
    }
}
