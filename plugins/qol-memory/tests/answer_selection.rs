use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use qol_memory::aliases::AliasMap;
use qol_memory::app::{request, warm::WarmState};
use qol_memory::ask::{self, AskRequest};
use qol_memory::store::Store;
use qol_plugin_daemon::daemon::ReadResult;
use qol_runtime::protocol::DaemonRequest;
use serde_json::{json, Value};

const HOME: &str = "/fixture/agent";

struct Fixture {
    root: PathBuf,
    store: Store,
    home: String,
}

impl Fixture {
    fn new(extra: &[Value]) -> Self {
        Self::in_home(extra, HOME)
    }

    fn in_home(extra: &[Value], home: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "qol-answer-selection-{}-{}-{}",
            std::process::id(),
            qol_memory::text::now_iso().replace(':', "-"),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut records = vec![
            capture("language-a", "Rust. What language is the quartz monorepo: a Cargo workspace with a JavaScript UI."),
            capture("language-b", "Rust. Which language is the quartz monorepo written in: the workspace uses Cargo."),
            capture("config", "Q: Where is the quartz configuration stored? A: /fixture/quartz/settings.toml"),
            capture("launch-a", "Run ./quartz-forge dev or ./quartz-forge -d. How to run Quartz in debug mode: dev and -d launch the instrumented build."),
            capture("launch-b", "./quartz-forge -d. How to start Quartz in debug mode: the same as dev, with debug logging enabled."),
            capture("launch-c", "./quartz-forge dev (or -d). What command is Quartz debug mode: dev and -d launch the instrumented build."),
            capture("version", "v3.2. What version of the topaz protocol is current: the current wire format."),
            capture("restart", "Yes. Does the quartz clipboard survive restarts: history is persisted."),
        ];
        for record in &mut records {
            record["agent_home"] = json!(home);
        }
        records.extend_from_slice(extra);
        let body = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("units.jsonl"), format!("{body}\n")).unwrap();
        let store = Store::resolve(Some(&root)).unwrap();
        Self {
            root,
            store,
            home: home.into(),
        }
    }

    fn ask(&self, query: &str, k: usize) -> Value {
        serde_json::to_value(
            ask::run(
                &self.store,
                &AliasMap::default(),
                &AskRequest {
                    query: query.into(),
                    k,
                    brief: false,
                    exclude_session: None,
                    agent_home: Some(self.home.clone()),
                },
            )
            .unwrap(),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn capture(key: &str, text: &str) -> Value {
    json!({"key":key,"kind":"capture","text":text,"agent_home":HOME,"cwd":"/fixture/project","session":"settled"})
}

fn cases() -> Vec<(&'static str, &'static str)> {
    vec![
        ("what language is quartz monorepo", "language-a"),
        ("what language is the quartz monorepo?", "language-a"),
        ("WHICH LANGUAGE IS THE QUARTZ MONOREPO?!", "language-a"),
        ("quartz monorepo: what language?", "language-a"),
        ("what's the quartz monorepo written in?", "language-a"),
        ("what langauge is the quartz monorepo", "language-a"),
        ("where is quartz configuration stored", "config"),
        ("where can I find the quartz configuration?", "config"),
        ("how to open quartz debug", "launch-a"),
        ("how to boot quartz debug", "launch-a"),
        ("how do I launch quartz in debug mode?", "launch-a"),
        ("quartz debug", "launch-a"),
        ("QUARTZ DEBUG?", "launch-a"),
        ("run quartz debug", "launch-a"),
        ("quartz in debug mode", "launch-a"),
        ("quartz monorepo language", "language-a"),
        ("quartz monorepo langauge", "language-a"),
        ("quartz configuration", "config"),
        ("configuration quartz", "config"),
        ("topaz protocol version", "version"),
        ("what version is the topaz protocol", "version"),
        ("does quartz clipboard survive restarts?", "restart"),
    ]
}

#[test]
fn equivalent_questions_select_the_same_capture_in_small_and_noisy_stores() {
    let mut results = Vec::new();
    let distractors = (0..80).map(|i| json!({
        "key":format!("discussion-{i}"),"kind":"assistant","agent_home":HOME,
        "text":format!("Discussion {i}: what language is quartz monorepo? The memory ranking bug affects language questions, quartz monorepo questions, and what language the quartz monorepo uses. What language is quartz monorepo?"),
    })).collect::<Vec<_>>();
    for noise in [&[][..], &distractors[..]] {
        let fixture = Fixture::new(noise);
        for k in [1, 5] {
            for (query, key) in cases() {
                let result = fixture.ask(query, k);
                results.push(json!({
                    "query":query,"expected_key":key,"k":k,"distractors":noise.len(),
                    "actual_key":result["answer"]["key"],"verdict":result["verdict"],
                    "passed":result["verdict"] == "answered" && result["answer"]["key"] == key,
                }));
            }
        }
    }
    if let Some(out) = std::env::var_os("QOL_MEMORY_EVAL_OUT") {
        std::fs::write(
            PathBuf::from(out).join("cases.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }
    let failures = results
        .iter()
        .filter(|result| result["passed"] != true)
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "{}",
        serde_json::to_string_pretty(&failures).unwrap()
    );
}

#[test]
fn meaning_changes_do_not_reuse_a_nearby_answer() {
    let fixture = Fixture::new(&[
        capture(
            "copy",
            "copy-alpha-beta. How to copy alpha to beta: transfers the files.",
        ),
        capture(
            "edit",
            "Q: What is the quartz forge edit syntax? A: Bare edit captures the active scene.",
        ),
        capture(
            "launch-negative",
            "./topaz-forge release. How to run Topaz without debug mode: launches the release build.",
        ),
        capture(
            "cpp-docs",
            "Q: Where is C++ documentation stored? A: /fixture/cpp/docs",
        ),
    ]);
    for query in [
        "what language is the topaz monorepo",
        "what is quartz forge",
        "how to stop quartz debug",
        "how to open quartz debug remotely",
        "how to open quartz without debug",
        "how to open quartz release",
        "where is quartz configuration stored on macos",
        "what version is topaz2 protocol",
        "does quartz clipboard not survive restarts",
        "what was topaz protocol version in 2020",
        "how to copy beta to alpha",
        "how to open quartz",
        "how to open topaz debug",
        "quartz debug remotely",
        "quartz without debug",
        "stop quartz debug",
        "restart quartz debug",
        "topaz2 protocol version",
        "quartz monorepo license",
        "quartz configuration on macos",
        "copy beta to alpha",
        "quartz clipboard not survive restarts",
        "C# documentation",
        "C documentation",
        "run quartz",
        "quartz quartz",
    ] {
        let result = fixture.ask(query, 5);
        assert!(result["answer"].is_null(), "{query}: {result}");
    }
}

#[test]
fn shorthand_preserves_symbols_and_unicode_in_project_names() {
    let fixture = Fixture::new(&[
        capture(
            "cpp",
            "Q: Where is C++ documentation stored? A: /fixture/cpp/docs",
        ),
        capture(
            "csharp",
            "Q: Where is C# documentation stored? A: /fixture/csharp/docs",
        ),
        capture(
            "foehn",
            "Q: Where is Føhn configuration stored? A: /fixture/foehn/settings",
        ),
    ]);
    for (query, key) in [
        ("C++ documentation", "cpp"),
        ("C# documentation", "csharp"),
        ("Føhn configuration", "foehn"),
    ] {
        assert_eq!(fixture.ask(query, 1)["answer"]["key"], key, "{query}");
    }
    assert!(fixture.ask("C documentation", 1)["answer"].is_null());
}

#[test]
fn shorthand_keeps_conflicting_modes_and_questions_as_choices() {
    for extra in [
        capture(
            "normal",
            "Q: How to run Quartz in normal mode? A: Run ./quartz-forge play.",
        ),
        capture(
            "debug-reason",
            "Q: Why does Quartz debug fail? A: The graphics driver is incompatible.",
        ),
    ] {
        let query = if extra["key"] == "normal" {
            "quartz mode"
        } else {
            "quartz debug"
        };
        let fixture = Fixture::new(&[extra]);
        let result = fixture.ask(query, 1);
        assert!(result["answer"].is_null(), "{query}: {result}");
        assert_eq!(result["verdict"], "candidates", "{query}: {result}");
        assert!(result["signals"]["conflicting_captures"].as_u64().unwrap() >= 2);
    }
}

#[test]
fn command_annotations_do_not_hide_different_arguments() {
    for answer in [
        "./quartz-forge dev (without -d)",
        "./quartz-forge release (or -r)",
        "./quartz-forge dev --remote",
        "./Quartz-forge dev (or -d)",
    ] {
        let fixture = Fixture::new(&[capture(
            "different-command",
            &format!("Q: What command is Quartz debug mode? A: {answer}"),
        )]);
        let result = fixture.ask("quartz debug", 1);
        assert_eq!(result["verdict"], "candidates", "{answer}: {result}");
        assert!(result["answer"].is_null());
    }
}

#[test]
fn feedback_session_exclusions_and_private_homes_apply_before_fact_selection() {
    let mut private = capture(
        "private-conflict",
        "Python. What language is quartz monorepo: the private workspace uses Python.",
    );
    private["agent_home"] = json!("/fixture/private-agent");
    let fixture = Fixture::new(&[private]);
    let query = "what language is quartz monorepo";
    assert_eq!(fixture.ask(query, 5)["answer"]["key"], "language-a");
    qol_memory::feedback::append_vote(
        fixture.store.root(),
        &qol_memory::retrieval_log::normalize_query(query),
        "language-a",
        -1,
    );
    assert_eq!(fixture.ask(query, 5)["answer"]["key"], "language-b");
    let out = ask::run(
        &fixture.store,
        &AliasMap::default(),
        &AskRequest {
            query: query.into(),
            k: 5,
            brief: false,
            exclude_session: Some("settled".into()),
            agent_home: Some(HOME.into()),
        },
    )
    .unwrap();
    assert!(out.answer.is_none());
}

#[test]
fn contradictory_captures_abstain_even_if_one_repeats_the_exact_question() {
    let fixture = Fixture::new(&[capture(
        "language-conflict",
        "Python. What language is the quartz monorepo: Python is used throughout.",
    )]);
    for (query, _) in cases().into_iter().filter(|(_, key)| *key == "language-a") {
        let result = fixture.ask(query, 5);
        assert_eq!(result["verdict"], "candidates", "{query}: {result}");
        assert!(result["answer"].is_null(), "{query}");
        assert_eq!(result["signals"]["conflicting_captures"], 3, "{query}");
        assert!(
            result["reason"].as_str().unwrap().contains("conflict"),
            "{query}: {result}"
        );
    }
}

#[test]
fn path_case_differences_remain_conflicting_answers() {
    let fixture = Fixture::new(&[capture(
        "config-case",
        "Q: Where is the quartz configuration stored? A: /fixture/quartz/Settings.toml",
    )]);
    let result = fixture.ask("where is quartz configuration stored", 5);
    assert!(result["answer"].is_null(), "{result}");
    assert_eq!(result["signals"]["conflicting_captures"], 2);
}

#[test]
fn daemon_rows_and_cold_ask_select_the_same_fact() {
    let caller = qol_agent_homes::Registry::load().resolve_caller(None);
    let fixture = Fixture::in_home(&[], &caller);
    let mut state = Arc::new(Mutex::new(
        WarmState::open(fixture.store.clone(), AliasMap::default()).unwrap(),
    ));
    for (query, key) in cases() {
        let result = daemon_call(
            &mut state,
            "rows",
            json!({"query":query,"agent_home":caller,"no_log":true}),
        );
        assert_eq!(result["verdict"], "answered", "{query}: {result}");
        assert_eq!(result["rows"][0]["kind"], "answer", "{query}");
        assert_eq!(result["rows"][0]["key"], key, "{query}: {result}");
        assert_eq!(
            result["rows"][0]["key"],
            fixture.ask(query, 5)["answer"]["key"]
        );
    }

    let query = "where is quartz configuration stored";
    let conflict = capture(
        "config-case",
        "Q: Where is the quartz configuration stored? A: /fixture/quartz/Settings.toml",
    );
    let appended = daemon_call(
        &mut state,
        "capture",
        json!({"unit":conflict,"agent_home":caller}),
    );
    assert_eq!(appended["appended"], 1, "{appended}");
    for _ in 0..2 {
        let result = daemon_call(
            &mut state,
            "rows",
            json!({"query":query,"agent_home":caller,"no_log":true}),
        );
        assert_eq!(result["verdict"], "candidates", "{result}");
        assert!(fixture.ask(query, 5)["answer"].is_null());
        state.lock().unwrap().invalidate_layers();
    }
}

fn daemon_call(state: &mut Arc<Mutex<WarmState>>, action: &str, input: Value) -> Value {
    let ReadResult::HandledWithData(result) = request::handle(
        state,
        &DaemonRequest {
            action: action.into(),
            input,
        },
    ) else {
        panic!("no data for {action}");
    };
    result
}
