use plugin_cli_sessions::attention::{reduce, Attention, Evidence, GRACE_SECS};
use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::signal::screen::{screen_hash, stable_screen};
use plugin_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{
    claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool, CliRuntimeState as RT,
    CliSessionInterpreter, CliViewportState,
};

fn pi_pane() -> Pane {
    Pane {
        id: kitty_session_id(1),
        root_pid: 1,
        cwd: "/a/proj".into(),
        title: "\u{03C0} - proj".into(),
        at_prompt: false,
        reported_cmd: Some("pi".into()),
        foreground_basenames: vec!["pi".into()],
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
        spawn_identity: None,
    }
}

#[test]
fn screen_hash_changes_with_content_and_is_stable_for_equal_text() {
    assert_eq!(screen_hash("same"), screen_hash("same"));
    assert_ne!(
        screen_hash("a"),
        screen_hash("b"),
        "different text hashes differ"
    );
}

#[test]
fn non_tool_screens_pass_through_unchanged() {
    let text = "plain output stays as-is";
    for tool in [claude_tool(), codex_tool(), generic_tool()] {
        assert_eq!(stable_screen(text, &tool).as_ref(), text, "tool: {tool:?}");
    }
}

#[test]
fn pi_footer_counter_changes_do_not_count_as_movement() {
    let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
    let base = format!(
        "conversation output\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
    );
    let footer = format!(
        "conversation output\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.480 (sub) 30.1%/1.0M (auto)"
    );
    let content = format!(
        "new output arrived\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
    );
    let hash = |text: &str| screen_hash(stable_screen(text, &pi_tool()).as_ref());
    assert_eq!(
        hash(&base),
        hash(&footer),
        "footer counters must not count as movement"
    );
    assert_ne!(
        hash(&base),
        hash(&content),
        "content changes must count as movement"
    );
}

#[test]
fn kimi_status_bar_changes_do_not_count_as_movement() {
    let boxed = "\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}";
    let base = format!(
        "conversation output\n{boxed}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)"
    );
    let status = format!(
        "conversation output\n{boxed}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 21% (51.5k/256k)"
    );
    let content = format!(
        "new output arrived\n{boxed}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)"
    );
    let hash = |text: &str| screen_hash(stable_screen(text, &kimi_tool()).as_ref());
    assert_eq!(
        hash(&base),
        hash(&status),
        "status bar changes must not count as movement"
    );
    assert_ne!(
        hash(&base),
        hash(&content),
        "content changes must count as movement"
    );
}

#[test]
fn every_live_pi_footer_height_is_stabilizable() {
    let pane = pi_pane();
    let interpreter = CliSessionInterpreter::system();
    let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
    let hash = |text: &str| screen_hash(stable_screen(text, &pi_tool()).as_ref());
    let build = |below: usize, cost: &str, content: &str| {
        let mut lines = vec![
            content.to_owned(),
            "\u{280B} Working...".to_owned(),
            rule.to_owned(),
            rule.to_owned(),
        ];
        for index in 0..below {
            if index + 1 == below {
                lines.push(cost.to_owned());
            } else {
                lines.push(format!("footer line {index}"));
            }
        }
        lines.join("\n")
    };
    for below in 0..=8 {
        let base = build(
            below,
            "$0.000 (sub) 0.0%/262k (auto)",
            "conversation output",
        );
        let evidence = interpreter.classify_screen(&pane, &base);
        let live = evidence.viewport == CliViewportState::Live;
        assert_eq!(live, (1..=6).contains(&below), "below={below}");
        if !live {
            continue;
        }
        let bumped = build(
            below,
            "$0.480 (sub) 30.1%/1.0M (auto)",
            "conversation output",
        );
        let changed = build(below, "$0.000 (sub) 0.0%/262k (auto)", "new output arrived");
        assert_eq!(
            hash(&base),
            hash(&bumped),
            "footer counters must not count as movement at below={below}"
        );
        assert_ne!(
            hash(&base),
            hash(&changed),
            "content changes must count as movement at below={below}"
        );
    }
}

#[test]
fn real_pi_frames_classify_live_and_stabilize() {
    const FRAMES: [&str; 6] = [
        include_str!("../../../libs/qol-terminal-sessions/tests/fixtures/pi_real/completion_line.txt"),
        include_str!("../../../libs/qol-terminal-sessions/tests/fixtures/pi_real/frozen_spinner_a.txt"),
        include_str!("../../../libs/qol-terminal-sessions/tests/fixtures/pi_real/frozen_spinner_b.txt"),
        include_str!("../../../libs/qol-terminal-sessions/tests/fixtures/pi_real/prompt_echo_with_token.txt"),
        include_str!("../../../libs/qol-terminal-sessions/tests/fixtures/pi_real/provider_error_terminated.txt"),
        include_str!("../../../libs/qol-terminal-sessions/tests/fixtures/pi_real/token_in_editor.txt"),
    ];
    let pane = pi_pane();
    let interpreter = CliSessionInterpreter::system();
    let hash = |text: &str| screen_hash(stable_screen(text, &pi_tool()).as_ref());
    for frame in FRAMES {
        let at_rest =
            frame.contains("Both tests land as described") || frame.contains("Operation aborted");
        let (viewport, runtime) = if at_rest {
            (CliViewportState::Unknown, RT::Unknown)
        } else {
            (CliViewportState::Live, RT::Working)
        };
        let evidence = interpreter.classify_screen(&pane, frame);
        assert_eq!(evidence.viewport, viewport, "viewport: {frame}");
        assert_eq!(evidence.runtime, runtime, "runtime: {frame}");
        let trimmed = stable_screen(frame, &pi_tool());
        assert!(
            !trimmed.contains("MCP: 3 servers enabled"),
            "the footer must be trimmed: {frame}"
        );
        assert_eq!(
            hash(frame),
            hash(&frame.replace("%/1.0M", "%/2.0M")),
            "a real footer counter change must not count as movement"
        );
    }
}

#[test]
fn a_settled_pi_screen_with_stable_footer_completes_after_grace() {
    let pane = pi_pane();
    let interpreter = CliSessionInterpreter::system();
    let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
    let screen = format!(
        "conversation output\n\u{280B} Working...\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
    );
    let screen_evidence = interpreter.classify_screen(&pane, &screen);
    let evidence = Evidence {
        descriptor_runtime: RT::Unknown,
        screen_runtime: screen_evidence.runtime,
        viewport: screen_evidence.viewport,
        file_fresh: Some(false),
        file_quiet_secs: Some(600),
        screen_changed: false,
        at_prompt: false,
        is_generic: false,
        is_service: false,
    };
    let prev = Attention {
        status: Status::Working,
        working_since: Some(0),
        settled_since: Some(0),
    };
    let out = reduce(&prev, &evidence, GRACE_SECS + 1);
    assert_eq!(
        out.attention.status,
        Status::YourTurn,
        "a settled stale loader is your turn"
    );
}
