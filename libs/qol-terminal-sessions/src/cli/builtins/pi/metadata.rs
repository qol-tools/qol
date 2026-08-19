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

fn id_from_path(path: &Path) -> Option<String> {
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
        .any(|text| text.contains(marker));
    Some(terminal && text_contains)
}

fn tail_runtime(path: &Path) -> CliRuntimeState {
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
}
