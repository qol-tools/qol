use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn read_autostart_target() -> Result<Option<PathBuf>> {
    let path = crate::installer::autostart_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    for line in content.lines() {
        let Some(exec_line) = line.strip_prefix("Exec=") else {
            continue;
        };
        return Ok(parse_exec_line(exec_line));
    }

    Ok(None)
}

fn parse_exec_line(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(PathBuf::from(&rest[..end]));
    }
    trimmed.split_whitespace().next().map(PathBuf::from)
}
