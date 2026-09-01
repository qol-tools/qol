use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use qol_agent_homes::{Harness, Registry};

use crate::cli::{tail, CliActivityEvidence, CliRuntimeState};
use crate::SessionFacts;

use super::environment::{ClaudeEnvironment, ClaudeSessionLocation};
use crate::cli::activity::{quiet_secs, recently_active};

pub(super) const SESSION_CACHE_TTL: Duration = Duration::from_secs(30);
pub(super) const MISSING_SESSION_CACHE_TTL: Duration = Duration::from_millis(500);
const REVERSE_READ_CHUNK: u64 = 64 * 1024;

pub(super) struct ClaudeMetadata {
    pub custom_title: Option<String>,
    pub external_id: Option<String>,
    pub has_activity: Option<bool>,
    pub runtime: CliRuntimeState,
    pub activity: CliActivityEvidence,
}

pub(super) struct ClaudeMetadataResolver {
    environment: Arc<dyn ClaudeEnvironment>,
    pub(super) projects_root: Option<PathBuf>,
    cache: Mutex<ClaudeCache>,
}

#[derive(Default)]
struct ClaudeCache {
    sessions: HashMap<i32, Timed<Option<ClaudeSessionLocation>>>,
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
    title: Option<String>,
    has_message: bool,
}

impl ClaudeMetadataResolver {
    pub fn new(environment: Arc<dyn ClaudeEnvironment>) -> Self {
        Self {
            environment,
            projects_root: None,
            cache: Mutex::new(ClaudeCache::default()),
        }
    }

    pub fn resolve(&self, session: &SessionFacts) -> ClaudeMetadata {
        let mut cache = self.cache.lock().ok();
        let location = cache
            .as_mut()
            .and_then(|cache| session_location(session, self.environment.as_ref(), cache));
        let facts = cache.as_mut().and_then(|cache| {
            location
                .as_ref()
                .and_then(|location| cached_facts(&location.transcript_path, cache))
        });
        let activity = facts
            .as_ref()
            .map(|facts| CliActivityEvidence {
                file_fresh: recently_active(facts.signature.modified),
                file_has_work: Some(facts.has_message),
                file_quiet_secs: quiet_secs(facts.signature.modified),
            })
            .unwrap_or_default();
        let runtime = location
            .as_ref()
            .map(|location| tail_runtime(&location.transcript_path))
            .unwrap_or(CliRuntimeState::Unknown);
        ClaudeMetadata {
            custom_title: facts.as_ref().and_then(|facts| facts.title.clone()),
            external_id: location.map(|location| location.external_id),
            has_activity: activity.combined(),
            runtime,
            activity,
        }
    }

    pub fn subscription_path(&self, session: &SessionFacts) -> Option<PathBuf> {
        let mut cache = self.cache.lock().ok()?;
        session_location(session, self.environment.as_ref(), &mut cache)
            .map(|location| location.transcript_path)
    }

    pub fn subscription_dir(&self, session: &SessionFacts) -> Option<PathBuf> {
        let root = if let Some(root) = &self.projects_root {
            root.clone()
        } else {
            Registry::load()
                .current(Harness::Claude)
                .path
                .join("projects")
        };
        Some(root.join(project_dir_name(&session.cwd)))
    }
}

fn project_dir_name(cwd: &str) -> String {
    cwd.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn session_location(
    session: &SessionFacts,
    environment: &dyn ClaudeEnvironment,
    cache: &mut ClaudeCache,
) -> Option<ClaudeSessionLocation> {
    session
        .foreground_pids
        .iter()
        .find_map(|pid| cached_session(*pid, environment, cache))
}

fn cached_session(
    pid: i32,
    environment: &dyn ClaudeEnvironment,
    cache: &mut ClaudeCache,
) -> Option<ClaudeSessionLocation> {
    if let Some(entry) = cache.sessions.get(&pid) {
        let fresh_for = if entry.value.is_some() {
            SESSION_CACHE_TTL
        } else {
            MISSING_SESSION_CACHE_TTL
        };
        if entry.checked_at.elapsed() < fresh_for {
            return entry.value.clone();
        }
    }
    let value = environment.session(pid);
    cache.sessions.insert(
        pid,
        Timed {
            value: value.clone(),
            checked_at: Instant::now(),
        },
    );
    value
}

fn cached_facts(path: &Path, cache: &mut ClaudeCache) -> Option<CachedFacts> {
    let signature = file_signature(path)?;
    if let Some(entry) = cache.facts.get(path) {
        if entry.signature == signature {
            return Some(clone_facts(entry));
        }
        if signature.length > entry.signature.length
            && entry.scanned_length == entry.signature.length
        {
            let title = latest_custom_title_since(path, entry.scanned_length)
                .or_else(|| entry.title.clone());
            let has_message = entry.has_message || any_message_since(path, entry.scanned_length);
            let facts = CachedFacts {
                signature,
                scanned_length: tail::last_complete_line(path).map_or(0, |line| line.end),
                title,
                has_message,
            };
            cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
            return Some(facts);
        }
    }
    let facts = CachedFacts {
        signature,
        scanned_length: tail::last_complete_line(path).map_or(0, |line| line.end),
        title: latest_custom_title(path),
        has_message: any_message(path),
    };
    cache.facts.insert(path.to_path_buf(), clone_facts(&facts));
    Some(facts)
}

fn clone_facts(facts: &CachedFacts) -> CachedFacts {
    CachedFacts {
        signature: facts.signature,
        scanned_length: facts.scanned_length,
        title: facts.title.clone(),
        has_message: facts.has_message,
    }
}

fn any_message(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    std::io::BufRead::lines(std::io::BufReader::new(file))
        .map_while(Result::ok)
        .any(|line| is_message(line.as_bytes()))
}

fn any_message_since(path: &Path, offset: u64) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return false;
    }
    let mut appended = Vec::new();
    if file.read_to_end(&mut appended).is_err() {
        return false;
    }
    appended.split(|byte| *byte == b'\n').any(is_message)
}

fn is_message(line: &[u8]) -> bool {
    contains_bytes(line, b"\"type\":\"user\"") || contains_bytes(line, b"\"type\":\"assistant\"")
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

fn latest_custom_title(path: &Path) -> Option<String> {
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
            if let Some(title) = custom_title(line) {
                return Some(title);
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    custom_title(&suffix)
}

fn latest_custom_title_since(path: &Path, offset: u64) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut appended = Vec::new();
    file.read_to_end(&mut appended).ok()?;
    appended
        .split(|byte| *byte == b'\n')
        .filter_map(custom_title)
        .next_back()
}

fn tail_runtime(path: &Path) -> CliRuntimeState {
    let Some(line) = tail::last_complete_line(path) else {
        return CliRuntimeState::Ready;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&line.bytes) else {
        return CliRuntimeState::Working;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("system" | "last-prompt" | "mode" | "permission-mode") => CliRuntimeState::Ready,
        Some("user") if is_interrupt_note(&value) => CliRuntimeState::Ready,
        Some(
            "ai-title"
            | "atis-latch"
            | "cost-state"
            | "artifact-comment-monitor"
            | "worktree-state",
        ) => CliRuntimeState::Ready,
        _ => CliRuntimeState::Working,
    }
}

fn is_interrupt_note(value: &Value) -> bool {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks.iter().any(|block| {
                block.get("type").and_then(Value::as_str) == Some("text")
                    && block
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| text.trim().starts_with("[Request interrupted by user"))
            })
        })
}

fn custom_title(line: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("custom-title") {
        return None;
    }
    value
        .get("customTitle")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
}
