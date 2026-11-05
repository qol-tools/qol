use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use super::resume::AGENT_PREFIX;

const TRANSCRIPT_SUFFIX: &str = ".jsonl";
const SUBAGENTS: &str = "subagents";
const UUID_KEY: &str = "uuid";
const PARENT_KEY: &str = "parentUuid";
const TIME_KEY: &str = "timestamp";

struct Segment {
    lines: Vec<String>,
    first: Option<usize>,
    first_parent: Option<String>,
    starts_at: Option<String>,
    ends_at: Option<String>,
    leaf: Option<String>,
}

fn row(line: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str(line) {
        Ok(Value::Object(row)) => Some(row),
        _ => None,
    }
}

fn text<'a>(row: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

impl Segment {
    fn read(path: &Path) -> Result<Self> {
        let body = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut segment = Self {
            lines: body.lines().map(str::to_owned).collect(),
            first: None,
            first_parent: None,
            starts_at: None,
            ends_at: None,
            leaf: None,
        };
        for (index, line) in segment.lines.iter().enumerate() {
            let Some(row) = row(line) else {
                continue;
            };
            let at = text(&row, TIME_KEY);
            if let Some(at) = at {
                if segment.ends_at.as_deref().is_none_or(|seen| at > seen) {
                    segment.ends_at = Some(at.to_owned());
                }
            }
            let Some(uuid) = text(&row, UUID_KEY) else {
                continue;
            };
            if segment.first.is_none() {
                segment.first = Some(index);
                segment.first_parent = text(&row, PARENT_KEY).map(str::to_owned);
                segment.starts_at = at.map(str::to_owned);
            }
            segment.leaf = Some(uuid.to_owned());
        }
        Ok(segment)
    }

    fn continues(&self) -> bool {
        self.first_parent.is_some()
    }

    fn linked_to(mut self, leaf: &str) -> Result<Vec<String>> {
        if let Some(index) = self.first {
            let mut first = row(&self.lines[index]).context("a transcript row is not an object")?;
            first.insert(PARENT_KEY.to_owned(), Value::String(leaf.to_owned()));
            self.lines[index] = Value::Object(first).to_string();
        }
        Ok(self.lines)
    }
}

fn transcript_of(conversation: &Path, agent_id: &str) -> PathBuf {
    conversation
        .join(SUBAGENTS)
        .join(format!("{AGENT_PREFIX}{agent_id}{TRANSCRIPT_SUFFIX}"))
}

fn is_real_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

fn earlier_parts(conversations: &Path, transcript: &Path, agent_id: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(conversations) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| transcript_of(&entry.path(), agent_id))
        .filter(|path| path != transcript && is_real_file(path))
        .collect()
}

fn part_before(parts: &[PathBuf], starts_at: &str) -> Option<(PathBuf, Segment)> {
    let mut best: Option<(PathBuf, Segment)> = None;
    for path in parts {
        let Ok(segment) = Segment::read(path) else {
            continue;
        };
        let (Some(ends_at), Some(_)) = (segment.ends_at.as_deref(), segment.leaf.as_deref()) else {
            continue;
        };
        let newer = best
            .as_ref()
            .is_none_or(|(_, kept)| kept.ends_at.as_deref() < Some(ends_at));
        if ends_at <= starts_at && newer {
            best = Some((path.clone(), segment));
        }
    }
    best
}

pub(super) fn join_fork(conversations: &Path, transcript: &Path, agent_id: &str) -> Result<bool> {
    let current = Segment::read(transcript)?;
    if !current.continues() {
        return Ok(false);
    }
    let mut parts = earlier_parts(conversations, transcript, agent_id);
    let mut chain = vec![current];
    while let Some(starts_at) = chain
        .last()
        .filter(|segment| segment.continues())
        .and_then(|segment| segment.starts_at.clone())
    {
        let Some((path, segment)) = part_before(&parts, &starts_at) else {
            break;
        };
        parts.retain(|part| part != &path);
        chain.push(segment);
    }
    if chain.len() == 1 {
        return Ok(false);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut leaf: Option<String> = None;
    for segment in chain.into_iter().rev() {
        let next_leaf = segment.leaf.clone();
        lines.extend(match leaf.as_deref() {
            Some(leaf) => segment.linked_to(leaf)?,
            None => segment.lines,
        });
        leaf = next_leaf.or(leaf);
    }
    let mut body = lines.join("\n");
    body.push('\n');
    qol_fs::atomic_write_private(transcript, body.as_bytes())
        .with_context(|| format!("failed to write {}", transcript.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const META_SUFFIX: &str = ".meta.json";

    fn line(uuid: &str, parent: Option<&str>, at: &str, session: &str) -> String {
        serde_json::json!({
            "type": "user",
            "uuid": uuid,
            "parentUuid": parent,
            "timestamp": at,
            "sessionId": session,
            "agentId": "afork",
        })
        .to_string()
    }

    fn write(conversations: &Path, conversation: &str, rows: &[String]) -> PathBuf {
        let path = transcript_of(&conversations.join(conversation), "afork");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("{}\n", rows.join("\n"))).unwrap();
        fs::write(
            path.with_file_name(format!("{AGENT_PREFIX}afork{META_SUFFIX}")),
            r#"{"agentType":"project-fork"}"#,
        )
        .unwrap();
        path
    }

    fn chain(path: &Path) -> Vec<(String, Option<String>)> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| {
                let row = row(line).unwrap();
                (
                    text(&row, UUID_KEY).unwrap().to_owned(),
                    text(&row, PARENT_KEY).map(str::to_owned),
                )
            })
            .collect()
    }

    fn pair(uuid: &str, parent: Option<&str>) -> (String, Option<String>) {
        (uuid.to_owned(), parent.map(str::to_owned))
    }

    #[test]
    fn a_fork_split_by_clear_gets_its_earlier_conversation_back_in_front() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        write(
            conversations,
            "before",
            &[
                line("u1", None, "2026-11-02T07:00:00.000Z", "before"),
                line("u2", Some("u1"), "2026-11-02T07:15:30.000Z", "before"),
            ],
        );
        let stub = write(
            conversations,
            "after",
            &[
                line("u3", Some("lost"), "2026-11-02T09:18:16.000Z", "after"),
                line("u4", Some("u3"), "2026-11-02T09:18:24.000Z", "after"),
            ],
        );
        assert!(join_fork(conversations, &stub, "afork").unwrap());
        assert_eq!(
            chain(&stub),
            vec![
                pair("u1", None),
                pair("u2", Some("u1")),
                pair("u3", Some("u2")),
                pair("u4", Some("u3")),
            ]
        );
        assert!(
            !join_fork(conversations, &stub, "afork").unwrap(),
            "a joined transcript starts at its launch, so joining again changes nothing"
        );
    }

    #[test]
    fn each_clear_adds_a_part_the_newest_earlier_copy_wins_and_an_unreadable_one_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        write(
            conversations,
            "first",
            &[line("u1", None, "2026-11-02T01:00:00.000Z", "first")],
        );
        write(
            conversations,
            "copied",
            &[
                line("u1", None, "2026-11-02T01:00:00.000Z", "first"),
                line("u2", Some("u1"), "2026-11-02T02:00:00.000Z", "copied"),
            ],
        );
        write(
            conversations,
            "second",
            &[line(
                "u3",
                Some("gone"),
                "2026-11-02T03:00:00.000Z",
                "second",
            )],
        );
        let unreadable = transcript_of(&conversations.join("broken"), "afork");
        fs::create_dir_all(unreadable.parent().unwrap()).unwrap();
        fs::write(&unreadable, [0xff, 0xfe, b'\n']).unwrap();
        write(
            conversations,
            "later",
            &[line("u9", None, "2026-11-02T08:00:00.000Z", "later")],
        );
        let stub = write(
            conversations,
            "third",
            &[line(
                "u4",
                Some("gone"),
                "2026-11-02T04:00:00.000Z",
                "third",
            )],
        );
        assert!(join_fork(conversations, &stub, "afork").unwrap());
        assert_eq!(
            chain(&stub),
            vec![
                pair("u1", None),
                pair("u2", Some("u1")),
                pair("u3", Some("u2")),
                pair("u4", Some("u3")),
            ]
        );
    }

    #[test]
    fn a_transcript_that_starts_at_its_launch_or_has_no_earlier_part_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let conversations = root.path();
        let whole = write(
            conversations,
            "whole",
            &[line("u1", None, "2026-11-02T01:00:00.000Z", "whole")],
        );
        let before = fs::read_to_string(&whole).unwrap();
        assert!(!join_fork(conversations, &whole, "afork").unwrap());
        assert_eq!(fs::read_to_string(&whole).unwrap(), before);
        fs::remove_dir_all(conversations.join("whole")).unwrap();
        let alone = write(
            conversations,
            "alone",
            &[line(
                "u2",
                Some("lost"),
                "2026-11-02T02:00:00.000Z",
                "alone",
            )],
        );
        let before = fs::read_to_string(&alone).unwrap();
        assert!(!join_fork(conversations, &alone, "afork").unwrap());
        assert_eq!(fs::read_to_string(&alone).unwrap(), before);
    }
}
