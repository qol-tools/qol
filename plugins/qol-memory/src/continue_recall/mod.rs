use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::store::seal::try_sealed_text;
use crate::store::{Store, BOILERPLATE_MARKERS};
use crate::text::{collapse_ws, parse_iso_millis, utf16_len, utf16_slice};

pub const SCHEMA: &str = "qol-memory-continue-v1";
const MIN_TEXT: usize = 40;
const MIN_DELTA: usize = 2;
const K: usize = 3;
const CAP_USER: usize = 2;
const CAP_COMPACTION: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ContinueRequest {
    pub cwd: String,
    pub session: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContinueOutcome {
    pub stage: String,
    pub reason: Option<String>,
    pub count: usize,
    pub block: Option<String>,
}

pub fn run(store: &Store, request: &ContinueRequest) -> anyhow::Result<ContinueOutcome> {
    if request.cwd.is_empty() || request.session.is_empty() {
        return Ok(emit(store, outcome("abstain", Some("no-cwd"))));
    }
    if std::env::var("QOL_MEMORY_CONTINUE_DISABLE").as_deref() == Ok("1") {
        return Ok(emit(store, outcome("disabled", Some("env"))));
    }
    if store.root().join("continue.disabled").exists() {
        return Ok(emit(store, outcome("disabled", Some("flag-file"))));
    }
    let marker = read_marker(store);
    let entry = marker
        .get("cwds")
        .and_then(|cwds| cwds.get(request.cwd.as_str()));
    let entry_ms = entry
        .and_then(|item| item.get("ts"))
        .and_then(Value::as_str)
        .map(|ts| parse_iso_millis(Some(ts)))
        .unwrap_or(0);
    let raw = match std::fs::read(store.units_path()) {
        Ok(raw) => raw,
        Err(_) => return Ok(emit(store, outcome("abstain", Some("read-error")))),
    };
    let total_lines = line_count(&raw);
    let store_reset = entry
        .and_then(|item| item.get("units_count"))
        .and_then(Value::as_u64)
        .is_some_and(|count| (total_lines as u64) < count);
    if store_reset {
        let _ = write_marker(store, &marker, &request.cwd, &request.session, total_lines);
        return Ok(emit(store, outcome("gate-miss", Some("store-reset"))));
    }
    let text = try_sealed_text(store.root(), &raw)
        .unwrap_or_else(|| String::from_utf8_lossy(&raw).into_owned());
    let picked = pick_units(&parse_units(&text), entry_ms, &request.session);
    if write_marker(store, &marker, &request.cwd, &request.session, total_lines).is_err() {
        return Ok(emit(store, outcome("abstain", Some("marker-write-error"))));
    }
    if picked.len() >= MIN_DELTA {
        let block = build_block(&picked, entry);
        log_entry(store, json!({"stage": "injected", "count": picked.len()}));
        return Ok(ContinueOutcome {
            stage: "injected".to_string(),
            reason: None,
            count: picked.len(),
            block: Some(block),
        });
    }
    log_entry(
        store,
        json!({
            "stage": "gate-miss",
            "reason": "below-min-delta",
            "delta": picked.len()
        }),
    );
    Ok(ContinueOutcome {
        stage: "gate-miss".to_string(),
        reason: Some("below-min-delta".to_string()),
        count: picked.len(),
        block: None,
    })
}

fn outcome(stage: &str, reason: Option<&str>) -> ContinueOutcome {
    ContinueOutcome {
        stage: stage.to_string(),
        reason: reason.map(str::to_owned),
        count: 0,
        block: None,
    }
}

fn emit(store: &Store, result: ContinueOutcome) -> ContinueOutcome {
    let mut fields = Map::new();
    fields.insert("stage".to_string(), json!(result.stage));
    if let Some(reason) = result.reason.as_deref() {
        fields.insert("reason".to_string(), json!(reason));
    }
    log_entry(store, Value::Object(fields));
    result
}

fn log_entry(store: &Store, entry: Value) {
    let mut line = Map::new();
    line.insert("ts".to_string(), json!(crate::text::now_iso()));
    if let Value::Object(fields) = entry {
        for (key, value) in fields {
            line.insert(key, value);
        }
    }
    let _ = std::fs::create_dir_all(store.root());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(store.root().join("hook.log"))
    {
        use std::io::Write;
        let serialized = serde_json::to_string(&Value::Object(line)).unwrap_or_default();
        let _ = writeln!(file, "{}", serialized);
    }
}

fn read_marker(store: &Store) -> Value {
    let fallback = json!({"schema": SCHEMA, "cwds": {}});
    let Ok(text) = std::fs::read_to_string(store.continue_marker_path()) else {
        return fallback;
    };
    let Ok(marker) = serde_json::from_str::<Value>(&text) else {
        return fallback;
    };
    if marker.get("schema").and_then(Value::as_str) == Some(SCHEMA)
        && marker.get("cwds").is_some_and(Value::is_object)
    {
        marker
    } else {
        fallback
    }
}

fn write_marker(
    store: &Store,
    marker: &Value,
    cwd: &str,
    session: &str,
    units_count: usize,
) -> anyhow::Result<()> {
    let mut updated = marker.clone();
    let now = crate::text::now_iso();
    let Some(map) = updated.as_object_mut() else {
        anyhow::bail!("qol-memory: continue marker is not an object");
    };
    let Some(cwds) = map.get_mut("cwds").and_then(Value::as_object_mut) else {
        anyhow::bail!("qol-memory: continue marker has no cwds map");
    };
    cwds.insert(
        cwd.to_string(),
        json!({
            "ts": now.clone(),
            "session": session,
            "units_count": units_count,
            "updated": now
        }),
    );
    let mut text = serde_json::to_string_pretty(&updated)?;
    text.push('\n');
    qol_fs::atomic_write(&store.continue_marker_path(), text.as_bytes())?;
    Ok(())
}

fn line_count(raw: &[u8]) -> usize {
    let mut count = raw.iter().filter(|byte| **byte == b'\n').count();
    if !raw.is_empty() && *raw.last().unwrap() != b'\n' {
        count += 1;
    }
    count
}

fn parse_units(text: &str) -> Vec<Value> {
    text.split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn pick_units(units: &[Value], entry_ms: i64, session: &str) -> Vec<Value> {
    let mut candidates: Vec<&Value> = units
        .iter()
        .filter(|unit| is_candidate(unit, entry_ms, session))
        .collect();
    candidates.sort_by(|left, right| {
        let order = ts_ms(right).cmp(&ts_ms(left));
        if order == std::cmp::Ordering::Equal {
            key_text(left).cmp(key_text(right))
        } else {
            order
        }
    });
    let mut counts: HashMap<String, [usize; 2]> = HashMap::new();
    let mut picked = Vec::new();
    for unit in candidates {
        let kind = unit.get("kind").and_then(Value::as_str).unwrap_or("");
        let owner = unit
            .get("session")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let slots = counts.entry(owner).or_insert([0, 0]);
        let index = if kind == "compaction" { 1 } else { 0 };
        let cap = if kind == "compaction" {
            CAP_COMPACTION
        } else {
            CAP_USER
        };
        if slots[index] >= cap {
            continue;
        }
        slots[index] += 1;
        picked.push(unit.clone());
        if picked.len() >= K {
            break;
        }
    }
    picked
}

fn is_candidate(unit: &Value, entry_ms: i64, session: &str) -> bool {
    let kind = unit.get("kind").and_then(Value::as_str);
    if kind != Some("user") && kind != Some("compaction") && kind != Some("capture") {
        return false;
    }
    let Some(text) = unit.get("text").and_then(Value::as_str) else {
        return false;
    };
    if utf16_len(text.trim()) < MIN_TEXT {
        return false;
    }
    if BOILERPLATE_MARKERS
        .iter()
        .any(|marker| text.contains(marker))
    {
        return false;
    }
    if unit
        .get("session")
        .and_then(Value::as_str)
        .is_some_and(|owner| owner == session)
    {
        return false;
    }
    let ts = ts_ms(unit);
    if ts == 0 {
        return false;
    }
    if entry_ms != 0 && ts <= entry_ms {
        return false;
    }
    true
}

fn ts_ms(unit: &Value) -> i64 {
    unit.get("ts")
        .and_then(Value::as_str)
        .map(|ts| parse_iso_millis(Some(ts)))
        .unwrap_or(0)
}

fn key_text(unit: &Value) -> &str {
    unit.get("key").and_then(Value::as_str).unwrap_or("")
}

fn build_block(picked: &[Value], entry: Option<&Value>) -> String {
    let mut lines = vec![format!(
        "[qol-memory continue] {} unit(s) landed in the store since your last session here ({}):",
        picked.len(),
        anchor_ts(entry)
    )];
    for unit in picked {
        lines.push(format!(
            "  NEW {} {} {} {} \"{}\"",
            unit.get("ts").and_then(Value::as_str).unwrap_or(""),
            unit.get("kind").and_then(Value::as_str).unwrap_or(""),
            utf16_slice(
                unit.get("session").and_then(Value::as_str).unwrap_or(""),
                0,
                8
            ),
            utf16_slice(unit.get("key").and_then(Value::as_str).unwrap_or(""), 0, 8),
            snippet(unit.get("text").and_then(Value::as_str).unwrap_or(""))
        ));
    }
    lines.join("\n")
}

fn anchor_ts(entry: Option<&Value>) -> String {
    let Some(ts) = entry
        .and_then(|item| item.get("ts"))
        .and_then(Value::as_str)
    else {
        return "never".to_string();
    };
    if ts.is_empty() {
        return "never".to_string();
    }
    strip_millis(ts)
}

fn strip_millis(ts: &str) -> String {
    let Some(base) = ts.strip_suffix('Z') else {
        return ts.to_string();
    };
    let bytes = base.as_bytes();
    if bytes.len() >= 4
        && bytes[bytes.len() - 4] == b'.'
        && bytes[bytes.len() - 3..].iter().all(u8::is_ascii_digit)
    {
        return format!("{}Z", &base[..base.len() - 4]);
    }
    ts.to_string()
}

fn snippet(text: &str) -> String {
    utf16_slice(collapse_ws(text).as_str(), 0, 140)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-continue-{}-{}-{}",
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

    const UNITS: &str = concat!(
        r#"{"key":"0123456789abcdef","source":"pi","file":"a.jsonl","session":"aaaaaaaa-1111","cwd":"/proj","kind":"user","ts":"2026-08-02T10:00:00.000Z","text":"Fix the  launcher   bug and verify the fix end to end"}"#,
        "\n",
        r#"{"key":"fedcba9876543210","source":"pi","file":"a.jsonl","session":"bbbbbbbb-2222","cwd":"/proj","kind":"user","ts":"2026-08-02T11:00:00.000Z","text":"Ship the  tray   icon change and update the changelog file"}"#,
        "\n",
        r#"{"key":"8888999900001111","source":"pi","file":"a.jsonl","session":"cccccccc-3333","cwd":"/proj","kind":"compaction","ts":"2026-08-02T12:00:00.000Z","text":"Rolled up  session   context with the distilled summary of decisions"}"#,
        "\n"
    );

    fn store_in(tag: &str) -> (TempDir, Store) {
        let dir = TempDir::new(tag);
        let root = dir.0.join("store");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::resolve(Some(root.as_path())).unwrap();
        (dir, store)
    }

    fn write_marker_fixture(store: &Store, ts: &str, units_count: u64) {
        let marker = json!({
            "schema": SCHEMA,
            "cwds": {
                "/proj": {
                    "ts": ts,
                    "session": "previous-session",
                    "units_count": units_count,
                    "updated": ts
                }
            }
        });
        std::fs::write(
            store.continue_marker_path(),
            format!("{}\n", serde_json::to_string_pretty(&marker).unwrap()),
        )
        .unwrap();
    }

    fn read_marker_field(store: &Store, cwd: &str, field: &str) -> Value {
        let text = std::fs::read_to_string(store.continue_marker_path()).unwrap();
        let marker: Value = serde_json::from_str(&text).unwrap();
        marker["cwds"][cwd][field].clone()
    }

    fn request() -> ContinueRequest {
        ContinueRequest {
            cwd: "/proj".to_string(),
            session: "current".to_string(),
        }
    }

    #[test]
    fn continue_injected_block_matches_fixture() {
        let (_dir, store) = store_in("injected");
        std::fs::write(store.units_path(), UNITS).unwrap();
        write_marker_fixture(&store, "2026-08-01T00:00:00.000Z", 3);
        let outcome = run(&store, &request()).unwrap();
        let expected = concat!(
            "[qol-memory continue] 3 unit(s) landed in the store since your last session here (2026-08-01T00:00:00Z):\n",
            "  NEW 2026-08-02T12:00:00.000Z compaction cccccccc 88889999 \"Rolled up session context with the distilled summary of decisions\"\n",
            "  NEW 2026-08-02T11:00:00.000Z user bbbbbbbb fedcba98 \"Ship the tray icon change and update the changelog file\"\n",
            "  NEW 2026-08-02T10:00:00.000Z user aaaaaaaa 01234567 \"Fix the launcher bug and verify the fix end to end\""
        );
        assert_eq!(
            outcome,
            ContinueOutcome {
                stage: "injected".to_string(),
                reason: None,
                count: 3,
                block: Some(expected.to_string()),
            }
        );
        assert_eq!(
            read_marker_field(&store, "/proj", "session"),
            json!("current")
        );
        assert_eq!(read_marker_field(&store, "/proj", "units_count"), json!(3));
        let log = std::fs::read_to_string(store.root().join("hook.log")).unwrap();
        assert!(log.contains("\"stage\":\"injected\""));
        assert!(log.contains("\"count\":3"));
    }

    #[test]
    fn continue_gate_miss_below_min_delta() {
        let (_dir, store) = store_in("gate-miss");
        std::fs::write(store.units_path(), UNITS).unwrap();
        write_marker_fixture(&store, "2026-08-05T00:00:00.000Z", 3);
        let outcome = run(&store, &request()).unwrap();
        assert_eq!(
            outcome,
            ContinueOutcome {
                stage: "gate-miss".to_string(),
                reason: Some("below-min-delta".to_string()),
                count: 0,
                block: None,
            }
        );
        assert_eq!(
            read_marker_field(&store, "/proj", "session"),
            json!("current")
        );
        assert_eq!(read_marker_field(&store, "/proj", "units_count"), json!(3));
        let log = std::fs::read_to_string(store.root().join("hook.log")).unwrap();
        assert!(log.contains("\"reason\":\"below-min-delta\""));
        assert!(log.contains("\"delta\":0"));
    }

    #[test]
    fn continue_store_reset_rewrites_marker() {
        let (_dir, store) = store_in("store-reset");
        std::fs::write(store.units_path(), UNITS).unwrap();
        write_marker_fixture(&store, "2026-08-01T00:00:00.000Z", 10);
        let outcome = run(&store, &request()).unwrap();
        assert_eq!(
            outcome,
            ContinueOutcome {
                stage: "gate-miss".to_string(),
                reason: Some("store-reset".to_string()),
                count: 0,
                block: None,
            }
        );
        assert_eq!(
            read_marker_field(&store, "/proj", "session"),
            json!("current")
        );
        assert_eq!(read_marker_field(&store, "/proj", "units_count"), json!(3));
        let log = std::fs::read_to_string(store.root().join("hook.log")).unwrap();
        assert!(log.contains("\"reason\":\"store-reset\""));
    }

    #[test]
    fn continue_injects_capture_units_like_user_units() {
        let (_dir, store) = store_in("capture-units");
        let capture_unit = json!({
            "key": "aaaabbbbccccdddd",
            "source": "agent",
            "cwd": "/proj",
            "kind": "capture",
            "ts": "2026-08-02T13:00:00.000Z",
            "text": "The capture lane stores one settled fact about the widget cache daemon"
        });
        let compaction_unit = json!({
            "key": "ddddeeeeffff0000",
            "source": "pi",
            "file": "a.jsonl",
            "session": "dddddddd-4444",
            "cwd": "/proj",
            "kind": "compaction",
            "ts": "2026-08-02T14:00:00.000Z",
            "text": "Rolled up  session   context with the distilled summary of decisions"
        });
        let body = [capture_unit, compaction_unit]
            .iter()
            .map(|unit| serde_json::to_string(unit).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(store.units_path(), body).unwrap();
        write_marker_fixture(&store, "2026-08-01T00:00:00.000Z", 2);
        let outcome = run(&store, &request()).unwrap();
        let expected = concat!(
            "[qol-memory continue] 2 unit(s) landed in the store since your last session here (2026-08-01T00:00:00Z):\n",
            "  NEW 2026-08-02T14:00:00.000Z compaction dddddddd ddddeeee \"Rolled up session context with the distilled summary of decisions\"\n",
            "  NEW 2026-08-02T13:00:00.000Z capture  aaaabbbb \"The capture lane stores one settled fact about the widget cache daemon\""
        );
        assert_eq!(
            outcome,
            ContinueOutcome {
                stage: "injected".to_string(),
                reason: None,
                count: 2,
                block: Some(expected.to_string()),
            }
        );
        assert_eq!(read_marker_field(&store, "/proj", "units_count"), json!(2));
    }
}
