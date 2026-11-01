use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

const MODEL_COLOR: &str = "\x1b[33m";
const COLOR_OFF: &str = "\x1b[39m";
const ELLIPSIS: char = '…';
const GAP: &str = "  ";
const TAIL_SEPARATOR: &str = " · ";
const CLAUDE_PREFIX: &str = "claude-";
const IDLE_STATUS: &str = "completed";
const IDLE_LABEL: &str = "idle";
const KEY_SEPARATOR: &str = ": ";
const KEY_NAME_LIMIT: usize = 64;
const NAME_SHARE: usize = 3;
const NAME_MIN: usize = 12;
const SUMMARY_MIN: usize = 4;
const STARTED_FORMAT: &str = "%b %-d %H:%M";
const SUBAGENTS_DIR: &str = "subagents";
const AGENT_PREFIX: &str = "agent-";
const TRANSCRIPT_SUFFIX: &str = ".jsonl";
const FIRST_ROW_LIMIT: u64 = 256 * 1024;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Rows {
    columns: Option<usize>,
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

struct Clock<'a> {
    now_ms: u64,
    first_row_ms: &'a dyn Fn(&str) -> Option<u64>,
    started_label: &'a dyn Fn(u64) -> Option<String>,
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

fn count_label(count: u64) -> String {
    if count < 1000 {
        return count.to_string();
    }
    format!("{:.1}k", count as f64 / 1000.0)
}

fn tokens_label(count: u64) -> String {
    format!("{} tokens", count_label(count))
}

fn elapsed_label(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn local_started_label(ms: u64) -> Option<String> {
    let ms = i64::try_from(ms).ok()?;
    let time = chrono::DateTime::from_timestamp_millis(ms)?.with_timezone(&chrono::Local);
    Some(time.format(STARTED_FORMAT).to_string())
}

fn width(text: &str) -> usize {
    text.chars().count()
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeadFit {
    Full,
    ModelOnly,
    NameOnly,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TailFit {
    Full,
    NoElapsed,
    ShortTokens,
    NoTokens,
}

const LADDER: [(HeadFit, TailFit); 6] = [
    (HeadFit::Full, TailFit::Full),
    (HeadFit::Full, TailFit::NoElapsed),
    (HeadFit::ModelOnly, TailFit::NoElapsed),
    (HeadFit::NameOnly, TailFit::NoElapsed),
    (HeadFit::NameOnly, TailFit::ShortTokens),
    (HeadFit::NameOnly, TailFit::NoTokens),
];

struct Head {
    text: String,
    width: usize,
}

fn head(name: &str, task: &Task, fit: HeadFit) -> Head {
    let mut text = name.to_owned();
    let mut width = width(name);
    if fit == HeadFit::NameOnly {
        return Head { text, width };
    }
    if let Some(model) = task.model.as_deref().filter(|model| !model.is_empty()) {
        let label = plain(&model_label(model));
        width += GAP.len() + label.chars().count();
        text.push_str(&format!("{GAP}{MODEL_COLOR}{label}{COLOR_OFF}"));
        if fit == HeadFit::Full {
            if let Some(effort) = task.effort.as_ref().and_then(effort_label) {
                let effort = plain(&effort);
                width += 1 + effort.chars().count();
                text.push(' ');
                text.push_str(&effort);
            }
        }
    }
    Head { text, width }
}

struct Tail {
    idle: bool,
    started: Option<String>,
    elapsed: Option<String>,
    tokens: u64,
}

impl Tail {
    fn text(&self, fit: TailFit) -> String {
        let mut parts = Vec::new();
        if self.idle {
            parts.push(IDLE_LABEL.to_owned());
        }
        parts.extend(self.started.clone());
        if fit == TailFit::Full {
            parts.extend(self.elapsed.clone());
        }
        match fit {
            TailFit::Full | TailFit::NoElapsed => parts.push(tokens_label(self.tokens)),
            TailFit::ShortTokens => parts.push(count_label(self.tokens)),
            TailFit::NoTokens => {}
        }
        parts.join(TAIL_SEPARATOR)
    }
}

fn fitted(name: &str, summary: &str, task: &Task, tail: &Tail, columns: usize) -> String {
    let name_floor = width(name).min(NAME_MIN);
    for (head_fit, tail_fit) in LADDER {
        let tail = tail.text(tail_fit);
        let fixed = head("", task, head_fit).width + GAP.len() + width(&tail);
        if name_floor + fixed > columns {
            continue;
        }
        if head_fit == HeadFit::Full && tail_fit == TailFit::Full && !summary.is_empty() {
            let capped = head(
                &truncate(
                    name,
                    (columns / NAME_SHARE).max(NAME_MIN).min(columns - fixed),
                ),
                task,
                head_fit,
            );
            let room = columns.saturating_sub(capped.width + GAP.len() * 2 + width(&tail));
            if room > 0 && room >= SUMMARY_MIN.min(width(summary)) {
                let summary = truncate(summary, room);
                let pad = room - width(&summary) + GAP.len();
                return format!("{}{GAP}{summary}{}{tail}", capped.text, " ".repeat(pad));
            }
        }
        let shown = head(&truncate(name, columns - fixed), task, head_fit);
        let pad = columns - shown.width - width(&tail);
        return format!("{}{}{tail}", shown.text, " ".repeat(pad));
    }
    let tail = tail.text(TailFit::NoTokens);
    if tail.is_empty() || width(&tail) + 2 > columns {
        return truncate(&format!("{name} {tail}").trim_end().to_owned(), columns);
    }
    let name = truncate(name, columns - width(&tail) - 1);
    let pad = columns - width(&name) - width(&tail);
    format!("{name}{}{tail}", " ".repeat(pad))
}

fn row(task: &Task, columns: Option<usize>, clock: &Clock) -> Option<String> {
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
    let started = (clock.first_row_ms)(&task.id).or(task.start_time);
    let tail = Tail {
        idle: task.status.as_deref() == Some(IDLE_STATUS),
        started: started
            .and_then(clock.started_label)
            .map(|label| plain(&label)),
        elapsed: started.map(|start| elapsed_label(clock.now_ms.saturating_sub(start) / 1000)),
        tokens: task.token_count,
    };
    match columns {
        Some(0) => None,
        Some(columns) => Some(fitted(&name, summary, task, &tail, columns)),
        None => {
            let shown = head(&name, task, HeadFit::Full);
            let line = [shown.text.as_str(), summary, &tail.text(TailFit::Full)]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(GAP);
            Some(line)
        }
    }
}

fn agent_transcript(session_transcript: &Path, id: &str) -> PathBuf {
    session_transcript
        .with_extension("")
        .join(SUBAGENTS_DIR)
        .join(format!("{AGENT_PREFIX}{id}{TRANSCRIPT_SUFFIX}"))
}

fn first_row_ms(transcript: &Path) -> Option<u64> {
    let file = File::open(transcript).ok()?;
    let mut line = String::new();
    BufReader::new(file.take(FIRST_ROW_LIMIT))
        .read_line(&mut line)
        .ok()?;
    let row: Value = serde_json::from_str(&line).ok()?;
    let stamp = chrono::DateTime::parse_from_rfc3339(row.get("timestamp")?.as_str()?).ok()?;
    u64::try_from(stamp.timestamp_millis()).ok()
}

fn render(raw: &str, clock: &Clock) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let rows: Rows =
        serde_json::from_str(raw).context("the agent rows payload is not valid JSON")?;
    Ok(rows
        .tasks
        .iter()
        .filter(|task| !task.id.is_empty())
        .filter_map(|task| {
            let content = row(task, rows.columns, clock)?;
            Some(json!({"id": task.id, "content": content}).to_string())
        })
        .collect())
}

fn session_transcript(raw: &str) -> Option<PathBuf> {
    let rows: Rows = serde_json::from_str(raw).ok()?;
    filled(rows.transcript_path.as_deref()).map(PathBuf::from)
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
    let session = session_transcript(&raw);
    let first_row = |id: &str| {
        session
            .as_deref()
            .and_then(|session| first_row_ms(&agent_transcript(session, id)))
    };
    let clock = Clock {
        now_ms,
        first_row_ms: &first_row,
        started_label: &local_started_label,
    };
    for line in render(&raw, &clock)? {
        crate::cli::line(line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;
    const STARTED: &str = "Nov 1 07:58";

    fn no_transcript(_id: &str) -> Option<u64> {
        None
    }

    fn fixed_label(_ms: u64) -> Option<String> {
        Some(STARTED.to_owned())
    }

    fn content(raw: Value) -> Vec<String> {
        let clock = Clock {
            now_ms: NOW,
            first_row_ms: &no_transcript,
            started_label: &fixed_label,
        };
        render(&raw.to_string(), &clock)
            .unwrap()
            .into_iter()
            .map(|line| {
                let line: Value = serde_json::from_str(&line).unwrap();
                line["content"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    fn visible(row: &str) -> String {
        row.replace(MODEL_COLOR, "").replace(COLOR_OFF, "")
    }

    fn squeezed(rows: &[String]) -> Vec<String> {
        rows.iter()
            .map(|row| row.split_whitespace().collect::<Vec<_>>().join(" "))
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
            "columns": 120,
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
        assert!(
            row.ends_with(&format!("  {STARTED} · 2m 5s · 31.2k tokens")),
            "{row:?}"
        );
        assert_eq!(visible(row).chars().count(), 120, "{row:?}");
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
        let visible = visible(&rows[0]);
        assert!(
            visible.starts_with("fork-a  Sonnet 5.5 4096  A very"),
            "{visible:?}"
        );
        assert!(visible.contains('…'), "{visible:?}");
        assert!(visible.ends_with("  512 tokens"), "{visible:?}");
        assert_eq!(visible.chars().count(), 60, "{visible:?}");
    }

    #[test]
    fn the_start_date_shows_on_every_row_even_idle_and_without_a_width() {
        let task = json!({
            "id": "a1",
            "name": "fork-a",
            "description": "work",
            "status": "completed",
            "startTime": NOW - 5_000,
            "tokenCount": 10,
        });
        let wide = content(json!({"columns": 80, "tasks": [task.clone()]}));
        assert!(
            wide[0].ends_with(&format!("  idle · {STARTED} · 5s · 10 tokens")),
            "{wide:?}"
        );
        let unbounded = content(json!({"tasks": [task]}));
        assert_eq!(
            unbounded,
            [format!("fork-a  work  idle · {STARTED} · 5s · 10 tokens")]
        );
    }

    #[test]
    fn the_transcript_start_outranks_the_run_start() {
        let first_row = |id: &str| (id == "a1").then_some(NOW - 3_600_000);
        let label = |ms: u64| Some(format!("at {ms}"));
        let clock = Clock {
            now_ms: NOW,
            first_row_ms: &first_row,
            started_label: &label,
        };
        let raw = json!({
            "tasks": [
                {"id": "a1", "name": "old", "startTime": NOW - 1_000},
                {"id": "a2", "name": "new", "startTime": NOW - 1_000},
            ],
        });
        let rows = render(&raw.to_string(), &clock).unwrap();
        assert!(
            rows[0].contains(&format!("at {} · 1h 0m", NOW - 3_600_000)),
            "{rows:?}"
        );
        assert!(
            rows[1].contains(&format!("at {} · 1s", NOW - 1_000)),
            "{rows:?}"
        );
    }

    #[test]
    fn no_row_is_ever_wider_than_its_columns() {
        let task = json!({
            "id": "a1",
            "description": "session-lookup-bug: Blocket session lookup",
            "label": "Creating pull request with gh",
            "status": "completed",
            "startTime": NOW - 7_300_000,
            "model": "claude-opus-5-5",
            "effort": "high",
            "tokenCount": 153_900,
        });
        for columns in 1..=140 {
            let rows = content(json!({"columns": columns, "tasks": [task.clone()]}));
            let shown = visible(&rows[0]);
            assert!(
                shown.chars().count() <= columns,
                "{columns}: {shown:?} is {} wide",
                shown.chars().count()
            );
            if columns >= 40 {
                assert_eq!(shown.chars().count(), columns, "{columns}: {shown:?}");
                assert!(shown.contains(STARTED), "{columns}: {shown:?}");
                assert!(shown.contains(IDLE_LABEL), "{columns}: {shown:?}");
            }
        }
    }

    #[test]
    fn a_shrinking_row_drops_the_summary_effort_model_elapsed_and_tokens_in_turn() {
        let task = json!({
            "id": "a1",
            "description": "kcd2-modding: continue KCD2 modding",
            "startTime": NOW - 25_000,
            "model": "claude-opus-5-5",
            "effort": "high",
            "tokenCount": 53_900,
        });
        let at = |columns: usize| {
            let rows = content(json!({"columns": columns, "tasks": [task.clone()]}));
            squeezed(&[visible(&rows[0])])[0].clone()
        };
        assert_eq!(
            at(100),
            format!(
                "kcd2-modding Opus 5.5 high continue KCD2 modding {STARTED} · 25s · 53.9k tokens"
            )
        );
        assert_eq!(
            at(64),
            format!("kcd2-modding Opus 5.5 high {STARTED} · 25s · 53.9k tokens")
        );
        assert_eq!(
            at(56),
            format!("kcd2-modding Opus 5.5 high {STARTED} · 53.9k tokens")
        );
        assert_eq!(
            at(52),
            format!("kcd2-modding Opus 5.5 {STARTED} · 53.9k tokens")
        );
        assert_eq!(at(45), format!("kcd2-modding {STARTED} · 53.9k tokens"));
        assert_eq!(at(36), format!("kcd2-modding {STARTED} · 53.9k"));
        assert_eq!(at(28), format!("kcd2-modding {STARTED}"));
        assert_eq!(at(18), format!("kcd2-… {STARTED}"));
        assert_eq!(at(8), "kcd2-mo…");
    }

    #[test]
    fn a_zero_width_pane_leaves_the_row_to_claude_code() {
        let rows = content(json!({
            "columns": 0,
            "tasks": [{"id": "a1", "name": "fork-a", "description": "work"}],
        }));
        assert!(rows.is_empty(), "{rows:?}");
    }

    #[test]
    fn a_row_without_a_resolved_model_or_width_still_renders_as_plain_text() {
        let rows = content(json!({
            "tasks": [
                {"id": "a1", "agentType": "project-fork", "description": "Start\x1b[31ming\n", "label": " "},
                {"id": "", "description": "dropped"},
            ],
        }));
        assert_eq!(rows, ["Start [31ming  0 tokens"]);
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
        assert_eq!(
            squeezed(&rows),
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
    fn a_key_name_description_shows_the_key_then_the_friendly_words_and_the_progress() {
        let rows = content(json!({
            "columns": 90,
            "tasks": [
                {"id": "a1", "description": "session-lookup-bug: Blocket session lookup", "label": "Creating pull request with gh", "status": "running"},
                {"id": "a2", "description": "kcd2-design: KCD2 light-steer poses", "label": "kcd2-design: KCD2 light-steer poses", "status": "completed"},
                {"id": "a3", "description": "Fix qol shot open latency", "label": "Fix qol shot open latency"},
                {"id": "a4", "description": "Odd: Words Here", "label": "Odd: Words Here"},
            ],
        }));
        assert_eq!(
            squeezed(&rows),
            [
                "session-lookup-bug Blocket session lookup · Creating pull request with gh 0 tokens",
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
        let visible = visible(&rows[0]);
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
        assert_eq!(
            rows,
            ["fork-a  \x1b[33mevil [2J\x1b[39m hi ]0;x gh  work  0 tokens"]
        );
    }

    #[test]
    fn an_empty_payload_writes_nothing_and_bad_json_fails() {
        let clock = Clock {
            now_ms: NOW,
            first_row_ms: &no_transcript,
            started_label: &fixed_label,
        };
        assert!(render("  ", &clock).unwrap().is_empty());
        assert!(render("{", &clock).is_err());
    }

    #[test]
    fn the_agent_transcript_sits_beside_the_session_transcript() {
        assert_eq!(
            agent_transcript(Path::new("/p/conv-1.jsonl"), "a9"),
            Path::new("/p/conv-1/subagents/agent-a9.jsonl")
        );
        assert_eq!(
            session_transcript(r#"{"transcript_path":"/p/c.jsonl"}"#),
            Some(PathBuf::from("/p/c.jsonl"))
        );
        assert_eq!(session_transcript(r#"{"transcript_path":" "}"#), None);
    }

    #[test]
    fn the_first_row_timestamp_is_read_and_anything_else_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.jsonl");
        std::fs::write(
            &good,
            "{\"type\":\"user\",\"timestamp\":\"2026-11-01T06:58:29.835Z\"}\n{\"type\":\"assistant\"}\n",
        )
        .unwrap();
        assert_eq!(first_row_ms(&good), Some(1_793_516_309_835));
        let bad = dir.path().join("bad.jsonl");
        std::fs::write(&bad, "{\"type\":\"user\"}\n").unwrap();
        assert_eq!(first_row_ms(&bad), None);
        assert_eq!(first_row_ms(&dir.path().join("missing.jsonl")), None);
    }

    #[test]
    fn the_local_start_label_reads_month_day_and_clock() {
        let label = local_started_label(1_793_516_309_835).unwrap();
        let parts: Vec<&str> = label.split(' ').collect();
        assert_eq!(parts.len(), 3, "{label}");
        assert_eq!(parts[0].len(), 3, "{label}");
        assert!(parts[1].parse::<u8>().is_ok(), "{label}");
        assert_eq!(parts[2].len(), 5, "{label}");
        assert_eq!(&parts[2][2..3], ":", "{label}");
    }
}
