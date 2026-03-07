use anyhow::{Context, Result};
use std::path::PathBuf;

pub(super) fn read_autostart_target() -> Result<Option<PathBuf>> {
    let path = crate::installer::autostart_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_cmd_autostart_target(&content).map(PathBuf::from))
}

fn parse_cmd_autostart_target(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.to_ascii_lowercase().starts_with("start") {
            continue;
        }
        if let Some(result) = parse_start_line(trimmed) {
            return Some(result);
        }
    }
    None
}

fn parse_start_line(trimmed: &str) -> Option<String> {
    let quoted = quoted_segments(trimmed);
    if quoted.len() >= 2 {
        return Some(quoted[1].clone());
    }
    if quoted.len() == 1 {
        return Some(quoted[0].clone());
    }
    let rest = trimmed
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(idx, _)| trimmed[idx..].trim())
        .unwrap_or("");
    if rest.is_empty() {
        return None;
    }
    Some(rest.trim_matches('"').to_string())
}

fn quoted_segments(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = None;
    for (idx, ch) in value.char_indices() {
        if ch != '"' {
            continue;
        }
        if let Some(open_idx) = start.take() {
            out.push(value[open_idx + 1..idx].to_string());
        } else {
            start = Some(idx);
        }
    }
    out
}
