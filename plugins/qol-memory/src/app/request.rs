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
        "rows" => rows(state, &request.input),
        "feedback" => feedback(state, &request.input),
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
        agent_home: optional_string_field(input, "agent_home", "ask")?,
    };
    let log = LogOptions {
        source: optional_string_field(input, "log_source", "ask")?
            .unwrap_or_else(|| DEFAULT_LOG_SOURCE.to_string()),
        cwd: optional_string_field(input, "log_cwd", "ask")?,
        fact: optional_string_field(input, "log_fact", "ask")?,
        no_log: bool_field(input, "no_log", "ask")?.unwrap_or(false),
    };
    let mut warm = lock_state(state);
    let started = std::time::Instant::now();
    let caller = qol_agent_homes::Registry::load().resolve_caller(req.agent_home.as_deref());
    let (store, aliases, units, notes, indexes) = warm.ask_views(&caller)?;
    let mut output =
        crate::ask::run_with_warm(store, aliases, &req, units, notes, indexes.as_ref())?;
    warm.verify_answer(&req, &log.source, &mut output)?;
    crate::ask::log_output(
        warm.store(),
        &req,
        &log,
        &output,
        started.elapsed().as_millis() as u64,
    );
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
    let mut unit = match input.get("unit") {
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
    let registry = qol_agent_homes::Registry::load();
    let caller =
        registry.resolve_caller(optional_string_field(input, "agent_home", "capture")?.as_deref());
    if let Some(fields) = unit.as_object_mut() {
        fields.insert("agent_home".to_string(), json!(caller));
    }
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

fn rows(state: &Arc<Mutex<WarmState>>, input: &Value) -> Result<Value> {
    let started = std::time::Instant::now();
    let query = string_field(input, "query", "rows")?.trim().to_string();
    if query.is_empty() {
        anyhow::bail!("rows: input.query must not be empty");
    }
    let req = AskRequest {
        query,
        k: DEFAULT_ASK_K,
        brief: false,
        exclude_session: None,
        agent_home: optional_string_field(input, "agent_home", "rows")?,
    };
    let log = LogOptions {
        source: "launcher".to_string(),
        cwd: None,
        fact: None,
        no_log: bool_field(input, "no_log", "rows")?.unwrap_or(false),
    };
    let mut warm = lock_state(state);
    let caller = qol_agent_homes::Registry::load().resolve_caller(req.agent_home.as_deref());
    let (store, aliases, units, notes, indexes) = warm.ask_views(&caller)?;
    let mut output =
        crate::ask::run_with_warm(store, aliases, &req, units, notes, indexes.as_ref())?;
    warm.verify_answer(&req, &log.source, &mut output)?;
    crate::ask::log_output(
        warm.store(),
        &req,
        &log,
        &output,
        started.elapsed().as_millis() as u64,
    );
    let (units, notes) = warm.layers()?;
    let flow_rows = crate::ask::rows::from_output(&output, units, notes);
    qol_runtime::probe!(
        "QOL_MEMORY_DAEMON",
        "event=rows_result verdict={} matching={} conflicts={} verification={:?}",
        output.verdict,
        output.signals.matching_captures,
        output.signals.conflicting_captures,
        output.verification
    );
    Ok(json!({
        "verdict": output.verdict,
        "confidence": output.confidence,
        "rows": flow_rows,
        "verification": output.verification,
    }))
}

fn feedback(state: &Arc<Mutex<WarmState>>, input: &Value) -> Result<Value> {
    let query = string_field(input, "query", "feedback")?;
    let key = string_field(input, "key", "feedback")?;
    let vote = vote_field(input, "feedback")?;
    let warm = lock_state(state);
    let store = warm.store().clone();
    let norm = crate::retrieval_log::normalize_query(&query);
    crate::feedback::append_vote(store.root(), &norm, &key, vote);
    Ok(json!({ "recorded": true }))
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

fn vote_field(input: &Value, action: &str) -> Result<i64> {
    let vote = input
        .get("vote")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("{action}: input.vote must be a number"))?;
    if vote != -1 && vote != 1 {
        anyhow::bail!("{action}: input.vote must be -1 or 1");
    }
    Ok(vote)
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
    fn first_capture_is_recallable_after_opening_a_missing_store() {
        let parent = temp_store("first-capture");
        let root = parent.join("new-store");
        let store = Store::resolve(Some(&root)).unwrap();
        let mut state = Arc::new(Mutex::new(
            WarmState::open(store, crate::aliases::embedded()).unwrap(),
        ));
        let question = "How to run KCD2 in debug mode?";
        let input = json!({"query":question,"no_log":true});
        let ReadResult::HandledWithData(empty) = respond(&mut state, "ask", input.clone()) else {
            panic!("a missing store must answer the request");
        };
        assert!(empty["answer"].is_null());
        let ReadResult::HandledWithData(captured) = respond(
            &mut state,
            "capture",
            json!({"text":format!("Q: {question} A: Run forge dev."),"cwd":"/fixture/project"}),
        ) else {
            panic!("first capture must create the store");
        };
        assert_eq!(captured["appended"], 1);
        let ReadResult::HandledWithData(answer) = respond(&mut state, "ask", input) else {
            panic!("the new capture must be visible to ask");
        };
        assert_eq!(answer["answer"]["text"], "Run forge dev.");
        drop(state);
        std::fs::remove_dir_all(parent).unwrap();
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
    fn request_rows_returns_the_answer_row_first() {
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
        let mut state = warm_state("rows-answer", &fillers);
        let text =
            "zephyr quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz";
        let ReadResult::HandledWithData(_) = respond(
            &mut state,
            "capture",
            json!({ "text": text, "cwd": "/tmp/proj" }),
        ) else {
            panic!("capture must answer with data for action `capture`");
        };
        let ReadResult::HandledWithData(value) =
            respond(&mut state, "rows", json!({ "query": text }))
        else {
            panic!("rows must answer with data for action `rows`");
        };
        assert_eq!(value["verdict"], "answered");
        let rows = value["rows"].as_array().expect("rows array");
        assert_eq!(rows[0]["kind"], "answer");
        assert_eq!(rows[0]["title"], crate::ask::rows::title_of(text));
    }

    #[test]
    fn request_rows_rejects_empty_query() {
        let mut state = warm_state("rows-empty", &[]);
        let result = respond(&mut state, "rows", json!({ "query": "   " }));
        assert!(
            matches!(
                result,
                ReadResult::Error(message) if message == "rows: input.query must not be empty"
            ),
            "whitespace-only query must be rejected"
        );

        let result = respond(&mut state, "rows", json!({}));
        assert!(
            matches!(result, ReadResult::Error(message) if message.contains("input.query")),
            "missing query must name the field"
        );
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
    fn request_capture_stamps_and_overwrites_the_agent_home() {
        let mut state = warm_state("capture-home", &[]);
        let ReadResult::HandledWithData(_) = respond(
            &mut state,
            "capture",
            json!({
                "text": "ember quartz flint cobalt onyx basalt garnet pyrite mica slate beryl opal jade topaz",
                "cwd": "/tmp/proj",
                "agent_home": "/tmp/qol-home-mine"
            }),
        ) else {
            panic!("capture must answer with data for action `capture`");
        };
        let mut provided = unit_value("u-stamp", "a provided whole unit stamped by the handler");
        provided["agent_home"] = json!("/tmp/qol-home-other");
        let ReadResult::HandledWithData(_) = respond(
            &mut state,
            "capture",
            json!({ "unit": provided, "agent_home": "/tmp/qol-home-mine" }),
        ) else {
            panic!("capture must answer with data for action `capture`");
        };
        let warm = state.lock().expect("lock");
        let stored = std::fs::read_to_string(warm.store().units_path()).expect("read units");
        assert_eq!(stored.lines().filter(|line| !line.is_empty()).count(), 2);
        assert!(stored.contains("\"agent_home\":\"/tmp/qol-home-mine\""));
        assert!(!stored.contains("/tmp/qol-home-other"));
    }

    #[test]
    fn request_feedback_appends_a_vote_line() {
        let mut state = warm_state("feedback", &[]);
        let ReadResult::HandledWithData(value) = respond(
            &mut state,
            "feedback",
            json!({ "query": "What is KCD2 Forge?", "key": "u-1", "vote": -1 }),
        ) else {
            panic!("feedback must answer with data for action `feedback`");
        };
        assert_eq!(value["recorded"], true);

        let warm = state.lock().expect("lock");
        let raw = std::fs::read_to_string(warm.store().root().join("feedback.jsonl"))
            .expect("read feedback.jsonl");
        let line: Value = serde_json::from_str(raw.lines().next().expect("one line"))
            .expect("feedback line parses");
        assert_eq!(line["norm"], "what is kcd2 forge");
        assert_eq!(line["key"], "u-1");
        assert_eq!(line["vote"], -1);
        assert!(line["ts"].as_str().expect("ts string").contains('T'));
    }

    #[test]
    fn request_feedback_validates_input() {
        let mut state = warm_state("feedback-bad", &[]);
        let result = respond(&mut state, "feedback", json!({}));
        assert!(
            matches!(result, ReadResult::Error(message) if message.contains("input.query")),
            "missing query must name the field"
        );

        let result = respond(&mut state, "feedback", json!({ "query": "q" }));
        assert!(
            matches!(result, ReadResult::Error(message) if message.contains("input.key")),
            "missing key must name the field"
        );

        let result = respond(&mut state, "feedback", json!({ "query": "q", "key": "k" }));
        assert!(
            matches!(result, ReadResult::Error(message) if message == "feedback: input.vote must be a number"),
            "missing vote must name the field"
        );

        let result = respond(
            &mut state,
            "feedback",
            json!({ "query": "q", "key": "k", "vote": 0 }),
        );
        assert!(
            matches!(result, ReadResult::Error(message) if message == "feedback: input.vote must be -1 or 1"),
            "out-of-range vote must be rejected"
        );

        let result = respond(
            &mut state,
            "feedback",
            json!({ "query": "q", "key": "k", "vote": "down" }),
        );
        assert!(
            matches!(result, ReadResult::Error(message) if message == "feedback: input.vote must be a number"),
            "non-numeric vote must be rejected"
        );

        let warm = state.lock().expect("lock");
        assert!(
            !warm.store().root().join("feedback.jsonl").exists(),
            "rejected votes must not touch feedback.jsonl"
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
