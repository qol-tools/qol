use std::fs;
use std::path::Path;

use serde::Deserialize;

use plugin_cli_sessions::attention::{reduce, Attention, Evidence, GRACE_SECS};
use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::tool::{from_cli_session, is_generic};
use qol_terminal_sessions::cli::CliSessionInterpreter;

#[derive(Deserialize)]
struct Meta {
    title: String,
    #[serde(default)]
    at_prompt: bool,
    foreground_basenames: Vec<String>,
    screen_changed: bool,
    #[serde(default)]
    prev: Option<String>,
    expect: Option<String>,
}

fn expected_status(name: &str) -> Status {
    match name {
        "Working" => Status::Working,
        "Service" => Status::Service,
        "YourTurn" => Status::YourTurn,
        "NeedsYou" => Status::NeedsYou,
        "Unknown" | "Idle" => Status::Unknown,
        "Acknowledged" => Status::Acknowledged,
        other => panic!("unknown expect value {other:?}"),
    }
}

fn classify_frame(meta: &Meta, screen: &str) -> Status {
    let pane = Pane {
        id: kitty_session_id(0),
        root_pid: 0,
        cwd: String::new(),
        title: meta.title.clone(),
        at_prompt: meta.at_prompt,
        reported_cmd: None,
        foreground_basenames: meta.foreground_basenames.clone(),
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
        spawn_identity: None,
    };
    let interpreter = CliSessionInterpreter::system();
    let cli_session = interpreter.describe(&pane);
    let tool = from_cli_session(&cli_session);
    let screen_evidence = interpreter.classify_screen(&pane, screen);
    let prev_status = meta
        .prev
        .as_deref()
        .map(expected_status)
        .unwrap_or(Status::Unknown);
    let prev = Attention {
        status: prev_status,
        working_since: meta.prev.as_ref().map(|_| 0),
        settled_since: meta.prev.as_ref().map(|_| 0),
    };
    let evidence = Evidence {
        descriptor_runtime: cli_session.evidence.runtime,
        screen_runtime: screen_evidence.runtime,
        viewport: screen_evidence.viewport,
        file_fresh: cli_session.evidence.activity.file_fresh,
        file_quiet_secs: cli_session.evidence.activity.file_quiet_secs,
        screen_changed: meta.screen_changed,
        at_prompt: meta.at_prompt,
        is_generic: is_generic(&tool),
        is_service: false,
    };
    reduce(&prev, &evidence, GRACE_SECS + 1).attention.status
}

#[test]
fn real_capture_corpus_classifies_as_labeled() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/corpus");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("corpus dir must exist") {
        let path = entry.unwrap().path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some(label) = name.strip_suffix(".meta.json") else {
            continue;
        };
        let meta: Meta =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).expect("valid meta json");
        let Some(expect) = meta.expect.as_deref() else {
            continue;
        };
        let screen = fs::read_to_string(dir.join(format!("{label}.txt")))
            .unwrap_or_else(|_| panic!("missing screen for {label}"));
        let got = classify_frame(&meta, &screen);
        assert_eq!(
            got,
            expected_status(expect),
            "fixture {label}: classified {got:?}, expected {expect}"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "corpus must contain at least one labeled frame"
    );
}
