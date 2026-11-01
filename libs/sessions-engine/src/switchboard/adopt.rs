use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::paths::{project_dir_name, switchboard_dir};
use super::resume::{meta_files, AGENT_PREFIX, FORK_AGENT};
use crate::fork::flag_value;

pub(super) const ADOPT: &str = "adopt";
const META_SUFFIX: &str = ".meta.json";
const TRANSCRIPT_SUFFIX: &str = ".jsonl";
const PARTIAL_SUFFIX: &str = ".partial";
const RESUME_BLOCKERS: [&str; 2] = ["stoppedByUser", "finished"];
const AGENT_ID_KEY: &str = "agentId";
const FRESH_ID_HEX: usize = 16;
const USAGE: &str = "usage: switchboard adopt --conversations <dir> --conversation <id> --fork <agent id or key-name> [--fresh]";

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Brought {
    Here,
    Copied,
    Fresh,
}

#[derive(Debug, Serialize)]
pub(super) struct Adopted {
    pub(super) agent_id: String,
    pub(super) name: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) brought: Brought,
}

struct Candidate {
    agent_id: String,
    transcript: PathBuf,
    meta_path: PathBuf,
    meta: serde_json::Map<String, Value>,
    is_current: bool,
    modified: SystemTime,
}

impl Candidate {
    fn name(&self) -> Option<String> {
        self.meta
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    fn key_name(&self) -> Option<String> {
        self.name()
            .map(|name| name.split(':').next().unwrap_or_default().trim().to_owned())
            .filter(|key| !key.is_empty())
    }

    fn adopted(&self, agent_id: String, brought: Brought) -> Adopted {
        Adopted {
            agent_id,
            name: self.name(),
            cwd: self
                .meta
                .get("cwd")
                .and_then(Value::as_str)
                .map(str::to_owned),
            brought,
        }
    }
}

fn is_id(text: &str) -> bool {
    (1..=128).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn is_real_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

fn candidate(meta_path: &Path, is_current: bool) -> Option<Candidate> {
    let name = meta_path.file_name()?.to_str()?;
    let agent_id = name
        .strip_prefix(AGENT_PREFIX)?
        .strip_suffix(META_SUFFIX)?
        .to_owned();
    if !is_id(&agent_id) {
        return None;
    }
    let transcript =
        meta_path.with_file_name(format!("{AGENT_PREFIX}{agent_id}{TRANSCRIPT_SUFFIX}"));
    if !is_real_file(&transcript) {
        return None;
    }
    let Ok(Value::Object(meta)) =
        serde_json::from_str::<Value>(&fs::read_to_string(meta_path).ok()?)
    else {
        return None;
    };
    if meta.get("agentType").and_then(Value::as_str) != Some(FORK_AGENT) {
        return None;
    }
    let modified = fs::metadata(&transcript).ok()?.modified().ok()?;
    Some(Candidate {
        agent_id,
        transcript,
        meta_path: meta_path.to_path_buf(),
        meta,
        is_current,
        modified,
    })
}

fn find(conversations: &Path, current: &str, fork: &str) -> Option<Candidate> {
    fs::read_dir(conversations)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .flat_map(|entry| {
            let is_current = entry.file_name().to_str() == Some(current);
            meta_files(&entry.path().join("subagents"))
                .into_iter()
                .filter_map(move |meta| candidate(&meta, is_current))
        })
        .filter_map(|one| {
            if one.agent_id == fork {
                Some((true, one))
            } else if one.key_name().as_deref() == Some(fork) {
                Some((false, one))
            } else {
                None
            }
        })
        .max_by_key(|(is_id, one)| (*is_id, one.modified, one.is_current))
        .map(|(_, one)| one)
}

fn resumable_meta(meta: &serde_json::Map<String, Value>) -> Value {
    let mut meta = meta.clone();
    for key in RESUME_BLOCKERS {
        meta.remove(key);
    }
    Value::Object(meta)
}

fn copy_transcript(source: &Path, destination: &Path, from: &str, to: &str) -> Result<()> {
    let partial = destination.with_extension(format!(
        "jsonl.{}{PARTIAL_SUFFIX}",
        fresh_id(&destination.to_string_lossy())
    ));
    let reader = BufReader::new(
        fs::File::open(source).with_context(|| format!("failed to open {}", source.display()))?,
    );
    let mut writer = BufWriter::new(
        fs::File::create_new(&partial)
            .with_context(|| format!("failed to create {}", partial.display()))?,
    );
    for line in reader.split(b'\n') {
        let line = line.with_context(|| format!("failed to read {}", source.display()))?;
        let parsed = if from == to {
            None
        } else {
            serde_json::from_slice::<Value>(&line).ok()
        };
        let rewritten = parsed.and_then(|row| match row {
            Value::Object(mut row)
                if row.get(AGENT_ID_KEY).and_then(Value::as_str) == Some(from) =>
            {
                row.insert(AGENT_ID_KEY.to_owned(), Value::String(to.to_owned()));
                serde_json::to_vec(&row).ok()
            }
            _ => None,
        });
        writer.write_all(rewritten.as_deref().unwrap_or(line.as_slice()))?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    drop(writer);
    fs::rename(&partial, destination).with_context(|| {
        let _ = fs::remove_file(&partial);
        format!("failed to place {}", destination.display())
    })
}

fn bring(found: &Candidate, subagents: &Path, agent_id: &str) -> Result<()> {
    fs::create_dir_all(subagents)
        .with_context(|| format!("failed to create {}", subagents.display()))?;
    let transcript = subagents.join(format!("{AGENT_PREFIX}{agent_id}{TRANSCRIPT_SUFFIX}"));
    let meta = subagents.join(format!("{AGENT_PREFIX}{agent_id}{META_SUFFIX}"));
    copy_transcript(&found.transcript, &transcript, &found.agent_id, agent_id)?;
    qol_fs::atomic_write(&meta, resumable_meta(&found.meta).to_string().as_bytes())
        .with_context(|| format!("failed to write {}", meta.display()))
}

pub(super) fn fresh_id(seed: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    let digest = Sha256::digest(format!("{seed}:{nanos}:{}", std::process::id()).as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("a{}", &hex[..FRESH_ID_HEX])
}

pub(super) fn adopt(
    conversations: &Path,
    current: &str,
    fork: &str,
    fresh: Option<String>,
) -> Result<Adopted> {
    if !is_id(current) || !is_id(fork) {
        bail!("a conversation and a fork are named by letters, digits, - and _");
    }
    let found = find(conversations, current, fork)
        .ok_or_else(|| anyhow!("no fork named {fork} has a conversation to bring back"))?;
    let subagents = conversations.join(current).join("subagents");
    match fresh {
        None if found.is_current => {
            if RESUME_BLOCKERS
                .iter()
                .any(|key| found.meta.contains_key(*key))
            {
                qol_fs::atomic_write(
                    &found.meta_path,
                    resumable_meta(&found.meta).to_string().as_bytes(),
                )
                .with_context(|| format!("failed to write {}", found.meta_path.display()))?;
            }
            Ok(found.adopted(found.agent_id.clone(), Brought::Here))
        }
        None => {
            bring(&found, &subagents, &found.agent_id)?;
            Ok(found.adopted(found.agent_id.clone(), Brought::Copied))
        }
        Some(agent_id) => {
            if !is_id(&agent_id) {
                bail!("the new agent id {agent_id} is not an id");
            }
            bring(&found, &subagents, &agent_id)?;
            Ok(found.adopted(agent_id, Brought::Fresh))
        }
    }
}

struct AdoptArgs {
    conversations: PathBuf,
    conversation: String,
    fork: String,
    is_fresh: bool,
}

fn parse(args: &[OsString]) -> Result<AdoptArgs> {
    let (mut conversations, mut conversation, mut fork, mut is_fresh) = (None, None, None, false);
    let mut index = 0;
    while index < args.len() {
        match args[index].to_str().ok_or_else(|| anyhow!("{USAGE}"))? {
            "--conversations" => {
                conversations = Some(flag_value(args, &mut index, "--conversations")?)
            }
            "--conversation" => {
                conversation = Some(flag_value(args, &mut index, "--conversation")?)
            }
            "--fork" => fork = Some(flag_value(args, &mut index, "--fork")?),
            "--fresh" => is_fresh = true,
            other => bail!("unknown adopt flag `{other}`\n{USAGE}"),
        }
        index += 1;
    }
    match (conversations, conversation, fork) {
        (Some(conversations), Some(conversation), Some(fork)) => Ok(AdoptArgs {
            conversations: PathBuf::from(conversations),
            conversation,
            fork,
            is_fresh,
        }),
        _ => bail!("{USAGE}"),
    }
}

fn is_switchboards(conversations: &Path, switchboard: Option<&Path>) -> bool {
    conversations.is_absolute()
        && switchboard.is_some_and(|switchboard| {
            conversations.file_name().and_then(|name| name.to_str())
                == Some(project_dir_name(switchboard).as_str())
        })
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let parsed = parse(args)?;
    if !is_switchboards(&parsed.conversations, switchboard_dir().as_deref()) {
        bail!(
            "{} is not the switchboard's conversations folder",
            parsed.conversations.display()
        );
    }
    let fresh = parsed.is_fresh.then(|| fresh_id(&parsed.fork));
    let adopted = adopt(
        &parsed.conversations,
        &parsed.conversation,
        &parsed.fork,
        fresh,
    )?;
    crate::cli::line(serde_json::to_string(&adopted)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fork(conversations: &Path, conversation: &str, id: &str, meta: &str, rows: &[String]) {
        let subagents = conversations.join(conversation).join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(subagents.join(format!("agent-{id}.meta.json")), meta).unwrap();
        fs::write(
            subagents.join(format!("agent-{id}.jsonl")),
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        fs::write(conversations.join(format!("{conversation}.jsonl")), "").unwrap();
    }

    fn rows(id: &str) -> Vec<String> {
        vec![
            format!(
                r#"{{"agentId":"{id}","type":"user","message":{{"content":"remember PINEAPPLE-42"}}}}"#
            ),
            format!(
                r#"{{"agentId":"{id}","type":"assistant","message":{{"content":"agentId {id} noted"}}}}"#
            ),
            "not json".to_owned(),
        ]
    }

    fn meta(description: &str, extra: &str) -> String {
        format!(
            r#"{{"agentType":"project-fork","description":"{description}","cwd":"/work/app"{extra}}}"#
        )
    }

    fn read_meta(path: PathBuf) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_fork_of_an_earlier_conversation_is_copied_in_under_its_own_id_by_id_or_key_name() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        fork(
            conversations,
            "older",
            "a1",
            &meta(
                "kcd2-modding: aim the rifle",
                r#","stoppedByUser":true,"finished":true"#,
            ),
            &rows("a1"),
        );
        fs::create_dir_all(conversations.join("current")).unwrap();
        let adopted = adopt(conversations, "current", "kcd2-modding", None).unwrap();
        assert_eq!(adopted.agent_id, "a1");
        assert_eq!(adopted.brought, Brought::Copied);
        assert_eq!(adopted.name.as_deref(), Some("kcd2-modding: aim the rifle"));
        assert_eq!(adopted.cwd.as_deref(), Some("/work/app"));
        let here = conversations.join("current").join("subagents");
        assert_eq!(
            fs::read_to_string(here.join("agent-a1.jsonl")).unwrap(),
            fs::read_to_string(conversations.join("older/subagents/agent-a1.jsonl")).unwrap(),
            "the same id keeps every row as it was"
        );
        assert_eq!(
            read_meta(here.join("agent-a1.meta.json")),
            serde_json::json!({"agentType": "project-fork", "description": "kcd2-modding: aim the rifle", "cwd": "/work/app"})
        );
        assert!(
            conversations
                .join("older/subagents/agent-a1.jsonl")
                .exists(),
            "the earlier conversation keeps its own copy"
        );
        let again = adopt(conversations, "current", "a1", None).unwrap();
        assert_eq!(
            (again.agent_id.as_str(), again.brought),
            ("a1", Brought::Here)
        );
    }

    #[test]
    fn a_fresh_copy_takes_a_new_id_in_every_row_that_named_the_old_one() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        fork(
            conversations,
            "current",
            "a1",
            &meta("fix-board: fix it", r#","stoppedByUser":true"#),
            &rows("a1"),
        );
        let adopted = adopt(conversations, "current", "fix-board", Some("a2".to_owned())).unwrap();
        assert_eq!(
            (adopted.agent_id.as_str(), adopted.brought),
            ("a2", Brought::Fresh)
        );
        let subagents = conversations.join("current/subagents");
        let copied = fs::read_to_string(subagents.join("agent-a2.jsonl")).unwrap();
        let lines: Vec<&str> = copied.lines().collect();
        assert_eq!(lines.len(), 3);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["agentId"], "a2");
        assert_eq!(first["message"]["content"], "remember PINEAPPLE-42");
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["agentId"], "a2");
        assert_eq!(second["message"]["content"], "agentId a1 noted");
        assert_eq!(lines[2], "not json");
        assert!(read_meta(subagents.join("agent-a2.meta.json"))
            .get("stoppedByUser")
            .is_none());
        assert!(
            subagents.join("agent-a1.jsonl").exists(),
            "the stopped fork keeps its own conversation"
        );
        let partials = fs::read_dir(&subagents)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(PARTIAL_SUFFIX)
            })
            .count();
        assert_eq!(partials, 0);
    }

    #[test]
    fn a_stopped_fork_of_this_conversation_is_made_resumable_again() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        fork(
            conversations,
            "current",
            "a1",
            &meta("fix-board: fix it", r#","stoppedByUser":true"#),
            &rows("a1"),
        );
        let subagents = conversations.join("current/subagents");
        let before = fs::read_to_string(subagents.join("agent-a1.jsonl")).unwrap();
        let adopted = adopt(conversations, "current", "a1", None).unwrap();
        assert_eq!(adopted.brought, Brought::Here);
        assert_eq!(
            read_meta(subagents.join("agent-a1.meta.json")),
            serde_json::json!({"agentType": "project-fork", "description": "fix-board: fix it", "cwd": "/work/app"})
        );
        assert_eq!(
            fs::read_to_string(subagents.join("agent-a1.jsonl")).unwrap(),
            before
        );
    }

    #[test]
    fn the_newest_fork_with_the_key_name_wins_and_an_id_beats_a_name() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        fork(
            conversations,
            "c1",
            "a1",
            &meta("same: one", ""),
            &rows("a1"),
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        fork(
            conversations,
            "c2",
            "a2",
            &meta("same: two", ""),
            &rows("a2"),
        );
        fork(
            conversations,
            "c2",
            "same",
            &meta("other: three", ""),
            &rows("same"),
        );
        let nested = conversations.join("c1/subagents/agent-a1");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("agent-a9.meta.json"), meta("deep: four", "")).unwrap();
        fs::write(nested.join("agent-a9.jsonl"), "{}\n").unwrap();
        fork(
            conversations,
            "c1",
            "a7",
            r#"{"agentType":"Explore","description":"explore: look"}"#,
            &rows("a7"),
        );
        let pick = |fork: &str| {
            let picked = adopt(conversations, "current", fork, None).map(|one| one.agent_id);
            let _ = fs::remove_dir_all(conversations.join("current"));
            picked
        };
        assert_eq!(pick("same").unwrap(), "same");
        fs::remove_file(conversations.join("c2/subagents/agent-same.meta.json")).unwrap();
        assert_eq!(pick("same").unwrap(), "a2");
        assert_eq!(pick("deep").unwrap(), "a9");
        assert!(pick("explore").is_err());
        assert!(pick("missing").is_err());
        assert!(pick("../c1").is_err());
        assert!(adopt(conversations, "../up", "same", None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_transcript_that_is_a_link_is_never_a_source() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        fork(
            conversations,
            "c1",
            "a1",
            &meta("linked: one", ""),
            &rows("a1"),
        );
        let subagents = conversations.join("c1/subagents");
        fs::remove_file(subagents.join("agent-a1.jsonl")).unwrap();
        std::os::unix::fs::symlink(
            root.path().join("elsewhere"),
            subagents.join("agent-a1.jsonl"),
        )
        .unwrap();
        fs::write(root.path().join("elsewhere"), "{}\n").unwrap();
        assert!(adopt(conversations, "current", "a1", None).is_err());
    }

    #[test]
    fn only_the_switchboards_own_conversations_folder_is_accepted() {
        let projects = std::env::temp_dir().join("projects");
        let switchboard = std::env::temp_dir().join("sessions").join("switchboard");
        let own = projects.join(project_dir_name(&switchboard));
        assert!(is_switchboards(&own, Some(&switchboard)));
        assert!(!is_switchboards(
            &projects.join("-other"),
            Some(&switchboard)
        ));
        assert!(!is_switchboards(
            Path::new(&project_dir_name(&switchboard)),
            Some(&switchboard)
        ));
        assert!(!is_switchboards(&own, None));
    }

    #[test]
    fn a_fresh_id_looks_like_a_claude_code_agent_id_and_differs_each_time() {
        let one = fresh_id("a1");
        assert_eq!(one.len(), 17);
        assert!(one.starts_with('a') && is_id(&one));
        assert_ne!(one, fresh_id("a2"));
    }

    #[test]
    fn the_flags_parse_and_a_missing_one_names_the_usage() {
        let args: Vec<OsString> = [
            "--conversations",
            "/c",
            "--conversation",
            "s",
            "--fork",
            "kcd2",
            "--fresh",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let parsed = parse(&args).unwrap();
        assert_eq!(parsed.conversations, PathBuf::from("/c"));
        assert_eq!(
            (parsed.conversation.as_str(), parsed.fork.as_str()),
            ("s", "kcd2")
        );
        assert!(parsed.is_fresh);
        let error = parse(&args[..4]).err().unwrap().to_string();
        assert!(error.contains("--fork"), "{error}");
        assert!(parse(&[OsString::from("--bogus")]).is_err());
    }
}
