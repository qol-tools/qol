use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

const FIRST_WINDOW: u64 = 1 << 20;
const WINDOW_GROWTH: u64 = 4;
const LAST_WINDOW: u64 = 1 << 26;
const TEXT_CHARS: usize = 8_000;
const ROWS_MAX: usize = 400;
const USER: &str = "user";
const ASSISTANT: &str = "assistant";
const USER_TAG: &str = "\"type\":\"user\"";
const ASSISTANT_TAG: &str = "\"type\":\"assistant\"";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct Said {
    role: &'static str,
    text: String,
}

fn clipped(text: String) -> String {
    match text.char_indices().nth(TEXT_CHARS) {
        Some((at, _)) => format!("{}\u{2026}", &text[..at]),
        None => text,
    }
}

fn text_of(row: &Value) -> Option<String> {
    let text = match row.pointer("/message/content")? {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect(),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

fn said_of(line: &str) -> Option<Said> {
    if !line.contains(USER_TAG) && !line.contains(ASSISTANT_TAG) {
        return None;
    }
    let row: Value = serde_json::from_str(line).ok()?;
    let role = match row.get("type").and_then(Value::as_str)? {
        USER => USER,
        ASSISTANT => ASSISTANT,
        _ => return None,
    };
    Some(Said {
        role,
        text: clipped(text_of(&row)?),
    })
}

fn tail(file: &mut File, len: u64, window: u64) -> io::Result<Vec<u8>> {
    let start = len.saturating_sub(window);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(len - start).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn newest_in(bytes: &[u8], is_whole: bool, rows: usize) -> Vec<Said> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    if !is_whole {
        lines.next();
    }
    lines.rev().filter_map(said_of).take(rows).collect()
}

pub(super) fn newest(transcript: &Path, rows: usize) -> Vec<Said> {
    let rows = rows.clamp(1, ROWS_MAX);
    let Ok(mut file) = File::open(transcript) else {
        return Vec::new();
    };
    let Ok(len) = file.metadata().map(|metadata| metadata.len()) else {
        return Vec::new();
    };
    let mut window = FIRST_WINDOW;
    loop {
        let Ok(bytes) = tail(&mut file, len, window) else {
            return Vec::new();
        };
        let is_whole = window >= len;
        let mut said = newest_in(&bytes, is_whole, rows);
        if said.len() >= rows || is_whole || window >= LAST_WINDOW {
            said.reverse();
            return said;
        }
        window = window.saturating_mul(WINDOW_GROWTH);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    fn row(kind: &str, content: Value) -> String {
        json!({"type": kind, "message": {"content": content}}).to_string()
    }

    fn said(role: &'static str, text: &str) -> Said {
        Said {
            role,
            text: text.to_owned(),
        }
    }

    fn written(rows: &[String]) -> (tempfile::TempDir, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent-a1.jsonl");
        fs::write(
            &path,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        (root, path)
    }

    #[test]
    fn the_newest_words_come_back_oldest_first_without_tool_rows() {
        let (_root, path) = written(&[
            row("user", json!("first ask")),
            row("assistant", json!([{"type": "thinking", "thinking": "hm"}])),
            row(
                "assistant",
                json!([{"type": "tool_use", "name": "Bash", "input": {}}]),
            ),
            row("user", json!([{"type": "tool_result", "content": "ran"}])),
            row("attachment", json!("not a message")),
            "not json".to_owned(),
            row(
                "assistant",
                json!([{"type": "text", "text": "Amber "}, {"type": "text", "text": "or teal?"}]),
            ),
            row("user", json!([{"type": "text", "text": "teal"}])),
            row("assistant", json!("   ")),
        ]);

        assert_eq!(
            newest(&path, 10),
            vec![
                said(USER, "first ask"),
                said(ASSISTANT, "Amber or teal?"),
                said(USER, "teal"),
            ]
        );
        assert_eq!(
            newest(&path, 2),
            vec![said(ASSISTANT, "Amber or teal?"), said(USER, "teal")]
        );
        assert_eq!(newest(&path, 0), vec![said(USER, "teal")]);
        assert_eq!(newest(Path::new("/missing/agent-a1.jsonl"), 5), Vec::new());
    }

    #[test]
    fn words_past_the_first_window_are_found_and_a_row_cut_by_the_window_is_skipped() {
        let filler = "x".repeat(700_000);
        let (_root, path) = written(&[
            row("assistant", json!("far back")),
            row(
                "user",
                json!([{"type": "tool_result", "content": filler.clone()}]),
            ),
            row("user", json!([{"type": "tool_result", "content": filler}])),
            row("assistant", json!("near the end")),
        ]);
        let bytes = fs::read(&path).unwrap();
        let window = &bytes[bytes.len() - 600_000..];

        assert_eq!(
            newest_in(window, false, 5),
            vec![said(ASSISTANT, "near the end")]
        );
        assert_eq!(
            newest(&path, 5),
            vec![said(ASSISTANT, "far back"), said(ASSISTANT, "near the end")]
        );
    }

    #[test]
    fn a_long_message_is_cut_on_a_character_boundary() {
        let long = "\u{e9}".repeat(TEXT_CHARS + 5);
        let (_root, path) = written(&[row("assistant", json!(long))]);

        let found = newest(&path, 1);

        assert_eq!(found[0].text.chars().count(), TEXT_CHARS + 1);
        assert!(found[0].text.ends_with('\u{2026}'));
    }
}
