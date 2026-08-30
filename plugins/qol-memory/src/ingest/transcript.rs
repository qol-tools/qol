use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::{json, Value};

use crate::ingest::redact::redact;
use crate::ingest::{unit_key, ASSISTANT_KIND, COMPACTION_KIND};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseCursor {
    pub offset: u64,
    pub session: Option<String>,
    pub cwd: Option<String>,
}

pub struct Parsed {
    pub units: Vec<Value>,
    pub cursor: ParseCursor,
}

struct Origin<'a> {
    source: &'a str,
    agent_home: &'a str,
    file: &'a str,
}

pub fn parse_file(
    path: &Path,
    source: &str,
    agent_home: &str,
    cursor: ParseCursor,
) -> anyhow::Result<Parsed> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(cursor.offset))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    let mut end = data.len();
    if !data.ends_with(b"\n") {
        end = data
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
    }
    let file_name = base_name(path);
    let origin = Origin {
        source,
        agent_home,
        file: &file_name,
    };
    let mut session = cursor.session;
    let mut cwd = cursor.cwd;
    let mut units = Vec::new();
    for line in data[..end].split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        handle_event(&event, &origin, &mut session, &mut cwd, &mut units);
    }
    Ok(Parsed {
        units,
        cursor: ParseCursor {
            offset: cursor.offset + end as u64,
            session,
            cwd,
        },
    })
}

fn handle_event(
    event: &Value,
    origin: &Origin,
    session: &mut Option<String>,
    cwd: &mut Option<String>,
    units: &mut Vec<Value>,
) {
    match event.get("type").and_then(Value::as_str) {
        Some("session") => {
            *session = event.get("id").and_then(Value::as_str).map(str::to_owned);
            *cwd = event.get("cwd").and_then(Value::as_str).map(str::to_owned);
        }
        Some("message") => {
            let message = event.get("message").unwrap_or(&Value::Null);
            let content = message.get("content").unwrap_or(&Value::Null);
            let ts = to_iso(message.get("timestamp"));
            match message.get("role").and_then(Value::as_str) {
                Some("user") => {
                    let text = redact(&text_of(content));
                    units.push(transcript_unit("user", origin, &ts, &text, session, cwd));
                }
                Some("assistant") => {
                    let text = redact(&text_of(content));
                    if text.trim().is_empty() {
                        return;
                    }
                    units.push(transcript_unit(
                        ASSISTANT_KIND,
                        origin,
                        &ts,
                        &text,
                        session,
                        cwd,
                    ));
                }
                _ => {}
            }
        }
        Some("compaction") => {
            let text = redact(event.get("summary").and_then(Value::as_str).unwrap_or(""));
            let ts = to_iso(event.get("timestamp"));
            let details = event.get("details").unwrap_or(&Value::Null);
            units.push(json!({
                "key": unit_key(origin.source, origin.file, ts.as_str(), &text),
                "source": origin.source,
                "agent_home": origin.agent_home,
                "host": crate::host::current(),
                "file": origin.file,
                "session": session.clone(),
                "cwd": cwd.clone(),
                "kind": "compaction",
                "ts": ts,
                "text": text,
                "filesRead": details
                    .get("readFiles")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                "filesModified": details
                    .get("modifiedFiles")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!([]))
            }));
        }
        Some("branch_summary") => {
            let text = redact(event.get("summary").and_then(Value::as_str).unwrap_or(""));
            let ts = to_iso(event.get("timestamp"));
            units.push(json!({
                "key": unit_key(origin.source, origin.file, ts.as_str(), &text),
                "source": origin.source,
                "agent_home": origin.agent_home,
                "host": crate::host::current(),
                "file": origin.file,
                "session": session.clone(),
                "cwd": cwd.clone(),
                "kind": "branch",
                "ts": ts,
                "text": text
            }));
        }
        Some("user") => {
            let message = event.get("message").unwrap_or(&Value::Null);
            let content = if message.is_null() {
                event.get("content").unwrap_or(&Value::Null)
            } else {
                message.get("content").unwrap_or(&Value::Null)
            };
            if is_tool_result(content) {
                return;
            }
            let text = redact(&text_of(content));
            if text.trim().is_empty() {
                return;
            }
            update_context(event, session, cwd);
            let ts = to_iso(event.get("timestamp"));
            let kind = if event.get("isCompactSummary").and_then(Value::as_bool) == Some(true) {
                COMPACTION_KIND
            } else {
                "user"
            };
            units.push(transcript_unit(kind, origin, &ts, &text, session, cwd));
        }
        Some("assistant") => {
            let message = event.get("message").unwrap_or(&Value::Null);
            let content = message.get("content").unwrap_or(&Value::Null);
            let text = redact(&text_of(content));
            if text.trim().is_empty() {
                return;
            }
            update_context(event, session, cwd);
            let ts = to_iso(event.get("timestamp"));
            units.push(transcript_unit(
                ASSISTANT_KIND,
                origin,
                &ts,
                &text,
                session,
                cwd,
            ));
        }
        Some("summary") => {
            let text = redact(event.get("summary").and_then(Value::as_str).unwrap_or(""));
            if text.trim().is_empty() {
                return;
            }
            let ts = to_iso(event.get("timestamp"));
            units.push(json!({
                "key": unit_key(origin.source, origin.file, ts.as_str(), &text),
                "source": origin.source,
                "agent_home": origin.agent_home,
                "host": crate::host::current(),
                "file": origin.file,
                "session": session.clone(),
                "cwd": cwd.clone(),
                "kind": "compaction",
                "ts": ts,
                "text": text
            }));
        }
        _ => {}
    }
}

fn transcript_unit(
    kind: &str,
    origin: &Origin,
    ts: &Value,
    text: &str,
    session: &Option<String>,
    cwd: &Option<String>,
) -> Value {
    json!({
        "key": unit_key(origin.source, origin.file, ts.as_str(), text),
        "source": origin.source,
        "agent_home": origin.agent_home,
        "host": crate::host::current(),
        "file": origin.file,
        "session": session.clone(),
        "cwd": cwd.clone(),
        "kind": kind,
        "ts": ts,
        "text": text
    })
}

fn update_context(event: &Value, session: &mut Option<String>, cwd: &mut Option<String>) {
    if let Some(id) = event
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        *session = Some(id.to_owned());
    }
    if let Some(dir) = event
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|dir| !dir.is_empty())
    {
        *cwd = Some(dir.to_owned());
    }
}

fn text_of(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let Some(blocks) = content.as_array() else {
        return String::new();
    };
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_tool_result(content: &Value) -> bool {
    content.as_array().is_some_and(|blocks| {
        blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

fn to_iso(ts: Option<&Value>) -> Value {
    match ts {
        Some(Value::Number(number)) => {
            json!(iso_from_millis(number.as_f64().unwrap_or(0.0) as i64))
        }
        Some(Value::String(text)) if !text.is_empty() => json!(text.clone()),
        _ => Value::Null,
    }
}

fn iso_from_millis(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let second_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        second_of_day / 3600,
        (second_of_day % 3600) / 60,
        second_of_day % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

fn base_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-transcript-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn iso_from_millis_renders_the_date_iso_shape() {
        assert_eq!(
            iso_from_millis(1_787_819_945_554),
            "2026-08-27T08:39:05.554Z"
        );
        assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn pi_transcript_yields_user_and_compaction_units() {
        let dir = TempDir::new("pi-parse");
        let path = dir.0.join("2026-08-27T08-39-05-000Z.jsonl");
        let body = concat!(
            r#"{"type":"session","id":"sess-1","cwd":"/tmp/proj"}"#,
            "\n",
            r#"{"type":"message","message":{"role":"toolResult","content":"out"},"timestamp":1}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"fix the launcher bug"}],"timestamp":1787819945554}}"#,
            "\n",
            r#"{"type":"compaction","summary":"rolled up session context","timestamp":"2026-08-27T09:00:00.000Z","details":{"readFiles":["a.rs"],"modifiedFiles":["b.rs"]}}"#,
            "\n",
            r#"{"type":"branch_summary","summary":"branch wrap","timestamp":"2026-08-27T09:30:00.000Z"}"#,
            "\n"
        );
        std::fs::write(&path, body).unwrap();
        let parsed =
            parse_file(&path, "pi", "/test-home/.pi/agent", ParseCursor::default()).unwrap();
        assert_eq!(parsed.units.len(), 3);

        let user = &parsed.units[0];
        assert_eq!(user.get("kind").and_then(Value::as_str), Some("user"));
        assert_eq!(
            user.get("ts").and_then(Value::as_str),
            Some("2026-08-27T08:39:05.554Z")
        );
        assert_eq!(user.get("session").and_then(Value::as_str), Some("sess-1"));
        assert_eq!(user.get("cwd").and_then(Value::as_str), Some("/tmp/proj"));
        assert_eq!(
            user.get("file").and_then(Value::as_str),
            Some("2026-08-27T08-39-05-000Z.jsonl")
        );
        assert_eq!(user.get("source").and_then(Value::as_str), Some("pi"));
        assert_eq!(
            user.get("agent_home").and_then(Value::as_str),
            Some("/test-home/.pi/agent")
        );
        assert_eq!(
            user.get("text").and_then(Value::as_str),
            Some("fix the launcher bug")
        );
        let key = unit_key(
            "pi",
            "2026-08-27T08-39-05-000Z.jsonl",
            Some("2026-08-27T08:39:05.554Z"),
            "fix the launcher bug",
        );
        assert_eq!(user.get("key").and_then(Value::as_str), Some(key.as_str()));

        let compaction = &parsed.units[1];
        assert_eq!(
            compaction.get("kind").and_then(Value::as_str),
            Some("compaction")
        );
        assert_eq!(
            compaction.get("filesRead"),
            Some(&serde_json::json!(["a.rs"]))
        );
        assert_eq!(
            compaction.get("filesModified"),
            Some(&serde_json::json!(["b.rs"]))
        );

        let branch = &parsed.units[2];
        assert_eq!(branch.get("kind").and_then(Value::as_str), Some("branch"));
        assert!(branch.get("filesRead").is_none());
        assert!(branch.get("filesModified").is_none());

        assert_eq!(parsed.cursor.offset, body.len() as u64);
        assert_eq!(parsed.cursor.session.as_deref(), Some("sess-1"));
        assert_eq!(parsed.cursor.cwd.as_deref(), Some("/tmp/proj"));
    }

    #[test]
    fn claude_transcript_skips_tool_results() {
        let dir = TempDir::new("claude-parse");
        let path = dir.0.join("session.jsonl");
        let body = concat!(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]},"sessionId":"skip-me","cwd":"/ignored","timestamp":"2026-01-01T00:00:00.000Z"}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"text","text":"a real user question"}]},"sessionId":"s9","cwd":"/work","timestamp":"2026-01-01T01:00:00.000Z"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"assistant reply"}]},"timestamp":"2026-01-01T01:00:01.000Z"}"#,
            "\n",
            r#"{"type":"summary","summary":"","timestamp":"2026-01-01T02:00:00.000Z"}"#,
            "\n",
            r#"{"type":"summary","summary":"kept summary text","timestamp":"2026-01-01T02:00:00.000Z"}"#,
            "\n"
        );
        std::fs::write(&path, body).unwrap();
        let parsed = parse_file(
            &path,
            "claude",
            "/test-home/.claude",
            ParseCursor::default(),
        )
        .unwrap();
        assert_eq!(parsed.units.len(), 3);
        let user = &parsed.units[0];
        assert_eq!(user.get("kind").and_then(Value::as_str), Some("user"));
        assert_eq!(
            user.get("agent_home").and_then(Value::as_str),
            Some("/test-home/.claude")
        );
        assert_eq!(user.get("session").and_then(Value::as_str), Some("s9"));
        assert_eq!(user.get("cwd").and_then(Value::as_str), Some("/work"));
        let assistant = &parsed.units[1];
        assert_eq!(
            assistant.get("kind").and_then(Value::as_str),
            Some("assistant")
        );
        assert_eq!(
            assistant.get("text").and_then(Value::as_str),
            Some("assistant reply")
        );
        assert_eq!(assistant.get("session").and_then(Value::as_str), Some("s9"));
        let summary = &parsed.units[2];
        assert_eq!(
            summary.get("kind").and_then(Value::as_str),
            Some("compaction")
        );
        assert_eq!(
            summary.get("text").and_then(Value::as_str),
            Some("kept summary text")
        );
        assert!(summary.get("filesRead").is_none());
    }

    #[test]
    fn claude_assistant_text_becomes_assistant_unit() {
        let dir = TempDir::new("claude-assistant");
        let path = dir.0.join("session.jsonl");
        let body = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hidden reasoning"},{"type":"text","text":"the launcher fix landed"},{"type":"tool_use","name":"Bash","input":{}}]},"sessionId":"sa","cwd":"/wa","timestamp":"2026-01-01T03:00:00.000Z"}"#,
            "\n"
        );
        std::fs::write(&path, body).unwrap();
        let parsed = parse_file(
            &path,
            "claude",
            "/test-home/.claude",
            ParseCursor::default(),
        )
        .unwrap();
        assert_eq!(parsed.units.len(), 1);
        let unit = &parsed.units[0];
        assert_eq!(unit.get("kind").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            unit.get("text").and_then(Value::as_str),
            Some("the launcher fix landed")
        );
        assert_eq!(unit.get("session").and_then(Value::as_str), Some("sa"));
        assert_eq!(unit.get("cwd").and_then(Value::as_str), Some("/wa"));
        assert_eq!(
            unit.get("ts").and_then(Value::as_str),
            Some("2026-01-01T03:00:00.000Z")
        );
        let key = unit_key(
            "claude",
            "session.jsonl",
            Some("2026-01-01T03:00:00.000Z"),
            "the launcher fix landed",
        );
        assert_eq!(unit.get("key").and_then(Value::as_str), Some(key.as_str()));
    }

    #[test]
    fn claude_assistant_without_text_blocks_yields_no_unit() {
        let dir = TempDir::new("claude-tooluse");
        let path = dir.0.join("session.jsonl");
        let body = concat!(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]},"sessionId":"sa","cwd":"/wa","timestamp":"2026-01-01T03:00:00.000Z"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[]},"timestamp":"2026-01-01T03:00:01.000Z"}"#,
            "\n"
        );
        std::fs::write(&path, body).unwrap();
        let parsed = parse_file(
            &path,
            "claude",
            "/test-home/.claude",
            ParseCursor::default(),
        )
        .unwrap();
        assert!(parsed.units.is_empty());
    }

    #[test]
    fn pi_assistant_text_becomes_assistant_unit() {
        let dir = TempDir::new("pi-assistant");
        let path = dir.0.join("session.jsonl");
        let body = concat!(
            r#"{"type":"session","id":"sess-2","cwd":"/tmp/proj"}"#,
            "\n",
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"compilation is green now"}],"timestamp":1787819945600}}"#,
            "\n"
        );
        std::fs::write(&path, body).unwrap();
        let parsed =
            parse_file(&path, "pi", "/test-home/.pi/agent", ParseCursor::default()).unwrap();
        assert_eq!(parsed.units.len(), 1);
        let unit = &parsed.units[0];
        assert_eq!(unit.get("kind").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            unit.get("text").and_then(Value::as_str),
            Some("compilation is green now")
        );
        assert_eq!(unit.get("session").and_then(Value::as_str), Some("sess-2"));
        assert_eq!(unit.get("cwd").and_then(Value::as_str), Some("/tmp/proj"));
        assert_eq!(
            unit.get("ts").and_then(Value::as_str),
            Some("2026-08-27T08:39:05.600Z")
        );
    }

    #[test]
    fn claude_compact_summary_becomes_compaction_unit() {
        let dir = TempDir::new("claude-compact");
        let path = dir.0.join("session.jsonl");
        let body = concat!(
            r#"{"type":"user","isCompactSummary":true,"message":{"content":[{"type":"text","text":"This session is being continued from a previous conversation"}]},"sessionId":"sc","cwd":"/wc","timestamp":"2026-01-01T04:00:00.000Z"}"#,
            "\n"
        );
        std::fs::write(&path, body).unwrap();
        let parsed = parse_file(
            &path,
            "claude",
            "/test-home/.claude",
            ParseCursor::default(),
        )
        .unwrap();
        assert_eq!(parsed.units.len(), 1);
        let unit = &parsed.units[0];
        assert_eq!(unit.get("kind").and_then(Value::as_str), Some("compaction"));
        assert_eq!(
            unit.get("text").and_then(Value::as_str),
            Some("This session is being continued from a previous conversation")
        );
        assert_eq!(unit.get("session").and_then(Value::as_str), Some("sc"));
        assert_eq!(unit.get("cwd").and_then(Value::as_str), Some("/wc"));
    }
}
