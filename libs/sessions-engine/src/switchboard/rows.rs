use std::io::Read;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::fork_state::{self, ForkState};

const MODEL_COLOR: &str = "\x1b[33m";
const COLOR_OFF: &str = "\x1b[39m";
const WORKING_COLOR: &str = "\x1b[32m";
const PARKED_COLOR: &str = "\x1b[34m";
const YOUR_TURN_COLOR: &str = "\x1b[1;33m";
const BOLD_OFF: &str = "\x1b[22m";
const WORKING_LABEL: &str = "working";
const PARKED_LABEL: &str = "parked";
const YOUR_TURN_LABEL: &str = "your turn";
const IDLE_LABEL: &str = "idle";
const DOT: &str = " · ";
const ELLIPSIS: char = '…';
const GAP: &str = "  ";
const CLAUDE_PREFIX: &str = "claude-";
const KEY_SEPARATOR: &str = ": ";
const KEY_NAME_LIMIT: usize = 64;
const NAME_SHARE: usize = 3;
const NAME_MIN: usize = 12;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Rows {
    columns: usize,
    transcript_path: Option<String>,
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

fn is_key_name(text: &str) -> bool {
    text.len() <= KEY_NAME_LIMIT
        && text.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn key_and_friendly(description: &str) -> (Option<&str>, Option<&str>) {
    let description = description.trim();
    if let Some((key, friendly)) = description.split_once(KEY_SEPARATOR) {
        if is_key_name(key.trim()) {
            return (Some(key.trim()), filled(Some(friendly)));
        }
    }
    if description.contains('-') && is_key_name(description) {
        return (Some(description), None);
    }
    (None, filled(Some(description)))
}

struct Drawn {
    text: String,
    width: usize,
}

fn head(name: &str, task: &Task) -> Drawn {
    let mut text = name.to_owned();
    let mut width = name.chars().count();
    if let Some(model) = task.model.as_deref().filter(|model| !model.is_empty()) {
        let label = plain(&model_label(model));
        width += GAP.len() + label.chars().count();
        text.push_str(&format!("{GAP}{MODEL_COLOR}{label}{COLOR_OFF}"));
        if let Some(effort) = task.effort.as_ref().and_then(effort_label) {
            let effort = plain(&effort);
            width += 1 + effort.chars().count();
            text.push(' ');
            text.push_str(&effort);
        }
    }
    Drawn { text, width }
}

fn state_label(state: &ForkState) -> Drawn {
    let (word, color) = match state {
        ForkState::Working => (WORKING_LABEL.to_owned(), Some(WORKING_COLOR)),
        ForkState::Parked(prs) => {
            let numbers = prs
                .iter()
                .filter_map(|pr| pr.rsplit('/').next())
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>();
            let word = if numbers.is_empty() {
                PARKED_LABEL.to_owned()
            } else {
                format!("{PARKED_LABEL} {}", numbers.join(","))
            };
            (word, Some(PARKED_COLOR))
        }
        ForkState::YourTurn => (YOUR_TURN_LABEL.to_owned(), Some(YOUR_TURN_COLOR)),
        ForkState::Idle => (IDLE_LABEL.to_owned(), None),
    };
    let width = word.chars().count();
    let text = match color {
        Some(color) => format!("{color}{word}{BOLD_OFF}{COLOR_OFF}"),
        None => word,
    };
    Drawn { text, width }
}

fn row(task: &Task, state: Option<&ForkState>, columns: usize, now_ms: u64) -> String {
    let (key, friendly) = key_and_friendly(&task.description);
    let name = plain(
        filled(task.name.as_deref())
            .or(key)
            .or(friendly)
            .or(filled(task.agent_type.as_deref()))
            .unwrap_or("agent"),
    );
    let progress = filled(task.label.as_deref())
        .filter(|label| *label != task.description.trim() && Some(*label) != friendly);
    let summary = [
        friendly.map(plain).filter(|friendly| *friendly != name),
        progress.map(plain),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let summary = summary.trim();
    let mut tail = tokens_label(task.token_count);
    if let Some(start) = task.start_time {
        tail = format!(
            "{}{DOT}{tail}",
            elapsed_label(now_ms.saturating_sub(start) / 1000)
        );
    }
    let label = state.map(state_label);
    if columns == 0 {
        let shown = head(&name, task);
        let parts = [
            Some(summary),
            label.as_ref().map(|label| label.text.as_str()),
        ];
        let rest = parts
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(GAP);
        return format!("{}{GAP}{rest}", shown.text).trim_end().to_owned();
    }
    let mut tail_width = tail.chars().count();
    if let Some(label) = &label {
        tail = format!("{}{DOT}{tail}", label.text);
        tail_width += label.width + DOT.chars().count();
    }
    let capped = truncate(&name, (columns / NAME_SHARE).max(NAME_MIN));
    let shown = head(&capped, task);
    let room = columns.saturating_sub(shown.width + GAP.len() * 2 + tail_width);
    if room == 0 {
        let others = shown.width - capped.chars().count();
        let fits = columns.saturating_sub(others + GAP.len() + tail_width);
        let shown = head(&truncate(&name, fits), task);
        let pad = columns.saturating_sub(shown.width + tail_width).max(1);
        return format!("{}{}{tail}", shown.text, " ".repeat(pad));
    }
    let summary = truncate(summary, room);
    let pad = room - summary.chars().count() + GAP.len();
    format!("{}{GAP}{summary}{}{tail}", shown.text, " ".repeat(pad))
}

fn render(raw: &str, now_ms: u64) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows: Rows =
        serde_json::from_str(raw).context("the agent rows payload is not valid JSON")?;
    let conversation = rows.transcript_path.as_deref().map(Path::new);
    Ok(rows
        .tasks
        .iter()
        .filter(|task| !task.id.is_empty())
        .map(|task| {
            let transcript = conversation
                .and_then(|conversation| fork_state::transcript_of(conversation, &task.id));
            let state = fork_state::state(task.status.as_deref(), transcript.as_deref());
            let content = row(task, state.as_ref(), rows.columns, now_ms);
            json!({"id": task.id, "content": content}).to_string()
        })
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

    fn visible(row: &str) -> String {
        let mut out = String::new();
        let mut chars = row.chars();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        }
        out
    }

    fn squeeze(rows: &[String]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                visible(row)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

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
            "columns": 60,
            "tasks": [
                {"id": "a1", "agentType": "project-fork", "description": "kcd2-design", "label": "kcd2-design", "status": "completed"},
                {"id": "a2", "agentType": "project-fork", "description": "forks-persist", "label": "Probing fork resume", "status": "running"},
                {"id": "a3", "agentType": "project-fork", "description": " "},
            ],
        }));
        assert_eq!(
            squeeze(&rows),
            [
                "kcd2-design idle · 0 tokens",
                "forks-persist Probing fork resume working · 0 tokens",
                "project-fork 0 tokens",
            ]
        );
        for row in &rows {
            assert_eq!(visible(row).chars().count(), 60, "{row:?}");
        }
    }

    #[test]
    fn a_key_name_description_shows_the_key_then_the_friendly_words_and_the_progress() {
        let rows = content(json!({
            "columns": 100,
            "tasks": [
                {"id": "a1", "description": "session-lookup-bug: Blocket session lookup", "label": "Creating pull request with gh", "status": "running"},
                {"id": "a2", "description": "kcd2-design: KCD2 light-steer poses", "label": "kcd2-design: KCD2 light-steer poses", "status": "completed"},
                {"id": "a3", "description": "Fix qol shot open latency", "label": "Fix qol shot open latency"},
                {"id": "a4", "description": "Odd: Words Here", "label": "Odd: Words Here"},
            ],
        }));
        assert_eq!(
            squeeze(&rows),
            [
                "session-lookup-bug Blocket session lookup · Creating pull request with gh working · 0 tokens",
                "kcd2-design KCD2 light-steer poses idle · 0 tokens",
                "Fix qol shot open latency 0 tokens",
                "Odd: Words Here 0 tokens",
            ]
        );
    }

    #[test]
    fn a_long_name_with_a_model_still_fits_the_pane_and_keeps_the_idle_marker() {
        let rows = content(json!({
            "columns": 50,
            "tasks": [{
                "id": "a1",
                "description": "Investigate why nightly sync stops after midnight and fix it",
                "model": "claude-opus-5-5",
                "effort": "high",
                "status": "completed",
            }],
        }));
        let visible = rows[0].replace(MODEL_COLOR, "").replace(COLOR_OFF, "");
        assert_eq!(visible.chars().count(), 50, "{visible:?}");
        assert!(visible.starts_with("Investigate why"), "{visible:?}");
        assert!(visible.contains("…  Opus 5.5 high"), "{visible:?}");
        assert!(visible.ends_with("  idle · 0 tokens"), "{visible:?}");
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
    fn each_fork_shows_its_cli_sessions_state_from_its_status_and_what_it_left_running() {
        use super::super::background::fixtures::waiting;
        use super::super::background::HOLD_COMMAND;
        let root = tempfile::tempdir().unwrap();
        let conversation = root.path().join("c0ab.jsonl");
        let subagents = root.path().join("c0ab").join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let watch = "node /x/bin/pr-watch.cjs https://github.com/qol-tools/qol/pull/407 --pretty";
        for (id, transcript) in [
            ("aturn", waiting("bhold", HOLD_COMMAND)),
            (
                "apark",
                [waiting("bhold", HOLD_COMMAND), waiting("bwatch", watch)].concat(),
            ),
            ("adone", String::new()),
            ("awork", waiting("bwatch", watch)),
        ] {
            std::fs::write(subagents.join(format!("agent-{id}.jsonl")), transcript).unwrap();
        }
        let task = |id: &str, status: &str| json!({"id": id, "description": format!("fork-{id}"), "status": status});
        let payload = |columns: usize| {
            json!({
                "columns": columns,
                "transcript_path": conversation.to_str().unwrap(),
                "tasks": [
                    task("aturn", "completed"),
                    task("apark", "completed"),
                    task("adone", "completed"),
                    task("awork", "running"),
                    task("amiss", "completed"),
                    task("akill", "killed"),
                ],
            })
        };
        let rows = content(payload(40));
        assert_eq!(
            squeeze(&rows),
            [
                "fork-aturn your turn · 0 tokens",
                "fork-apark parked #407 · 0 tokens",
                "fork-adone idle · 0 tokens",
                "fork-awork working · 0 tokens",
                "fork-amiss idle · 0 tokens",
                "fork-akill 0 tokens",
            ]
        );
        for row in &rows {
            assert_eq!(visible(row).chars().count(), 40, "{row:?}");
        }
        assert!(
            rows[0].ends_with("\x1b[1;33myour turn\x1b[22m\x1b[39m · 0 tokens"),
            "{:?}",
            rows[0]
        );
        assert!(
            rows[1].contains("\x1b[34mparked #407\x1b[22m\x1b[39m"),
            "{:?}",
            rows[1]
        );
        assert!(
            rows[3].contains("\x1b[32mworking\x1b[22m\x1b[39m"),
            "{:?}",
            rows[3]
        );
        assert_eq!(
            content(payload(0))
                .iter()
                .map(String::as_str)
                .map(visible)
                .collect::<Vec<_>>(),
            [
                "fork-aturn  your turn",
                "fork-apark  parked #407",
                "fork-adone  idle",
                "fork-awork  working",
                "fork-amiss  idle",
                "fork-akill",
            ]
        );
    }

    #[test]
    fn an_empty_payload_writes_nothing_and_bad_json_fails() {
        assert!(render("  ", NOW).unwrap().is_empty());
        assert!(render("{", NOW).is_err());
    }
}
