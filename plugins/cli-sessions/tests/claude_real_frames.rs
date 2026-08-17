use plugin_cli_sessions::attention::{reduce, Attention, Evidence, GRACE_SECS};
use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{CliRuntimeState, CliSessionInterpreter, CliViewportState};

const WORKING_WITH_TASKLIST: &str = include_str!("fixtures/claude_real/working_win1.txt");
const STATUS_BELOW_FOOTER: &str = include_str!("fixtures/claude_real/status_below_footer.txt");

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
        file_quiet_secs: fresh.map(|is_fresh| if is_fresh { 0 } else { 600 }),
        screen_changed: changed,
        at_prompt: false,
        is_generic: false,
        is_service: false,
    }
}

#[test]
fn a_spinner_above_the_session_footer_still_reads_as_live_work() {
    let interpreter = CliSessionInterpreter::system();
    let evidence = interpreter.classify_screen(&pane(), STATUS_BELOW_FOOTER);
    assert_eq!(evidence.runtime, CliRuntimeState::Working);
    assert_eq!(evidence.viewport, CliViewportState::Live);
}

#[test]
fn a_missing_transcript_is_not_an_authoritative_ready_and_the_screen_verdict_holds() {
    let evidence = evidence_for(WORKING_WITH_TASKLIST, Some(true), false);
    assert_eq!(evidence.screen_runtime, CliRuntimeState::Working);
    assert_eq!(evidence.descriptor_runtime, CliRuntimeState::Unknown);
    let prev = Attention {
        status: Status::Working,
        working_since: Some(0),
        settled_since: Some(0),
    };
    let out = reduce(&prev, &evidence, GRACE_SECS + 1);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "without a transcript the screen's working verdict holds instead of settling"
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
