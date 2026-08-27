use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use qol_plugin_daemon::daemon::ReadResult;
use qol_runtime::protocol::DaemonRequest;
use serde_json::{json, Value};

use crate::app::warm::WarmState;
use crate::ask::{AskRequest, LogOptions};

const DEFAULT_ASK_K: usize = 5;
const DEFAULT_LOG_SOURCE: &str = "daemon";

pub fn handle(state: &mut Arc<Mutex<WarmState>>, request: &DaemonRequest) -> ReadResult<()> {
    let result = match request.action.as_str() {
        "ping" | "kill" => return ReadResult::Handled,
        "ask" => ask(state, &request.input),
        "status" => status(state),
        "continue" => continue_request(state, &request.input),
        "capture" => capture(state, &request.input),
        "reindex" => reindex(state),
        _ => return ReadResult::Fallback,
    };
    match result {
        Ok(payload) => ReadResult::HandledWithData(payload),
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn ask(state: &Arc<Mutex<WarmState>>, input: &Value) -> Result<Value> {
    let req = AskRequest {
        query: string_field(input, "query", "ask")?,
        k: number_field(input, "k", "ask")?.unwrap_or(DEFAULT_ASK_K as u64) as usize,
        brief: bool_field(input, "brief", "ask")?.unwrap_or(false),
        exclude_session: optional_string_field(input, "exclude_session", "ask")?,
    };
    let log = LogOptions {
        source: optional_string_field(input, "log_source", "ask")?
            .unwrap_or_else(|| DEFAULT_LOG_SOURCE.to_string()),
        cwd: optional_string_field(input, "log_cwd", "ask")?,
        fact: optional_string_field(input, "log_fact", "ask")?,
        no_log: bool_field(input, "no_log", "ask")?.unwrap_or(false),
    };
    let mut warm = lock_state(state);
    let (store, aliases, units, notes) = warm.views()?;
    let output = crate::ask::run_and_log_with_layers(store, aliases, &req, &log, units, notes)?;
    serde_json::to_value(output).context("failed to encode the qol-memory ask response")
}

fn status(state: &Arc<Mutex<WarmState>>) -> Result<Value> {
    let mut warm = lock_state(state);
    let (store, _aliases, units, notes) = warm.views()?;
    crate::ask::status_with_layers(store, units, notes)
}

fn continue_request(state: &Arc<Mutex<WarmState>>, input: &Value) -> Result<Value> {
    let request: crate::continue_recall::ContinueRequest = serde_json::from_value(input.clone())
        .context("continue: input.cwd and input.session must be strings")?;
    let warm = lock_state(state);
    let outcome = crate::continue_recall::run(warm.store(), &request)?;
    serde_json::to_value(outcome).context("failed to encode the qol-memory continue response")
}

fn capture(state: &Arc<Mutex<WarmState>>, input: &Value) -> Result<Value> {
    let unit = match input.get("unit") {
        Some(unit) => {
            if !unit.is_object() {
                anyhow::bail!("capture: input.unit must be a JSON object");
            }
            unit.clone()
        }
        None => {
            let text = string_field(input, "text", "capture")?;
            let cwd = string_field(input, "cwd", "capture")?;
            let text = text.trim();
            if text.is_empty() {
                anyhow::bail!("capture: input.text must not be empty");
            }
            let cwd = cwd.trim();
            if cwd.is_empty() {
                anyhow::bail!("capture: input.cwd must not be empty");
            }
            crate::ingest::capture_unit(text, cwd, &crate::text::now_iso())
        }
    };
    let mut warm = lock_state(state);
    let store = warm.store().clone();
    let appended = crate::ingest::append_units(&store, std::slice::from_ref(&unit), warm.keys())?;
    if appended > 0 {
        warm.push_units(std::slice::from_ref(&unit));
    }
    let key = unit
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(json!({ "appended": appended, "key": key }))
}

fn reindex(state: &Arc<Mutex<WarmState>>) -> Result<Value> {
    let mut warm = lock_state(state);
    let store = warm.store().clone();
    let layers = crate::app::warm::reindex(&store)?;
    warm.invalidate_layers();
    Ok(json!({ "layers": layers }))
}

fn lock_state(state: &Arc<Mutex<WarmState>>) -> MutexGuard<'_, WarmState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn string_field(input: &Value, field: &str, action: &str) -> Result<String> {
    optional_string_field(input, field, action)?
        .ok_or_else(|| anyhow::anyhow!("{action}: input.{field} must be a string"))
}

fn optional_string_field(input: &Value, field: &str, action: &str) -> Result<Option<String>> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::to_owned)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{action}: input.{field} must be a string")),
    }
}

fn number_field(input: &Value, field: &str, action: &str) -> Result<Option<u64>> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{action}: input.{field} must be a number")),
    }
}

fn bool_field(input: &Value, field: &str, action: &str) -> Result<Option<bool>> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{action}: input.{field} must be a boolean")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::aliases::AliasMap;
    use crate::store::Store;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_store(tag: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "qol-memory-request-{tag}-{}-{id}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp store");
        dir
    }

    fn unit_value(key: &str, text: &str) -> Value {
        json!({
            "key": key,
            "source": "pi",
            "file": "a.jsonl",
            "session": "sess-live-aaa1",
            "cwd": "/repo",
            "kind": "user",
            "ts": "2026-08-01T09:00:00.000Z",
            "text": text
        })
    }

    fn warm_state(tag: &str, units: &[Value]) -> Arc<Mutex<WarmState>> {
        let root = temp_store(tag);
        let body = units
            .iter()
            .map(|value| serde_json::to_string(value).expect("serialize"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("units.jsonl"), format!("{body}\n")).expect("write units");
        let run_dir = root.join("notes").join("2026-08-05T10-00-00-000Z");
        std::fs::create_dir_all(&run_dir).expect("notes run dir");
        std::fs::write(
            run_dir.join("notes.jsonl"),
            "{\"key\":\"n-1\",\"cls\":\"decision\",\"source_kind\":\"decision\",\"source_ts\":\"2026-08-04T08:00:00.000Z\",\"text\":\"Decision: the clipboard ring survives tray restarts\"}\n",
        )
        .expect("write notes");
        let store = Store::resolve(Some(&root)).expect("store resolves");
        Arc::new(Mutex::new(
            WarmState::open(store, AliasMap::default()).expect("warm opens"),
        ))
    }

    fn respond(state: &mut Arc<Mutex<WarmState>>, action: &str, input: Value) -> ReadResult<()> {
        handle(
            state,
            &DaemonRequest {
                action: action.to_string(),
                input,
            },
        )
    }

    #[test]
    fn request_ask_uses_warm_layers() {
        let mut state = warm_state(
            "ask",
            &[unit_value(
                "u-1",
                "the plugin clipboard history ring survives tray restarts when the daemon runs",
            )],
        );
        let result = respond(
            &mut state,
            "ask",
            json!({ "query": "does the clipboard ring survive tray restarts", "no_log": true }),
        );
        let ReadResult::HandledWithData(value) = result else {
            panic!("ask must answer with data for action `ask`");
        };
        assert_eq!(
            value["query"],
            "does the clipboard ring survive tray restarts"
        );
        assert_eq!(value["counts"]["units"], 1);
        assert_eq!(value["counts"]["notes"], 1);
        assert!(value.get("reason").is_some());
    }

    #[test]
    fn request_ask_defaults_match_the_contract() {
        let mut state = warm_state(
            "ask-defaults",
            &[unit_value(
                "u-1",
                "an ordinary settled user fact about the launcher",
            )],
        );
        let ReadResult::HandledWithData(value) =
            respond(&mut state, "ask", json!({ "query": "launcher" }))
        else {
            panic!("ask must answer with data for action `ask`");
        };
        assert_eq!(value["query"], "launcher");
    }

    #[test]
    fn request_ask_requires_a_query() {
        let mut state = warm_state("ask-no-query", &[]);
        let result = respond(&mut state, "ask", json!({}));
        assert!(
            matches!(result, ReadResult::Error(message) if message.contains("input.query")),
            "missing query must name the field"
        );
    }

    #[test]
    fn request_capture_appends_and_reports_count() {
        let mut state = warm_state("capture", &[]);
        let unit = unit_value(
            "u-cap-1",
            "a captured settled fact from the request handler",
        );
        let ReadResult::HandledWithData(value) =
            respond(&mut state, "capture", json!({ "unit": unit }))
        else {
            panic!("capture must answer with data for action `capture`");
        };
        assert_eq!(value["appended"], 1);

        let warm = state.lock().expect("lock");
        let stored = std::fs::read_to_string(warm.store().units_path()).expect("read units");
        assert!(stored.contains("u-cap-1"));
    }

    #[test]
    fn request_capture_from_text_is_idempotent_and_recallable() {
        let fillers = [
            unit_value(
                "filler-01",
                "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november",
            ),
            unit_value(
                "filler-02",
                "oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu bakery candle",
            ),
            unit_value(
                "filler-03",
                "dragon engine forest garden hammer island jacket kettle lantern mountain noodle ocean pillow quilt",
            ),
            unit_value(
                "filler-04",
                "river saddle tunnel umbrella violin window yogurt bacon donut ember falcon gravel hazel ivory",
            ),
        ];
        let mut state = warm_state("capture-text", &fillers);
        let input = json!({
            "text": "zephyr quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            "cwd": "/tmp/proj"
        });
        let ReadResult::HandledWithData(value) = respond(&mut state, "capture", input.clone())
        else {
            panic!("capture must answer with data for action `capture`");
        };
        assert_eq!(value["appended"], 1);
        let expected = crate::ingest::capture_unit(
            "zephyr quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
            "/tmp/proj",
            "ignored",
        );
        assert_eq!(value["key"], expected["key"]);
        let key = value["key"].as_str().expect("capture answers a key");
        assert_eq!(key.len(), 16);
        assert!(key.chars().all(|ch| ch.is_ascii_hexdigit()));

        let ReadResult::HandledWithData(value) = respond(&mut state, "capture", input) else {
            panic!("capture must answer with data for action `capture`");
        };
        assert_eq!(value["appended"], 0);
        assert_eq!(value["key"], expected["key"]);

        let ReadResult::HandledWithData(value) = respond(
            &mut state,
            "ask",
            json!({ "query": "zephyr quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz", "no_log": true }),
        ) else {
            panic!("ask must answer with data for action `ask`");
        };
        assert_eq!(value["verdict"], "answered");
        assert_eq!(value["answer"]["layer"], "unit");
        assert_eq!(value["answer"]["source_kind"], "capture");
    }

    #[test]
    fn request_capture_rejects_empty_text() {
        let mut state = warm_state("capture-empty", &[]);
        let result = respond(
            &mut state,
            "capture",
            json!({ "text": "   ", "cwd": "/tmp/proj" }),
        );
        assert!(
            matches!(result, ReadResult::Error(message) if message == "capture: input.text must not be empty"),
            "whitespace-only text must be rejected"
        );
    }

    #[test]
    fn request_capture_rejects_non_object_units() {
        let mut state = warm_state("capture-bad", &[]);
        let result = respond(&mut state, "capture", json!({ "unit": "nope" }));
        assert!(
            matches!(result, ReadResult::Error(message) if message == "capture: input.unit must be a JSON object"),
            "non-object unit must name the field"
        );

        let result = respond(&mut state, "capture", json!({ "unit": null }));
        assert!(
            matches!(result, ReadResult::Error(message) if message == "capture: input.unit must be a JSON object"),
            "non-object unit must name the field"
        );
    }

    #[test]
    fn request_unknown_action_is_fallback() {
        let mut state = warm_state("fallback", &[]);
        let result = respond(&mut state, "warp", json!({}));
        assert!(matches!(result, ReadResult::Fallback));
    }

    #[test]
    fn request_ping_and_kill_are_handled() {
        let mut state = warm_state("ping", &[]);
        assert!(matches!(
            respond(&mut state, "ping", Value::Null),
            ReadResult::Handled
        ));
        assert!(matches!(
            respond(&mut state, "kill", Value::Null),
            ReadResult::Handled
        ));
    }
}
