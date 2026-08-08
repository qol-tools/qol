use plugin_cli_sessions::attention::{reduce, Attention, Evidence, GRACE_SECS};
use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{CliRuntimeState, CliSessionInterpreter};

const WORKING_WITH_TASKLIST: &str = include_str!("fixtures/claude_real/working_win1.txt");

fn pane() -> Pane {
    Pane {
        id: kitty_session_id(0),
        root_pid: 1,
        cwd: "/a/proj".into(),
        title: "proj".into(),
        at_prompt: false,
        reported_cmd: Some("claude".into()),
        foreground_basenames: vec!["claude".into()],
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
        spawn_identity: None,
    }
}

fn evidence_for(screen: &str, fresh: Option<bool>, changed: bool) -> Evidence {
    let interpreter = CliSessionInterpreter::system();
    let cli_session = interpreter.describe(&pane());
    let screen_evidence = interpreter.classify_screen(&pane(), screen);
    Evidence {
        descriptor_runtime: cli_session.evidence.runtime,
        screen_runtime: screen_evidence.runtime,
        viewport: screen_evidence.viewport,
        file_fresh: fresh,
        screen_changed: changed,
        at_prompt: false,
        is_generic: false,
        is_service: false,
    }
}

#[test]
fn a_working_turn_stays_working_when_the_spinner_leaves_the_classifier_window() {
    let evidence = evidence_for(WORKING_WITH_TASKLIST, Some(true), false);
    assert_eq!(
        evidence.screen_runtime,
        CliRuntimeState::Unknown,
        "the shared classifier only reads the recent tail"
    );
    let prev = Attention {
        status: Status::Working,
        working_since: Some(0),
        settled_since: Some(0),
    };
    let out = reduce(&prev, &evidence, GRACE_SECS + 1);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "a fresh transcript keeps the turn working even when the spinner sits above the tail"
    );
}

#[test]
fn the_same_frame_completes_once_the_transcript_goes_stale() {
    let evidence = evidence_for(WORKING_WITH_TASKLIST, Some(false), false);
    let prev = Attention {
        status: Status::Working,
        working_since: Some(0),
        settled_since: Some(0),
    };
    let out = reduce(&prev, &evidence, GRACE_SECS + 1);
    assert_eq!(
        out.attention.status,
        Status::YourTurn,
        "a settled frame with a stale transcript is a completed turn"
    );
}
