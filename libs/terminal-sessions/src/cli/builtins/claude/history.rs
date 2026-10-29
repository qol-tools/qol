use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use qol_agent_homes::{Harness, Registry};
use serde_json::Value;

use crate::cli::chat::text_blocks;
use crate::cli::{CliConversation, CliConversationMatch, CliResumeSettings, CliToolId};

use super::environment::{custom_config_dir, open_session_ids};
use super::metadata::{
    contains_bytes, file_signature, is_local_command, scan_backwards, FileSignature,
    TranscriptFacts,
};
use super::search::search_transcripts;

const HEAD_BUDGET: u64 = 256 * 1024;
const TAIL_BUDGET: u64 = 256 * 1024;
const TAIL_CHUNK: u64 = 16 * 1024;
const PROMPT_CHARS: usize = 2048;
const RECENT_PROMPTS: usize = 4;
const ENTRYPOINT_KEY: &[u8] = b"\"entrypoint\"";
const USER_TAG: &[u8] = b"\"type\":\"user\"";
const AI_TITLE_TAG: &[u8] = b"\"type\":\"ai-title\"";
const PR_LINK_TAG: &[u8] = b"\"type\":\"pr-link\"";
const HEADLESS_ENTRYPOINT_PREFIX: &str = "sdk";

#[derive(Clone)]
pub(super) struct HistoryRoot {
    pub(super) home: PathBuf,
    pub(super) config_dir: Option<PathBuf>,
}

struct Cached {
    signature: FileSignature,
    conversation: Option<CliConversation>,
}

pub(super) struct ClaudeHistory {
    roots: Option<Vec<HistoryRoot>>,
    cache: Mutex<HashMap<PathBuf, Cached>>,
}

impl ClaudeHistory {
    pub(super) fn system() -> Self {
        Self {
            roots: None,
            cache: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(super) fn with_roots(roots: Vec<HistoryRoot>) -> Self {
        Self {
            roots: Some(roots),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn roots(&self) -> Vec<HistoryRoot> {
        if let Some(roots) = &self.roots {
            return roots.clone();
        }
        Registry::load()
            .homes()
            .to_vec()
            .into_iter()
            .filter(|home| home.harness == Harness::Claude)
            .map(|home| HistoryRoot {
                config_dir: custom_config_dir(&home.path),
                home: home.path,
            })
            .collect()
    }

    pub(super) fn closed_conversations(
        &self,
        tool: &CliToolId,
        since: SystemTime,
    ) -> Vec<CliConversation> {
        let Ok(mut cache) = self.cache.lock() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for root in self.roots() {
            let open = open_session_ids(&root.home);
            let projects = Harness::Claude.transcripts_dir(&root.home);
            for path in projects.as_deref().map(transcripts).unwrap_or_default() {
                let Some(signature) = file_signature(&path) else {
                    continue;
                };
                if signature.modified.is_none_or(|modified| modified < since) {
                    continue;
                }
                let conversation = match cache.get(&path) {
                    Some(cached) if cached.signature == signature => cached.conversation.clone(),
                    _ => {
                        let conversation = read_conversation(&path, tool, &root, signature);
                        cache.insert(
                            path.clone(),
                            Cached {
                                signature,
                                conversation: conversation.clone(),
                            },
                        );
                        conversation
                    }
                };
                seen.insert(path);
                found.extend(
                    conversation.filter(|conversation| !open.contains(&conversation.external_id)),
                );
            }
        }
        cache.retain(|path, _| seen.contains(path));
        found.sort_by_key(|conversation| std::cmp::Reverse(conversation.last_active));
        let mut ids = HashSet::new();
        found.retain(|conversation| ids.insert(conversation.external_id.clone()));
        found
    }

    pub(super) fn search_conversations(
        &self,
        since: SystemTime,
        external_ids: &HashSet<String>,
        terms: &[String],
    ) -> Vec<CliConversationMatch> {
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        for root in self.roots() {
            let open = open_session_ids(&root.home);
            let projects = Harness::Claude.transcripts_dir(&root.home);
            for path in projects.as_deref().map(transcripts).unwrap_or_default() {
                let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                if !external_ids.contains(id) || open.contains(id) || seen.contains(id) {
                    continue;
                }
                let recent = file_signature(&path)
                    .and_then(|signature| signature.modified)
                    .is_some_and(|modified| modified >= since);
                if recent {
                    seen.insert(id.to_owned());
                    paths.push(path);
                }
            }
        }
        search_transcripts(paths, terms)
    }
}

fn transcripts(projects: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(projects) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .filter_map(|project| fs::read_dir(project).ok())
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect()
}

fn read_conversation(
    path: &Path,
    tool: &CliToolId,
    root: &HistoryRoot,
    signature: FileSignature,
) -> Option<CliConversation> {
    let external_id = path.file_stem()?.to_str()?.to_owned();
    let head = read_head(path)?;
    if head
        .entrypoint
        .as_deref()
        .is_none_or(|entrypoint| entrypoint.starts_with(HEADLESS_ENTRYPOINT_PREFIX))
    {
        return None;
    }
    let cwd = head.cwd?;
    let tail = read_tail(path, signature.length);
    let last_active = signature
        .modified?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(CliConversation {
        tool: tool.clone(),
        external_id,
        cwd,
        title: tail.facts.title.clone().or(tail.ai_title),
        first_prompt: head.first_prompt,
        recent_prompts: tail.recent_prompts,
        last_active,
        pr_urls: tail.pr_urls,
        settings: CliResumeSettings {
            model: tail.facts.model(),
            effort: tail.facts.request.and_then(|request| request.effort),
            permission_mode: tail.facts.permission_mode,
            claude_config_dir: root
                .config_dir
                .as_deref()
                .and_then(Path::to_str)
                .map(str::to_owned),
        },
    })
}

#[derive(Default)]
struct Head {
    entrypoint: Option<String>,
    cwd: Option<String>,
    first_prompt: Option<String>,
}

fn read_head(path: &Path) -> Option<Head> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file.take(HEAD_BUDGET));
    let mut head = Head::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            break;
        }
        let entry = head.entrypoint.is_none() && contains_bytes(&line, ENTRYPOINT_KEY);
        let user = contains_bytes(&line, USER_TAG);
        if !(entry || user) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if entry {
            head.entrypoint = string_field(&value, "entrypoint");
        }
        if head.cwd.is_none() {
            head.cwd = string_field(&value, "cwd").filter(|cwd| !cwd.is_empty());
        }
        if let Some(prompt) = user.then(|| user_prompt(&value)).flatten() {
            head.first_prompt = Some(prompt);
            break;
        }
    }
    Some(head)
}

fn user_prompt(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("user")
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
        || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || is_local_command(value)
    {
        return None;
    }
    let text = text_blocks(value.get("message")?.get("content")?);
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(PROMPT_CHARS).collect())
}

#[derive(Default)]
struct Tail {
    facts: TranscriptFacts,
    ai_title: Option<String>,
    recent_prompts: Vec<String>,
    pr_urls: Vec<String>,
}

impl Tail {
    fn absorb(&mut self, line: &[u8], newer: bool) {
        self.facts.absorb(line, newer);
        if (newer || self.ai_title.is_none()) && contains_bytes(line, AI_TITLE_TAG) {
            if let Some(title) = parsed_field(line, "aiTitle")
                .map(|title| title.trim().to_owned())
                .filter(|title| !title.is_empty())
            {
                self.ai_title = Some(title);
            }
        }
        if (newer || self.recent_prompts.len() < RECENT_PROMPTS) && contains_bytes(line, USER_TAG) {
            if let Some(prompt) = serde_json::from_slice::<Value>(line)
                .ok()
                .and_then(|value| user_prompt(&value))
            {
                if newer {
                    self.recent_prompts.insert(0, prompt);
                    self.recent_prompts.truncate(RECENT_PROMPTS);
                } else {
                    self.recent_prompts.push(prompt);
                }
            }
        }
        if contains_bytes(line, PR_LINK_TAG) {
            if let Some(url) = parsed_field(line, "prUrl") {
                if !self.pr_urls.contains(&url) {
                    self.pr_urls.push(url);
                }
            }
        }
    }

    fn complete(&self) -> bool {
        self.facts.title.is_some()
            && self.facts.permission_mode.is_some()
            && self.facts.request.is_some()
    }

    fn fill_from(&mut self, older: Tail) {
        self.facts = std::mem::take(&mut self.facts).or(older.facts);
        self.ai_title = self.ai_title.take().or(older.ai_title);
        for prompt in older.recent_prompts {
            if self.recent_prompts.len() < RECENT_PROMPTS && !self.recent_prompts.contains(&prompt)
            {
                self.recent_prompts.push(prompt);
            }
        }
        for url in older.pr_urls.into_iter().rev() {
            if !self.pr_urls.contains(&url) {
                self.pr_urls.push(url);
            }
        }
    }
}

fn read_tail(path: &Path, length: u64) -> Tail {
    let mut tail = Tail::default();
    scan_backwards(path, TAIL_CHUNK, TAIL_BUDGET, |line| {
        tail.absorb(line, false);
        false
    });
    if length > TAIL_BUDGET && !tail.complete() {
        tail.fill_from(read_start(path));
    }
    tail
}

fn read_start(path: &Path) -> Tail {
    let mut start = Tail::default();
    let Ok(file) = fs::File::open(path) else {
        return start;
    };
    let mut reader = BufReader::new(file.take(HEAD_BUDGET));
    let mut line = Vec::new();
    while reader
        .read_until(b'\n', &mut line)
        .is_ok_and(|read| read > 0)
    {
        start.absorb(&line, true);
        line.clear();
    }
    start
}

fn parsed_field(line: &[u8], name: &str) -> Option<String> {
    string_field(&serde_json::from_slice::<Value>(line).ok()?, name)
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value.get(name).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const ENTRY: &str = r#"{"type":"attachment","entrypoint":"cli","cwd":"/work/proj","attachment":{"type":"hook_success"}}"#;

    fn tool() -> CliToolId {
        CliToolId::new("claude").unwrap()
    }

    fn history(home: &Path, config_dir: Option<&Path>) -> ClaudeHistory {
        ClaudeHistory::with_roots(vec![HistoryRoot {
            home: home.to_path_buf(),
            config_dir: config_dir.map(Path::to_path_buf),
        }])
    }

    fn write_transcript(home: &Path, id: &str, lines: &[&str]) -> PathBuf {
        let project = home.join("projects").join("-work-proj");
        fs::create_dir_all(&project).unwrap();
        let path = project.join(format!("{id}.jsonl"));
        fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        path
    }

    fn prompt(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "cwd": "/work/proj",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        })
        .to_string()
    }

    fn ids(conversations: &[CliConversation]) -> Vec<&str> {
        conversations
            .iter()
            .map(|conversation| conversation.external_id.as_str())
            .collect()
    }

    #[test]
    fn an_interactive_transcript_becomes_a_closed_conversation_with_its_resume_settings() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("work-account");
        let first = prompt("fix the login bug\nwith details");
        let path = write_transcript(
            home.path(),
            "abc",
            &[
                r#"{"type":"permission-mode","permissionMode":"plan"}"#,
                ENTRY,
                r#"{"type":"user","isMeta":true,"cwd":"/work/proj","message":{"role":"user","content":"<local-command-caveat>ignored"}}"#,
                &first,
                r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/6"}"#,
                &prompt("a later prompt"),
                r#"{"type":"api-request","querySource":"repl_main_thread","params":{"model":"claude-opus-5-5","output_config":{"effort":"high"}}}"#,
                r#"{"type":"ai-title","aiTitle":"Older title"}"#,
                r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/7"}"#,
                r#"{"type":"ai-title","aiTitle":"Login bug fix"}"#,
                r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/8"}"#,
                r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/7"}"#,
                r#"{"type":"permission-mode","permissionMode":"bypassPermissions"}"#,
            ],
        );
        let modified = fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let found = history(home.path(), Some(&config)).closed_conversations(&tool(), UNIX_EPOCH);

        assert_eq!(
            found,
            [CliConversation {
                tool: tool(),
                external_id: "abc".to_owned(),
                cwd: "/work/proj".to_owned(),
                title: Some("Login bug fix".to_owned()),
                first_prompt: Some("fix the login bug\nwith details".to_owned()),
                recent_prompts: vec![
                    "a later prompt".to_owned(),
                    "fix the login bug\nwith details".to_owned(),
                ],
                last_active: modified,
                pr_urls: vec![
                    "https://github.com/o/r/pull/7".to_owned(),
                    "https://github.com/o/r/pull/8".to_owned(),
                    "https://github.com/o/r/pull/6".to_owned(),
                ],
                settings: CliResumeSettings {
                    permission_mode: Some("bypassPermissions".to_owned()),
                    model: Some("claude-opus-5-5".to_owned()),
                    effort: Some("high".to_owned()),
                    claude_config_dir: config.to_str().map(str::to_owned),
                },
            }]
        );
    }

    #[test]
    fn headless_unlabelled_old_and_open_transcripts_are_left_out() {
        let home = tempfile::tempdir().unwrap();
        let first = prompt("hello");
        write_transcript(home.path(), "kept", &[ENTRY, &first]);
        write_transcript(
            home.path(),
            "headless",
            &[
                r#"{"type":"attachment","entrypoint":"sdk-py","cwd":"/work/proj"}"#,
                &first,
            ],
        );
        write_transcript(home.path(), "unlabelled", &[&first]);
        let old = write_transcript(home.path(), "old", &[ENTRY, &first]);
        let since = SystemTime::now() - Duration::from_secs(3_600);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(since - Duration::from_secs(60))
            .unwrap();
        write_transcript(home.path(), "open", &[ENTRY, &first]);
        let sessions = home.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join(format!("{}.json", std::process::id())),
            r#"{"sessionId":"open","cwd":"/work/proj"}"#,
        )
        .unwrap();
        write_transcript(home.path(), "zero", &[ENTRY, &first]);
        fs::write(
            sessions.join("0.json"),
            r#"{"sessionId":"zero","cwd":"/work/proj"}"#,
        )
        .unwrap();

        let found = history(home.path(), None).closed_conversations(&tool(), since);

        let mut listed = ids(&found);
        listed.sort_unstable();
        assert_eq!(listed, ["kept", "zero"]);
        let found = [found
            .into_iter()
            .find(|conversation| conversation.external_id == "kept")
            .unwrap()];
        assert_eq!(found[0].settings.claude_config_dir, None);
        assert_eq!(found[0].title, None);
        assert!(found[0].pr_urls.is_empty());
    }

    #[test]
    fn a_session_record_holds_its_conversation_only_while_its_own_process_runs() {
        let home = tempfile::tempdir().unwrap();
        let first = prompt("hello");
        write_transcript(home.path(), "reused", &[ENTRY, &first]);
        let sessions = home.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let record = sessions.join(format!("{}.json", std::process::id()));
        let write_record = |process_start: &str| {
            fs::write(
                &record,
                serde_json::json!({ "sessionId": "reused", "cwd": "/work/proj", "procStart": process_start })
                    .to_string(),
            )
            .unwrap();
        };

        write_record("not this process");
        let found = history(home.path(), None).closed_conversations(&tool(), UNIX_EPOCH);
        assert_eq!(ids(&found), ["reused"]);

        if let Some(started) = qol_process::process_start_time(std::process::id()) {
            write_record(&started.to_string());
            let found = history(home.path(), None).closed_conversations(&tool(), UNIX_EPOCH);
            assert!(found.is_empty(), "{found:?}");
        }
    }

    #[test]
    fn a_long_transcript_is_read_from_the_metadata_claude_appends_at_its_end() {
        let home = tempfile::tempdir().unwrap();
        let first = prompt("hello");
        let filler = prompt(&"x".repeat(4_096));
        let mut lines = vec![
            r#"{"type":"permission-mode","permissionMode":"default"}"#.to_owned(),
            ENTRY.to_owned(),
            first,
            r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/3"}"#.to_owned(),
        ];
        lines.extend(std::iter::repeat_n(filler, 80));
        lines.extend([
            r#"{"type":"api-request","querySource":"repl_main_thread","params":{"model":"claude-opus-5-5","output_config":{"effort":"low"}}}"#.to_owned(),
            r#"{"type":"custom-title","customTitle":"Named early"}"#.to_owned(),
            r#"{"type":"permission-mode","permissionMode":"plan"}"#.to_owned(),
            r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/4"}"#.to_owned(),
        ]);
        let lines = lines.iter().map(String::as_str).collect::<Vec<_>>();
        write_transcript(home.path(), "long", &lines);

        let found = history(home.path(), None).closed_conversations(&tool(), UNIX_EPOCH);

        assert_eq!(found[0].title.as_deref(), Some("Named early"));
        assert_eq!(found[0].settings.permission_mode.as_deref(), Some("plan"));
        assert_eq!(found[0].settings.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(found[0].settings.effort.as_deref(), Some("low"));
        assert_eq!(found[0].pr_urls, ["https://github.com/o/r/pull/4"]);
    }

    #[test]
    fn settings_written_only_at_the_start_of_a_long_transcript_are_read() {
        let home = tempfile::tempdir().unwrap();
        let first = prompt("hello");
        let filler = prompt(&"x".repeat(4_096));
        let mut lines = vec![
            r#"{"type":"permission-mode","permissionMode":"default"}"#.to_owned(),
            ENTRY.to_owned(),
            first,
            r#"{"type":"permission-mode","permissionMode":"acceptEdits"}"#.to_owned(),
            r#"{"type":"custom-title","customTitle":"Early name"}"#.to_owned(),
            r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/3"}"#.to_owned(),
            r#"{"type":"api-request","querySource":"repl_main_thread","params":{"model":"claude-opus-5-5","output_config":{"effort":"low"}}}"#.to_owned(),
        ];
        lines.push(r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/2"}"#.to_owned());
        lines.extend(std::iter::repeat_n(filler, 80));
        lines.extend([
            r#"{"type":"ai-title","aiTitle":"Generated"}"#.to_owned(),
            r#"{"type":"pr-link","prUrl":"https://github.com/o/r/pull/4"}"#.to_owned(),
        ]);
        let lines = lines.iter().map(String::as_str).collect::<Vec<_>>();
        write_transcript(home.path(), "long", &lines);

        let found = history(home.path(), None).closed_conversations(&tool(), UNIX_EPOCH);

        assert_eq!(found[0].title.as_deref(), Some("Early name"));
        assert_eq!(
            found[0].settings.permission_mode.as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(found[0].settings.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(found[0].settings.effort.as_deref(), Some("low"));
        assert_eq!(
            found[0].pr_urls,
            [
                "https://github.com/o/r/pull/4",
                "https://github.com/o/r/pull/2",
                "https://github.com/o/r/pull/3"
            ]
        );
    }

    #[test]
    fn a_conversation_in_two_claude_homes_is_listed_once_from_the_newer_copy() {
        let older = tempfile::tempdir().unwrap();
        let newer = tempfile::tempdir().unwrap();
        let first = prompt("hello");
        let stale = write_transcript(older.path(), "dup", &[ENTRY, &first]);
        write_transcript(newer.path(), "dup", &[ENTRY, &first]);
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(600))
            .unwrap();
        let account = newer.path().join("account");
        let history = ClaudeHistory::with_roots(vec![
            HistoryRoot {
                home: older.path().to_path_buf(),
                config_dir: None,
            },
            HistoryRoot {
                home: newer.path().to_path_buf(),
                config_dir: Some(account.clone()),
            },
        ]);

        let found = history.closed_conversations(&tool(), UNIX_EPOCH);

        assert_eq!(ids(&found), ["dup"]);
        assert_eq!(
            found[0].settings.claude_config_dir.as_deref(),
            account.to_str()
        );
    }

    #[test]
    fn search_skips_ids_outside_the_candidates_and_transcripts_older_than_since() {
        let home = tempfile::tempdir().unwrap();
        let first = prompt("find the refund");
        write_transcript(home.path(), "wanted", &[ENTRY, &first]);
        write_transcript(home.path(), "unwanted", &[ENTRY, &first]);
        let old = write_transcript(home.path(), "old", &[ENTRY, &first]);
        let since = SystemTime::now() - Duration::from_secs(3_600);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(since - Duration::from_secs(60))
            .unwrap();
        let candidates = ["wanted", "old"]
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>();

        let found = history(home.path(), None).search_conversations(
            since,
            &candidates,
            &["refund".to_owned()],
        );

        let listed = found
            .iter()
            .map(|found| found.external_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(listed, ["wanted"]);
    }

    #[test]
    fn a_grown_transcript_is_read_again_and_a_custom_title_wins() {
        let home = tempfile::tempdir().unwrap();
        let first = prompt("hello");
        let path = write_transcript(
            home.path(),
            "abc",
            &[
                ENTRY,
                &first,
                r#"{"type":"ai-title","aiTitle":"Generated"}"#,
            ],
        );
        let history = history(home.path(), None);
        let found = history.closed_conversations(&tool(), UNIX_EPOCH);
        assert_eq!(found[0].title.as_deref(), Some("Generated"));

        let mut grown = fs::read_to_string(&path).unwrap();
        grown.push_str("{\"type\":\"custom-title\",\"customTitle\":\"Named by me\"}\n");
        fs::write(&path, grown).unwrap();
        let found = history.closed_conversations(&tool(), UNIX_EPOCH);
        assert_eq!(found[0].title.as_deref(), Some("Named by me"));

        fs::remove_file(&path).unwrap();
        assert!(history.closed_conversations(&tool(), UNIX_EPOCH).is_empty());
    }
}
