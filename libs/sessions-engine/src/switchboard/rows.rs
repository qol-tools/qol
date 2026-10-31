use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const MODEL_COLOR: &str = "\x1b[33m";
const COLOR_OFF: &str = "\x1b[39m";
const ELLIPSIS: char = '…';
const GAP: &str = "  ";
const CLAUDE_PREFIX: &str = "claude-";
const IDLE_STATUS: &str = "completed";
const IDLE_LABEL: &str = "idle";

#[derive(Default, Deserialize)]
#[serde(default)]
struct Rows {
    columns: usize,
    tasks: Vec<Task>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Task {
    id: String,
    name: Option<String>,
    agent_type: Option<String>,
    description: String,
    label: Option<String>,
    status: Option<String>,
    start_time: Option<u64>,
    model: Option<String>,
    effort: Option<Value>,
    token_count: u64,
}

fn model_label(model: &str) -> String {
    let (id, suffix) = match model.find('[') {
        Some(at) => (&model[..at], &model[at..]),
        None => (model, ""),
    };
    let Some(rest) = id.strip_prefix(CLAUDE_PREFIX) else {
        return model.to_owned();
    };
    let mut words = Vec::new();
    let mut version = Vec::new();
    for part in rest.split('-') {
        if part.is_empty() || (part.len() == 8 && part.chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        if part.chars().all(|c| c.is_ascii_digit()) {
            version.push(part);
        } else {
            let mut chars = part.chars();
            let first = chars.next().map(|c| c.to_ascii_uppercase());
            words.push(first.into_iter().chain(chars).collect::<String>());
        }
    }
    if words.is_empty() {
        return model.to_owned();
    }
    let mut label = words.join(" ");
    if !version.is_empty() {
        label.push(' ');
        label.push_str(&version.join("."));
    }
    label.push_str(suffix);
    label
}

fn effort_label(effort: &Value) -> Option<String> {
    match effort {
        Value::String(level) if !level.is_empty() => Some(level.clone()),
        Value::Number(budget) => Some(budget.to_string()),
        _ => None,
    }
}

fn tokens_label(count: u64) -> String {
    if count < 1000 {
        return format!("{count} tokens");
    }
    format!("{:.1}k tokens", count as f64 / 1000.0)
}

fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut cut: String = text.chars().take(width - 1).collect();
    cut.push(ELLIPSIS);
    cut
}

fn plain(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn filled(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|text| !text.is_empty())
}

fn row(task: &Task, columns: usize, now_ms: u64) -> String {
    let name = plain(
        filled(task.name.as_deref())
            .or(filled(Some(&task.description)))
            .or(filled(task.agent_type.as_deref()))
            .unwrap_or("agent"),
    );
    let mut head = name.clone();
    let mut head_width = name.chars().count();
    if let Some(model) = task.model.as_deref().filter(|model| !model.is_empty()) {
        let label = plain(&model_label(model));
        head_width += GAP.len() + label.chars().count();
        head.push_str(&format!("{GAP}{MODEL_COLOR}{label}{COLOR_OFF}"));
        if let Some(effort) = task.effort.as_ref().and_then(effort_label) {
            let effort = plain(&effort);
            head_width += 1 + effort.chars().count();
            head.push(' ');
            head.push_str(&effort);
        }
    }
    let summary = [
        filled(task.label.as_deref()),
        filled(Some(&task.description)),
    ]
    .into_iter()
    .flatten()
    .map(plain)
    .find(|summary| summary.trim() != name.trim())
    .unwrap_or_default();
    let summary = summary.trim();
    let mut tail = tokens_label(task.token_count);
    if let Some(start) = task.start_time {
        tail = format!(
            "{} · {tail}",
            elapsed_label(now_ms.saturating_sub(start) / 1000)
        );
    }
    if task.status.as_deref() == Some(IDLE_STATUS) {
        tail = format!("{IDLE_LABEL} · {tail}");
    }
    let tail_width = tail.chars().count();
    let room = columns.saturating_sub(head_width + GAP.len() * 2 + tail_width);
    if columns == 0 || room == 0 {
        return format!("{head}{GAP}{summary}").trim_end().to_owned();
    }
    let summary = truncate(summary, room);
    let pad = room - summary.chars().count() + GAP.len();
    format!("{head}{GAP}{summary}{}{tail}", " ".repeat(pad))
}

fn render(raw: &str, now_ms: u64) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows: Rows =
        serde_json::from_str(raw).context("the agent rows payload is not valid JSON")?;
    Ok(rows
        .tasks
        .iter()
        .filter(|task| !task.id.is_empty())
        .map(|task| json!({"id": task.id, "content": row(task, rows.columns, now_ms)}).to_string())
        .collect())
}

pub(super) fn run() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("failed to read the agent rows payload")?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default();
    for line in render(&raw, now_ms)? {
        crate::cli::line(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    fn content(raw: Value) -> Vec<String> {
        render(&raw.to_string(), NOW)
            .unwrap()
            .into_iter()
            .map(|line| {
                let line: Value = serde_json::from_str(&line).unwrap();
                line["content"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    #[test]
    fn model_ids_read_as_family_and_version() {
        for (id, label) in [
            ("claude-opus-5-5", "Opus 5.5"),
            ("claude-sonnet-5-5", "Sonnet 5.5"),
            ("claude-haiku-4-5-20251001", "Haiku 4.5"),
            ("claude-3-5-sonnet-20241022", "Sonnet 3.5"),
            ("claude-opus-4-6[1m]", "Opus 4.6[1m]"),
            ("claude-fable", "Fable"),
            ("gpt-5", "gpt-5"),
            ("claude-", "claude-"),
        ] {
            assert_eq!(model_label(id), label, "{id}");
        }
    }

    #[test]
    fn a_row_names_the_agent_its_model_and_effort_before_the_summary() {
        let rows = content(json!({
            "columns": 100,
            "tasks": [{
                "id": "a1",
                "agentType": "project-fork",
                "description": "RTX 3080 limits research",
                "label": "Checking gpu-unlock RTX 3080 support",
                "startTime": NOW - 125_000,
                "model": "claude-opus-5-5",
                "effort": "high",
                "tokenCount": 31_234,
            }],
        }));
        let row = &rows[0];
        assert!(
            row.starts_with(
                "RTX 3080 limits research  \x1b[33mOpus 5.5\x1b[39m high  Checking gpu-unlock RTX 3080 support"
            ),
            "{row:?}"
        );
        assert!(row.ends_with("  2m 5s · 31.2k tokens"), "{row:?}");
        let visible = row.replace(MODEL_COLOR, "").replace(COLOR_OFF, "");
        assert_eq!(visible.chars().count(), 100, "{visible:?}");
    }

    #[test]
    fn a_narrow_row_cuts_the_summary_and_keeps_the_model() {
        let rows = content(json!({
            "columns": 60,
            "tasks": [{
                "id": "a1",
                "name": "fork-a",
                "description": "A very long description of the task at hand here",
                "model": "claude-sonnet-5-5",
                "effort": 4096,
                "tokenCount": 512,
            }],
        }));
        let visible = rows[0].replace(MODEL_COLOR, "").replace(COLOR_OFF, "");
        assert!(
            visible.starts_with("fork-a  Sonnet 5.5 4096  A very"),
            "{visible:?}"
        );
        assert!(visible.contains('…'), "{visible:?}");
        assert!(visible.ends_with("  512 tokens"), "{visible:?}");
        assert_eq!(visible.chars().count(), 60, "{visible:?}");
    }

    #[test]
    fn a_row_without_a_resolved_model_or_width_still_renders_as_plain_text() {
        let rows = content(json!({
            "tasks": [
                {"id": "a1", "agentType": "project-fork", "description": "Start\x1b[31ming\n", "label": " "},
                {"id": "", "description": "dropped"},
            ],
        }));
        assert_eq!(rows, ["Start [31ming"]);
    }

    #[test]
    fn an_unnamed_fork_is_named_by_its_description_and_an_idle_one_says_so() {
        let rows = content(json!({
            "columns": 50,
            "tasks": [
                {"id": "a1", "agentType": "project-fork", "description": "kcd2-design", "label": "kcd2-design", "status": "completed"},
                {"id": "a2", "agentType": "project-fork", "description": "forks-persist", "label": "Probing fork resume", "status": "running"},
                {"id": "a3", "agentType": "project-fork", "description": " "},
            ],
        }));
        let squeezed: Vec<String> = rows
            .iter()
            .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        assert_eq!(
            squeezed,
            [
                "kcd2-design idle · 0 tokens",
                "forks-persist Probing fork resume 0 tokens",
                "project-fork 0 tokens",
            ]
        );
        for row in &rows {
            assert_eq!(row.chars().count(), 50, "{row:?}");
        }
    }

    #[test]
    fn a_model_or_effort_carrying_escapes_reaches_the_row_as_plain_text() {
        let rows = content(json!({
            "tasks": [{
                "id": "a1",
                "name": "fork-a",
                "description": "work",
                "model": "evil\x1b[2J",
                "effort": "hi\x1b]0;x\x07gh",
            }],
        }));
        assert_eq!(rows, ["fork-a  \x1b[33mevil [2J\x1b[39m hi ]0;x gh  work"]);
    }

    #[test]
    fn an_empty_payload_writes_nothing_and_bad_json_fails() {
        assert!(render("  ", NOW).unwrap().is_empty());
        assert!(render("{", NOW).is_err());
    }
}
