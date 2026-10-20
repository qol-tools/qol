use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use qol_agent_homes::{Harness, Registry};

use crate::cli::chat::{read_chat, text_blocks, ChatRole, ChatTurn, COMPACTED_SUMMARY_HEADER};
use crate::cli::launch_flags::EFFORT_LEVELS;
use crate::cli::{tail, CliActivityEvidence, CliResumeSettings, CliRuntimeState};
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
    runtime_tail: tail::RuntimeTail,
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
pub(super) struct FileSignature {
    pub(super) modified: Option<SystemTime>,
    pub(super) length: u64,
}

#[derive(Clone)]
struct CachedFacts {
    signature: FileSignature,
    scanned_length: u64,
    transcript: TranscriptFacts,
}

#[derive(Clone, Default)]
pub(super) struct TranscriptFacts {
    pub(super) title: Option<String>,
    has_message: bool,
    pub(super) permission_mode: Option<String>,
    pub(super) request: Option<RequestTier>,
    assistant_model: Option<String>,
}

#[derive(Clone)]
pub(super) struct RequestTier {
    model: String,
    pub(super) effort: Option<String>,
}

impl ClaudeMetadataResolver {
    pub fn new(environment: Arc<dyn ClaudeEnvironment>) -> Self {
        Self {
            environment,
            projects_root: None,
            cache: Mutex::new(ClaudeCache::default()),
            runtime_tail: tail::RuntimeTail::default(),
        }
    }

    fn located_facts(
        &self,
        session: &SessionFacts,
    ) -> (Option<ClaudeSessionLocation>, Option<CachedFacts>) {
        let mut cache = self.cache.lock().ok();
        let location = cache
            .as_mut()
            .and_then(|cache| session_location(session, self.environment.as_ref(), cache));
        let facts = cache.as_mut().and_then(|cache| {
            location
                .as_ref()
                .and_then(|location| cached_facts(&location.transcript_path, cache))
        });
        (location, facts)
    }

    pub fn resolve(&self, session: &SessionFacts) -> ClaudeMetadata {
        let (location, facts) = self.located_facts(session);
        let activity = facts
            .as_ref()
            .map(|facts| CliActivityEvidence {
                file_fresh: recently_active(facts.signature.modified),
                file_has_work: Some(facts.transcript.has_message),
                file_quiet_secs: quiet_secs(facts.signature.modified),
            })
            .unwrap_or_default();
        let runtime = location
            .as_ref()
            .map(|location| {
                self.runtime_tail
                    .latest(&location.transcript_path, tail_runtime)
            })
            .unwrap_or(CliRuntimeState::Unknown);
        ClaudeMetadata {
            custom_title: facts.and_then(|facts| facts.transcript.title),
            external_id: location.map(|location| location.external_id),
            has_activity: activity.combined(),
            runtime,
            activity,
        }
    }

    pub fn resume_settings(&self, session: &SessionFacts) -> CliResumeSettings {
        let (location, facts) = self.located_facts(session);
        let transcript = facts.map(|facts| facts.transcript).unwrap_or_default();
        CliResumeSettings {
            model: transcript.model(),
            effort: transcript.request.and_then(|request| request.effort),
            permission_mode: transcript.permission_mode,
            claude_config_dir: location
                .and_then(|location| location.config_dir)
                .and_then(|directory| directory.to_str().map(str::to_owned)),
        }
    }

    pub fn subscription_path(&self, session: &SessionFacts) -> Option<PathBuf> {
        let mut cache = self.cache.lock().ok()?;
        session_location(session, self.environment.as_ref(), &mut cache)
            .map(|location| location.transcript_path)
    }

    pub fn remote_controlled(&self, session: &SessionFacts) -> bool {
        session
            .foreground_pids
            .iter()
            .any(|pid| self.environment.remote_controlled(*pid))
    }

    pub fn busy(&self, session: &SessionFacts) -> bool {
        session
            .foreground_pids
            .iter()
            .any(|pid| self.environment.busy(*pid))
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
            return Some(entry.clone());
        }
        if signature.length > entry.signature.length
            && entry.scanned_length == entry.signature.length
        {
            let mut transcript = entry.transcript.clone();
            absorb_appended(path, entry.scanned_length, &mut transcript);
            let facts = CachedFacts {
                signature,
                scanned_length: tail::complete_length(path).unwrap_or(0),
                transcript,
            };
            cache.facts.insert(path.to_path_buf(), facts.clone());
            return Some(facts);
        }
    }
    let mut transcript = TranscriptFacts::default();
    absorb_backwards(path, &mut transcript);
    let facts = CachedFacts {
        signature,
        scanned_length: tail::complete_length(path).unwrap_or(0),
        transcript,
    };
    cache.facts.insert(path.to_path_buf(), facts.clone());
    Some(facts)
}

impl TranscriptFacts {
    pub(super) fn or(self, older: Self) -> Self {
        let tiered = self.request.is_some() || self.assistant_model.is_some();
        Self {
            title: self.title.or(older.title),
            has_message: self.has_message || older.has_message,
            permission_mode: self.permission_mode.or(older.permission_mode),
            request: if tiered { self.request } else { older.request },
            assistant_model: if tiered {
                self.assistant_model
            } else {
                older.assistant_model
            },
        }
    }

    pub(super) fn model(&self) -> Option<String> {
        self.request
            .as_ref()
            .map(|request| request.model.clone())
            .or_else(|| self.assistant_model.clone())
    }

    fn complete(&self) -> bool {
        self.title.is_some()
            && self.has_message
            && self.permission_mode.is_some()
            && self.request.is_some()
    }

    pub(super) fn absorb(&mut self, line: &[u8], newer: bool) {
        self.has_message = self.has_message || is_message(line);
        let title = (newer || self.title.is_none()) && contains_bytes(line, CUSTOM_TITLE_TAG);
        let mode =
            (newer || self.permission_mode.is_none()) && contains_bytes(line, PERMISSION_MODE_KEY);
        let request = (newer || self.request.is_none()) && contains_bytes(line, API_REQUEST_TAG);
        let assistant = self.request.is_none()
            && (newer || self.assistant_model.is_none())
            && contains_bytes(line, ASSISTANT_TAG);
        if !(title || mode || request || assistant) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if let Some(found) = title.then(|| custom_title(&value)).flatten() {
            self.title = Some(found);
        }
        if let Some(found) = mode.then(|| permission_mode(&value)).flatten() {
            self.permission_mode = Some(found);
        }
        if let Some(found) = request.then(|| request_tier(&value)).flatten() {
            self.request = Some(found);
        }
        if let Some(found) = assistant.then(|| main_thread_model(&value)).flatten() {
            self.assistant_model = Some(found);
        }
    }
}

const CUSTOM_TITLE_TAG: &[u8] = b"\"type\":\"custom-title\"";
const PERMISSION_MODE_KEY: &[u8] = b"\"permissionMode\"";
const API_REQUEST_TAG: &[u8] = b"\"type\":\"api-request\"";
const ASSISTANT_TAG: &[u8] = b"\"type\":\"assistant\"";
const MAIN_THREAD_QUERY_SOURCES: [&str; 2] = ["repl_main_thread", "sdk"];

fn absorb_appended(path: &Path, offset: u64, transcript: &mut TranscriptFacts) -> Option<()> {
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut appended = Vec::new();
    file.read_to_end(&mut appended).ok()?;
    for line in appended.split(|byte| *byte == b'\n') {
        transcript.absorb(line, true);
    }
    Some(())
}

fn absorb_backwards(path: &Path, transcript: &mut TranscriptFacts) -> Option<()> {
    scan_backwards(path, REVERSE_READ_CHUNK, u64::MAX, |line| {
        transcript.absorb(line, false);
        transcript.complete()
    })
}

pub(super) fn scan_backwards(
    path: &Path,
    chunk_size: u64,
    budget: u64,
    mut visit: impl FnMut(&[u8]) -> bool,
) -> Option<()> {
    let mut file = fs::File::open(path).ok()?;
    let mut cursor = file.metadata().ok()?.len();
    let floor = cursor.saturating_sub(budget);
    let mut suffix = Vec::new();
    while cursor > floor {
        let start = cursor.saturating_sub(chunk_size).max(floor);
        let length = usize::try_from(cursor - start).ok()?;
        let mut chunk = vec![0; length];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&suffix);
        let lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let complete_from = usize::from(start > 0);
        for line in lines[complete_from..].iter().rev() {
            if visit(line) {
                return Some(());
            }
        }
        suffix = lines.first().copied().unwrap_or_default().to_vec();
        cursor = start;
    }
    Some(())
}

fn is_message(line: &[u8]) -> bool {
    contains_bytes(line, b"\"type\":\"user\"") || contains_bytes(line, b"\"type\":\"assistant\"")
}

pub(super) fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub(super) fn file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileSignature {
        modified: metadata.modified().ok(),
        length: metadata.len(),
    })
}

fn tail_runtime(path: &Path) -> CliRuntimeState {
    tail::latest_runtime(path, runtime_entry)
}

fn runtime_entry(value: &Value) -> Option<CliRuntimeState> {
    match value.get("type").and_then(Value::as_str) {
        Some("system") if value.get("subtype").and_then(Value::as_str) == Some("turn_duration") => {
            Some(CliRuntimeState::Ready)
        }
        Some("user") if is_interrupt_note(value) => Some(CliRuntimeState::Ready),
        Some("assistant") => {
            let stop = value
                .get("message")
                .and_then(|message| message.get("stop_reason"))
                .and_then(Value::as_str);
            Some(match stop {
                Some("end_turn" | "stop_sequence" | "max_tokens") => CliRuntimeState::Ready,
                _ => CliRuntimeState::Working,
            })
        }
        Some("user") if is_local_command(value) => None,
        Some("attachment") if is_hook_output(value) => None,
        Some("user" | "attachment") => Some(CliRuntimeState::Working),
        _ => None,
    }
}

const LOCAL_COMMAND_TAGS: [&str; 3] = [
    "<local-command-caveat>",
    "<command-name>",
    "<local-command-stdout>",
];

pub(super) fn is_local_command(value: &Value) -> bool {
    let content = value
        .get("message")
        .and_then(|message| message.get("content"));
    let text = match content {
        Some(Value::String(text)) => Some(text.as_str()),
        Some(Value::Array(blocks)) => blocks
            .first()
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str),
        _ => None,
    };
    text.is_some_and(|text| {
        let text = text.trim_start();
        LOCAL_COMMAND_TAGS.iter().any(|tag| text.starts_with(tag))
    })
}

fn is_hook_output(value: &Value) -> bool {
    value
        .get("attachment")
        .and_then(|attachment| attachment.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.starts_with("hook_"))
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

fn custom_title(value: &Value) -> Option<String> {
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

fn permission_mode(value: &Value) -> Option<String> {
    value
        .get("permissionMode")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn request_tier(value: &Value) -> Option<RequestTier> {
    if value.get("type").and_then(Value::as_str) != Some("api-request")
        || !value
            .get("querySource")
            .and_then(Value::as_str)
            .is_some_and(|source| MAIN_THREAD_QUERY_SOURCES.contains(&source))
    {
        return None;
    }
    let model = value
        .pointer("/params/model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())?;
    Some(RequestTier {
        model: model.to_owned(),
        effort: value
            .pointer("/params/output_config/effort")
            .and_then(Value::as_str)
            .filter(|effort| EFFORT_LEVELS.contains(effort))
            .map(str::to_owned),
    })
}

pub(super) fn chat_transcript(path: &Path) -> Option<Vec<ChatTurn>> {
    read_chat(path, append_chat_turn)
}

pub(super) fn current_model(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    contents
        .lines()
        .rev()
        .find_map(|line| main_thread_model(&serde_json::from_str::<Value>(line).ok()?))
}

fn main_thread_model(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("assistant")
        || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    value
        .pointer("/message/model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty() && !model.starts_with('<'))
        .map(str::to_owned)
}

fn append_chat_turn(turns: &mut Vec<ChatTurn>, value: &Value) {
    if value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
        || value.get("promptSource").and_then(Value::as_str) == Some("system")
    {
        return;
    }
    let role = match value.get("type").and_then(Value::as_str) {
        Some("user") => ChatRole::User,
        Some("assistant") => ChatRole::Assistant,
        Some("attachment") => {
            if let Some(text) = queued_prompt(value) {
                turns.push(ChatTurn {
                    role: ChatRole::User,
                    text: text.to_owned(),
                });
            }
            return;
        }
        _ => return,
    };
    let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
    else {
        return;
    };
    let compact_summary = role == ChatRole::User
        && value.get("isCompactSummary").and_then(Value::as_bool) == Some(true);
    let text = match content {
        Value::String(text) if role == ChatRole::User => {
            if is_harness_prompt(text) {
                return;
            }
            if compact_summary {
                format!("{COMPACTED_SUMMARY_HEADER}\n{text}")
            } else {
                text.clone()
            }
        }
        content => text_blocks(content),
    };
    turns.push(ChatTurn { role, text });
}

fn queued_prompt(value: &Value) -> Option<&str> {
    let attachment = value.get("attachment")?;
    if attachment.get("type").and_then(Value::as_str) != Some("queued_command")
        || attachment.get("commandMode").and_then(Value::as_str) != Some("prompt")
    {
        return None;
    }
    attachment
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|text| !is_harness_prompt(text))
}

fn is_harness_prompt(text: &str) -> bool {
    let text = text.trim_start();
    [
        "<command-name",
        "<command-message",
        "<local-command-stdout",
        "<local-command-caveat",
        "<system-reminder",
        "<task-notification",
        "<bash-input",
        "<bash-stdout",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

#[cfg(test)]
mod runtime_regressions {
    use super::tail_runtime;
    use crate::cli::CliRuntimeState;

    #[test]
    fn metadata_preserves_the_latest_lifecycle_transition() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("session.jsonl");
        let working = r#"{"type":"user"}"#;
        let ready = r#"{"type":"assistant","message":{"stop_reason":"end_turn"}}"#;
        let metadata = [
            r#"{"type":"cost-state"}"#,
            r#"{"type":"system","subtype":"other"}"#,
            r#"{"type":"custom-title"}"#,
        ];
        for entry in metadata {
            for (events, expected) in [
                (format!("{working}\n"), CliRuntimeState::Working),
                (format!("{working}\n{ready}\n"), CliRuntimeState::Ready),
                (format!("{ready}\n{working}\n"), CliRuntimeState::Working),
                (String::new(), CliRuntimeState::Unknown),
            ] {
                std::fs::write(&path, format!("{events}{entry}\n\n{{partial")).unwrap();
                assert_eq!(tail_runtime(&path), expected, "{events}{entry}");
            }
        }
    }

    #[test]
    fn missing_empty_partial_and_invalid_transcripts_do_not_prove_readiness() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("session.jsonl");
        assert_eq!(tail_runtime(&path), CliRuntimeState::Unknown);
        for text in ["", "{partial", "invalid\n", "\n\n"] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(tail_runtime(&path), CliRuntimeState::Unknown, "{text}");
        }
    }

    #[test]
    fn hook_output_and_local_commands_leave_the_turn_state_as_it_was() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("session.jsonl");
        let ready = r#"{"type":"system","subtype":"turn_duration"}"#;
        let working = r#"{"type":"assistant","message":{"stop_reason":"tool_use"}}"#;
        let session_start_hooks = [
            r#"{"type":"attachment","attachment":{"type":"hook_success"}}"#,
            r#"{"type":"attachment","attachment":{"type":"hook_system_message"}}"#,
            r#"{"type":"attachment","attachment":{"type":"hook_additional_context"}}"#,
        ]
        .join("\n");
        let local_command = [
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>Caveat: run directly</local-command-caveat>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-name>/reload-plugins</command-name>\n<command-message>reload-plugins</command-message>"}}"#,
            r#"{"type":"system","subtype":"local_command","content":"<local-command-stdout>Reloaded</local-command-stdout>"}"#,
        ]
        .join("\n");
        let older_local_command = [
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<command-name>/model</command-name>"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model</local-command-stdout>"}}"#,
        ]
        .join("\n");
        for (events, expected) in [
            (session_start_hooks.clone(), CliRuntimeState::Unknown),
            (
                format!("{session_start_hooks}\n{local_command}"),
                CliRuntimeState::Unknown,
            ),
            (format!("{ready}\n{local_command}"), CliRuntimeState::Ready),
            (
                format!("{ready}\n{older_local_command}"),
                CliRuntimeState::Ready,
            ),
            (
                format!("{ready}\n{session_start_hooks}"),
                CliRuntimeState::Ready,
            ),
            (
                format!("{working}\n{local_command}"),
                CliRuntimeState::Working,
            ),
            (
                format!("{working}\n{session_start_hooks}"),
                CliRuntimeState::Working,
            ),
        ] {
            std::fs::write(&path, format!("{events}\n{{\"type\":\"cost-state\"}}\n")).unwrap();
            assert_eq!(tail_runtime(&path), expected, "{events}");
        }
    }

    #[test]
    fn prompts_skill_commands_and_other_attachments_still_start_a_turn() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("session.jsonl");
        let ready = r#"{"type":"system","subtype":"turn_duration"}"#;
        for entry in [
            r#"{"type":"user","message":{"role":"user","content":"fix the bug"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<command-message>qol-memory</command-message>\n<command-name>/qol-memory</command-name>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<task-notification>done</task-notification>"}}"#,
            r#"{"type":"attachment","attachment":{"type":"queued_command"}}"#,
        ] {
            std::fs::write(&path, format!("{ready}\n{entry}\n")).unwrap();
            assert_eq!(tail_runtime(&path), CliRuntimeState::Working, "{entry}");
        }
    }
}
