use std::fs;
use std::path::Path;

use serde::Deserialize;

use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::{for_tool, status_for, Ctx};
use plugin_cli_sessions::tool::Tool;
use qol_terminal_sessions::cli::CliSessionInterpreter;

#[derive(Deserialize)]
struct Meta {
    title: String,
    #[serde(default)]
    at_prompt: bool,
    foreground_basenames: Vec<String>,
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
    };
    let cli_session = CliSessionInterpreter::system().describe(&pane);
    let tool = Tool::from_cli_session(&cli_session);
    let strategy = for_tool(tool);
    let reading = strategy.read(&Ctx {
        pane: &pane,
        cli_session,
        screen: Some(screen),
        screen_changed: true,
        prev: None,
        now: 0,
        is_service: false,
    });
    status_for(Status::Unknown, reading.phase)
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
