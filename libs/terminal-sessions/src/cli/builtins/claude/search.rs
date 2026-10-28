use std::fs;
use std::io::{BufRead, BufReader};
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

use aho_corasick::AhoCorasick;
use serde_json::Value;

use crate::cli::{ChatRole, ChatTurn, CliConversationMatch};

use super::metadata::append_chat_turn;

const READ_BUFFER: usize = 1 << 20;
const EXCERPT_LEAD_CHARS: usize = 60;
const EXCERPT_CHARS: usize = 200;
const ELLIPSIS: &str = "…";

struct Best {
    hits: usize,
    role: ChatRole,
    text: String,
    offset: usize,
}

pub(super) fn search_transcripts(
    paths: Vec<PathBuf>,
    terms: &[String],
) -> Vec<CliConversationMatch> {
    if paths.is_empty() || terms.is_empty() {
        return Vec::new();
    }
    let Ok(searcher) = AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(terms)
    else {
        return Vec::new();
    };
    let mut sized = paths
        .into_iter()
        .map(|path| {
            let length = fs::metadata(&path).map_or(0, |metadata| metadata.len());
            (length, path)
        })
        .collect::<Vec<_>>();
    sized.sort_by_key(|(length, _)| std::cmp::Reverse(*length));
    let workers = thread::available_parallelism()
        .map_or(1, NonZero::get)
        .min(sized.len());
    let next = AtomicUsize::new(0);
    let found = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let Some((_, path)) = sized.get(next.fetch_add(1, Ordering::Relaxed)) else {
                    break;
                };
                if let Some(hit) = search_file(path, &searcher, terms) {
                    if let Ok(mut found) = found.lock() {
                        found.push(hit);
                    }
                }
            });
        }
    });
    found.into_inner().unwrap_or_default()
}

fn search_file(
    path: &Path,
    searcher: &AhoCorasick,
    terms: &[String],
) -> Option<CliConversationMatch> {
    let external_id = path.file_stem()?.to_str()?.to_owned();
    let mut reader = BufReader::with_capacity(READ_BUFFER, fs::File::open(path).ok()?);
    let mut line = Vec::new();
    let mut seen = vec![false; terms.len()];
    let mut best: Option<Best> = None;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            break;
        }
        if !searcher.is_match(&line) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        for turn in searchable_turns(&value) {
            absorb(turn, terms, &mut seen, &mut best);
        }
        if seen.iter().all(|seen| *seen) {
            break;
        }
    }
    if !seen.iter().all(|seen| *seen) {
        return None;
    }
    let best = best?;
    Some(CliConversationMatch {
        external_id,
        excerpt: excerpt(&best),
    })
}

fn absorb(turn: ChatTurn, terms: &[String], seen: &mut [bool], best: &mut Option<Best>) {
    if turn.text.trim().is_empty() {
        return;
    }
    let lowered = turn.text.to_lowercase();
    let mut hits = 0;
    let mut first = usize::MAX;
    for (term, seen) in terms.iter().zip(seen.iter_mut()) {
        if let Some(position) = lowered.find(term.as_str()) {
            hits += 1;
            first = first.min(position);
            *seen = true;
        }
    }
    if hits == 0 || best.as_ref().is_some_and(|best| best.hits >= hits) {
        return;
    }
    let offset = if lowered.len() == turn.text.len() && turn.text.is_char_boundary(first) {
        first
    } else {
        0
    };
    *best = Some(Best {
        hits,
        role: turn.role,
        text: turn.text,
        offset,
    });
}

fn searchable_turns(value: &Value) -> Vec<ChatTurn> {
    let mut turns = Vec::new();
    append_chat_turn(&mut turns, value);
    turns.extend(tool_inputs(value).into_iter().map(|text| ChatTurn {
        role: ChatRole::Assistant,
        text,
    }));
    turns
}

fn tool_inputs(value: &Value) -> Vec<String> {
    if value.get("type").and_then(Value::as_str) != Some("assistant")
        || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return Vec::new();
    }
    let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|block| block.get("input"))
        .map(|input| {
            let mut leaves = Vec::new();
            string_leaves(input, &mut leaves);
            leaves.join("\n")
        })
        .collect()
}

fn string_leaves<'a>(value: &'a Value, leaves: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => leaves.push(text),
        Value::Array(items) => items.iter().for_each(|item| string_leaves(item, leaves)),
        Value::Object(fields) => fields
            .values()
            .for_each(|field| string_leaves(field, leaves)),
        _ => {}
    }
}

fn excerpt(best: &Best) -> String {
    let text = best.text.as_str();
    let start = text[..best.offset]
        .char_indices()
        .rev()
        .nth(EXCERPT_LEAD_CHARS - 1)
        .map_or(0, |(index, _)| index);
    let mut window = String::new();
    let mut chars = 0;
    let mut gap = false;
    let mut cut_end = false;
    for ch in text[start..].chars() {
        if ch.is_whitespace() {
            gap = chars > 0;
            continue;
        }
        let needed = if gap { 2 } else { 1 };
        if chars + needed > EXCERPT_CHARS {
            cut_end = true;
            break;
        }
        if gap {
            window.push(' ');
            chars += 1;
            gap = false;
        }
        window.push(ch);
        chars += 1;
    }
    let role = match best.role {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    };
    let cut_start = !text[..start].trim().is_empty();
    let lead = if cut_start { ELLIPSIS } else { "" };
    let tail = if cut_end { ELLIPSIS } else { "" };
    format!("{role}: {lead}{window}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        })
        .to_string()
    }

    fn assistant(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
        })
        .to_string()
    }

    fn write(dir: &Path, id: &str, lines: &[String]) -> PathBuf {
        let path = dir.join(format!("{id}.jsonl"));
        fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        path
    }

    fn terms(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn terms_spread_over_a_prompt_and_a_reply_match_with_the_richest_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "abc",
            &[
                user("please check the invoice"),
                assistant("the refund for the invoice was issued"),
            ],
        );

        let found = search_transcripts(vec![path], &terms(&["invoice", "refund"]));

        assert_eq!(
            found,
            [CliConversationMatch {
                external_id: "abc".to_owned(),
                excerpt: "assistant: the refund for the invoice was issued".to_owned(),
            }]
        );
    }

    #[test]
    fn only_files_holding_every_term_are_returned() {
        let dir = tempfile::tempdir().unwrap();
        let both = write(dir.path(), "both", &[user("invoice"), assistant("refund")]);
        let one = write(dir.path(), "one", &[user("invoice only")]);

        let found = search_transcripts(vec![one, both], &terms(&["invoice", "refund"]));

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].external_id, "both");
    }

    #[test]
    fn a_term_only_inside_a_tool_result_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let result = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "the refund policy" }
            ] },
        })
        .to_string();
        let path = write(dir.path(), "abc", &[user("hello there"), result]);

        let found = search_transcripts(vec![path], &terms(&["hello", "refund"]));

        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_system_reminder_prompt_does_not_match() {
        let dir = tempfile::tempdir().unwrap();
        let reminder = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": "<system-reminder>refund policy</system-reminder>" },
        })
        .to_string();
        let path = write(dir.path(), "abc", &[reminder]);

        let found = search_transcripts(vec![path], &terms(&["refund"]));

        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_term_inside_a_tool_call_input_matches() {
        let dir = tempfile::tempdir().unwrap();
        let call = serde_json::json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [{
                "type": "tool_use",
                "name": "mcp__gmail__send_message",
                "input": { "to": "a@b.c", "body": "Quarterly budget attached", "cc": ["x@y.z"] },
            }] },
        })
        .to_string();
        let path = write(dir.path(), "abc", &[user("send it"), call]);

        let found = search_transcripts(vec![path], &terms(&["budget"]));

        assert_eq!(found.len(), 1);
        assert!(found[0].excerpt.starts_with("assistant: "), "{found:?}");
        assert!(
            found[0].excerpt.contains("Quarterly budget attached"),
            "{found:?}"
        );
    }

    #[test]
    fn matching_ignores_case_and_keeps_the_original_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "abc", &[user("Please Review The INVOICE")]);

        let found = search_transcripts(vec![path], &terms(&["invoice"]));

        assert_eq!(found[0].excerpt, "user: Please Review The INVOICE");
    }

    #[test]
    fn a_hit_deep_in_a_long_message_is_cut_with_ellipses() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "{}needle{}",
            "lorem ipsum ".repeat(30),
            " dolor   sit\namet ".repeat(40)
        );
        let path = write(dir.path(), "abc", &[user(&text)]);

        let found = search_transcripts(vec![path], &terms(&["needle"]));

        let body = found[0].excerpt.strip_prefix("user: ").unwrap();
        assert!(
            body.starts_with(ELLIPSIS) && body.ends_with(ELLIPSIS),
            "{body}"
        );
        assert!(body.contains("needle"));
        assert_eq!(body.chars().count(), EXCERPT_CHARS + 2);
        assert!(!body.contains("  ") && !body.contains('\n'));
    }
}
