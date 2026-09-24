use std::fs;
use std::path::Path;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub text: String,
}

pub(super) fn merge_chat_turns(turns: Vec<ChatTurn>) -> Vec<ChatTurn> {
    let mut merged: Vec<ChatTurn> = Vec::new();
    for turn in turns {
        let text = turn.text.trim();
        if text.is_empty() {
            continue;
        }
        match merged.last_mut() {
            Some(last) if last.role == turn.role => {
                last.text.push_str("\n\n");
                last.text.push_str(text);
            }
            _ => merged.push(ChatTurn {
                role: turn.role,
                text: text.to_owned(),
            }),
        }
    }
    merged
}

pub(super) fn read_chat(
    path: &Path,
    append: fn(&mut Vec<ChatTurn>, &Value),
) -> Option<Vec<ChatTurn>> {
    let contents = fs::read_to_string(path).ok()?;
    let mut turns = Vec::new();
    for line in contents.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        append(&mut turns, &value);
    }
    Some(merge_chat_turns(turns))
}

pub(super) fn text_blocks(content: &Value) -> String {
    if let Value::String(text) = content {
        return text.clone();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
}
