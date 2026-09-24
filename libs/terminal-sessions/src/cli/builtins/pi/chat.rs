use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cli::chat::{read_chat, text_blocks, ChatRole, ChatTurn};

pub(super) fn newest_path(paths: &[PathBuf]) -> Option<PathBuf> {
    paths
        .iter()
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        })
        .map(|path| (*path).to_path_buf())
}

pub(super) fn chat_transcript(path: &Path) -> Option<Vec<ChatTurn>> {
    read_chat(path, append_chat_turn)
}

fn append_chat_turn(turns: &mut Vec<ChatTurn>, value: &Value) {
    match value.get("type").and_then(Value::as_str) {
        Some("message") => {
            let Some(message) = value.get("message") else {
                return;
            };
            let role = match message.get("role").and_then(Value::as_str) {
                Some("user") => ChatRole::User,
                Some("assistant") => ChatRole::Assistant,
                _ => return,
            };
            let text = text_blocks(message.get("content").unwrap_or(&Value::Null));
            if role == ChatRole::User && is_injected_user_text(&text) {
                return;
            }
            turns.push(ChatTurn { role, text });
        }
        Some("compaction") => {
            if let Some(summary) = value.get("summary").and_then(Value::as_str) {
                turns.push(ChatTurn {
                    role: ChatRole::User,
                    text: format!("[compacted summary]\n{summary}"),
                });
            }
        }
        _ => {}
    }
}

fn is_injected_user_text(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("<skill ") || text.starts_with("<qol sessions:")
}
