use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::signal::screen::screen_hash;
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::claude::Claude;
use plugin_cli_sessions::strategy::codex::Codex;
use plugin_cli_sessions::strategy::kimi::Kimi;
use plugin_cli_sessions::strategy::pi::Pi;
use plugin_cli_sessions::strategy::{
    phase_for, running_since_for, status_for, Cli, Ctx, Phase, Prev, Strategy,
};
use qol_terminal_sessions::cli::{
    codex_tool, kimi_tool, pi_tool, CliSessionDescriptor, CliSessionInterpreter,
};

fn pane(at_prompt: bool, cmd: &str, title: &str) -> Pane {
    Pane {
        id: kitty_session_id(1),
        root_pid: 1,
        cwd: "/a/proj".into(),
        title: title.into(),
        at_prompt,
        reported_cmd: Some(cmd.into()),
        foreground_basenames: vec![cmd.into()],
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
    }
}

fn ctx<'a>(
    p: &'a Pane,
    screen: Option<&'a str>,
    changed: bool,
    prev: Option<Prev>,
    now: u64,
) -> Ctx<'a> {
    let cli_session = CliSessionInterpreter::system().describe(p);
    Ctx {
        pane: p,
        cli_session,
        screen,
        screen_changed: changed,
        prev,
        now,
        is_service: false,
    }
}

fn codex_ctx<'a>(
    p: &'a Pane,
    screen: Option<&'a str>,
    name: Option<&str>,
    has_activity: Option<bool>,
) -> Ctx<'a> {
    Ctx {
        pane: p,
        cli_session: CliSessionDescriptor {
            tool: codex_tool(),
            display_name: name.map(str::to_owned),
            external_id: Some("test-session".to_owned()),
            has_activity,
        },
        screen,
        screen_changed: false,
        prev: None,
        now: 0,
        is_service: false,
    }
}

fn ran_since(start: u64) -> Option<Prev> {
    Some(Prev {
        status: Status::Working,
        running_since: Some(start),
    })
}

fn pi_ctx<'a>(
    p: &'a Pane,
    screen: Option<&'a str>,
    name: Option<&str>,
    has_activity: Option<bool>,
) -> Ctx<'a> {
    Ctx {
        pane: p,
        cli_session: CliSessionDescriptor {
            tool: pi_tool(),
            display_name: name.map(str::to_owned),
            external_id: Some("019fc6e8-18a0-7983-9fd6-0200f1e9a72b".to_owned()),
            has_activity,
        },
        screen,
        screen_changed: false,
        prev: None,
        now: 0,
        is_service: false,
    }
}

fn kimi_ctx<'a>(
    p: &'a Pane,
    screen: Option<&'a str>,
    name: Option<&str>,
    has_activity: Option<bool>,
) -> Ctx<'a> {
    Ctx {
        pane: p,
        cli_session: CliSessionDescriptor {
            tool: kimi_tool(),
            display_name: name.map(str::to_owned),
            external_id: Some("session_40794504-36ff-4315-be5a-5357029b19e1".to_owned()),
            has_activity,
        },
        screen,
        screen_changed: false,
        prev: None,
        now: 0,
        is_service: false,
    }
}

#[test]
fn status_for_maps_every_phase_with_ack_carry() {
    let cases = [
        (Status::Unknown, Phase::Busy, Status::Working),
        (Status::Unknown, Phase::Blocked, Status::NeedsYou),
        (Status::Working, Phase::Done, Status::YourTurn),
        (Status::Unknown, Phase::Idle, Status::Unknown),
        (Status::Unknown, Phase::Service, Status::Service),
        (Status::Acknowledged, Phase::Done, Status::Acknowledged),
        (Status::Acknowledged, Phase::Idle, Status::Acknowledged),
        (Status::Acknowledged, Phase::Busy, Status::Working),
    ];
    for (prev, phase, expected) in cases {
        assert_eq!(status_for(prev, phase), expected, "{prev:?}+{phase:?}");
    }
}

#[test]
fn running_since_tracks_busy_phase_only() {
    let cases = [
        (None, Phase::Busy, 100, Some(100)),
        (Some(50), Phase::Busy, 100, Some(50)),
        (None, Phase::Service, 100, Some(100)),
        (Some(50), Phase::Service, 100, Some(50)),
        (Some(50), Phase::Blocked, 100, None),
        (Some(50), Phase::Done, 100, None),
        (Some(50), Phase::Idle, 100, None),
    ];
    for (prev, phase, now, expected) in cases {
        assert_eq!(
            running_since_for(prev, phase, now),
            expected,
            "{prev:?}+{phase:?}"
        );
    }
}

const EVERY_PREV: [Option<Status>; 6] = [
    None,
    Some(Status::Working),
    Some(Status::Unknown),
    Some(Status::YourTurn),
    Some(Status::Acknowledged),
    Some(Status::NeedsYou),
];

fn moving_prev_is_unsettled(prev: Option<Status>) -> bool {
    matches!(prev, None | Some(Status::Working))
}

#[test]
fn phase_for_maps_all_evidence_combinations() {
    let settled = [
        (false, false, false, false, Phase::Idle),
        (false, false, false, true, Phase::Busy),
        (false, false, true, false, Phase::Done),
        (false, false, true, true, Phase::Done),
        (false, true, false, false, Phase::Blocked),
        (false, true, false, true, Phase::Blocked),
        (false, true, true, false, Phase::Blocked),
        (false, true, true, true, Phase::Blocked),
        (true, false, false, false, Phase::Busy),
        (true, false, false, true, Phase::Busy),
        (true, false, true, false, Phase::Busy),
        (true, false, true, true, Phase::Busy),
        (true, true, false, false, Phase::Busy),
        (true, true, false, true, Phase::Busy),
        (true, true, true, false, Phase::Busy),
        (true, true, true, true, Phase::Busy),
    ];
    assert_eq!(settled.len(), 16);
    for prev in EVERY_PREV {
        for (working, awaiting, turn_taken, changed, settled_expected) in settled {
            let expected = if changed && !working && moving_prev_is_unsettled(prev) {
                Phase::Busy
            } else {
                settled_expected
            };
            assert_eq!(
                phase_for(working, awaiting, turn_taken, changed, prev),
                expected,
                "working={working} awaiting={awaiting} turn_taken={turn_taken} screen_changed={changed} prev={prev:?}"
            );
        }
    }
}

#[test]
fn a_moving_screen_never_reads_done_before_the_session_has_settled() {
    for prev in [None, Some(Status::Working)] {
        assert_eq!(
            phase_for(false, false, true, true, prev),
            Phase::Busy,
            "historical turn evidence must not beat live movement for prev={prev:?}"
        );
        assert_eq!(
            phase_for(false, true, false, true, prev),
            Phase::Busy,
            "a moving choice prompt must settle before it reads blocked for prev={prev:?}"
        );
    }
}

#[test]
fn a_settled_session_keeps_its_waiting_phase_across_a_redraw() {
    for prev in [
        Some(Status::Unknown),
        Some(Status::YourTurn),
        Some(Status::Acknowledged),
        Some(Status::NeedsYou),
    ] {
        assert_eq!(phase_for(false, false, true, true, prev), Phase::Done);
        assert_eq!(phase_for(false, true, false, true, prev), Phase::Blocked);
    }
}

#[test]
fn a_redraw_without_waiting_evidence_still_reads_busy() {
    for prev in EVERY_PREV {
        assert_eq!(
            phase_for(false, false, false, true, prev),
            Phase::Busy,
            "movement is the only working signal a hidden spinner leaves for prev={prev:?}"
        );
    }
}

#[test]
fn first_observation_of_a_moving_session_never_arms_your_turn() {
    let phase = phase_for(false, false, true, true, None);
    assert_eq!(phase, Phase::Busy);
    assert_eq!(status_for(Status::Unknown, phase), Status::Working);
}

#[test]
fn cli_service_flag_reads_live_unless_at_prompt() {
    let running = pane(false, "qol dev", "qol dev");
    let mut live = ctx(&running, Some("compiling assets"), true, None, 100);
    live.is_service = true;
    assert_eq!(
        Cli.read(&live).phase,
        Phase::Service,
        "a running generic process flagged as a service reads live"
    );

    let at_prompt = pane(true, "qol dev", "qol dev");
    let mut idle = ctx(&at_prompt, None, false, None, 100);
    idle.is_service = true;
    assert_eq!(
        Cli.read(&idle).phase,
        Phase::Idle,
        "an at-prompt pane is never live even when flagged"
    );
}

#[test]
fn cli_default_phase_from_lifecycle() {
    let running = pane(false, "npm i", "x");
    let r = Cli.read(&ctx(&running, Some("compiling"), true, None, 100));
    assert_eq!(r.phase, Phase::Busy);

    let blocked = Cli.read(&ctx(
        &running,
        Some("Overwrite? [y/N]"),
        false,
        ran_since(50),
        100,
    ));
    assert_eq!(blocked.phase, Phase::Blocked);

    let at_prompt = pane(true, "ls", "x");
    assert_eq!(
        Cli.read(&ctx(&at_prompt, None, false, None, 100)).phase,
        Phase::Idle
    );
    assert_eq!(
        Cli.read(&ctx(&at_prompt, None, false, ran_since(90), 100))
            .phase,
        Phase::Done,
        "finished after a long run => your turn"
    );
    assert_eq!(
        Cli.read(&ctx(&at_prompt, None, false, ran_since(98), 100))
            .phase,
        Phase::Idle,
        "a quick command does not flash done"
    );
}

#[test]
fn claude_choice_picker_without_caret_is_blocked() {
    let p = pane(false, "claude", "\u{2733} uninstaller");
    let screen = "How should the uninstaller be invoked?\n\n1. gpui popup picker\n2. Keep terminal CLI\n3. Both: popup + CLI\n\nEnter to select \u{00B7} \u{2191}/\u{2193} to navigate \u{00B7} n to add notes \u{00B7} Esc to cancel";
    assert_eq!(
        Claude.read(&ctx(&p, Some(screen), false, None, 0)).phase,
        Phase::Blocked,
        "an on-screen choice picker is needs-you even when the caret glyph is absent"
    );
}

#[test]
fn claude_phase_from_screen() {
    let p = pane(false, "claude", "\u{2733} Topic");
    let cases = [
        ("\u{273B} Brewed for 2m 9s", Phase::Done),
        (
            "\u{273B} Processing\u{2026} (5s \u{00B7} \u{2193} 1k tokens)",
            Phase::Busy,
        ),
        (
            "\u{276F} 1. Yes\n  2. No\n  enter to confirm",
            Phase::Blocked,
        ),
        (
            "1. step one\n2. step two\n\u{273B} Working\u{2026} (5s \u{00B7} esc to interrupt)",
            Phase::Busy,
        ),
        (
            "Discover plugins (1/221)\n\u{276F} o code-review\n  o github\n  Type to search \u{00B7} Space to toggle \u{00B7} Enter to view",
            Phase::Idle,
        ),
        ("Welcome to Claude Code\n\u{276F} ", Phase::Idle),
    ];
    for (screen, expected) in cases {
        assert_eq!(
            Claude.read(&ctx(&p, Some(screen), false, None, 0)).phase,
            expected,
            "screen: {screen:?}"
        );
    }
}

#[test]
fn claude_working_title_overrides_scrolled_screen() {
    let scrolled = "1. step\n2. step\n  [ ] todo\n  enter to confirm";
    let star = pane(false, "claude", "\u{2736} Improve logging");
    assert_eq!(
        Claude
            .read(&ctx(&star, Some(scrolled), false, None, 0))
            .phase,
        Phase::Busy,
        "a working title beats scrolled-up marker-like content (no false needs-you)"
    );
    let braille = pane(false, "claude", "\u{2810} Improve logging");
    assert_eq!(
        Claude
            .read(&ctx(&braille, Some("some scrolled code"), false, None, 0))
            .phase,
        Phase::Busy
    );
    let parked = pane(false, "claude", "\u{2733} Improve logging");
    assert_eq!(
        Claude
            .read(&ctx(
                &parked,
                Some("\u{273B} Brewed for 1m"),
                false,
                None,
                0
            ))
            .phase,
        Phase::Done,
        "a parked title does not force busy"
    );
}

#[test]
fn claude_label_strips_status_glyph() {
    let mut p = pane(false, "claudedw", "\u{2733} Improve logging");
    p.foreground_basenames = vec!["claude".to_owned()];
    let r = Claude.read(&ctx(&p, Some(""), false, None, 0));
    assert_eq!(r.label.as_deref(), Some("Improve logging"));
}

#[test]
fn pi_working_loader_is_busy() {
    let p = pane(false, "pi", "\u{03C0} - qol-monorepo");
    let cases = [
        "\u{280B} Working...",
        "\u{2819} Working... (esc to interrupt)",
        "\u{280B} Retrying (1/3) in 4s... (esc to cancel)",
        "\u{2838} Compacting... (esc to cancel)",
        "conversation output\n\u{2807} Working...\n\n\u{2500}\u{2500}\u{2500}\n/tmp\n$0.000 (sub) 9.4%/262k (auto)",
    ];
    for screen in cases {
        assert_eq!(
            Pi.read(&pi_ctx(&p, Some(screen), Some("proj"), Some(true)))
                .phase,
            Phase::Busy,
            "screen: {screen:?}"
        );
    }
}

#[test]
fn pi_selector_is_blocked() {
    let p = pane(false, "pi", "\u{03C0} - proj");
    let screen = "Replace current session with /tmp/x.jsonl?\n\n\u{276F} Yes\n  No\n\n\u{2191}\u{2193} navigate  enter select  esc cancel";
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some(screen), Some("proj"), Some(true)))
            .phase,
        Phase::Blocked,
        "an extension selector waits for the user"
    );
}

#[test]
fn pi_hint_in_conversation_text_does_not_false_positive() {
    let p = pane(false, "pi", "\u{03C0} - proj");
    // A conversation that discusses the selector hint format — but it's
    // scrolled up, so the tail is clean (editor + footer only).
    let mut screen = String::new();
    // Enough filler lines to push any hint mentions out of the tail-8 window.
    for i in 0..20 {
        screen.push_str(&format!("conversation line {i}\n"));
    }
    screen.push_str(
        "The selector renders a hint: \u{2191}\u{2193} navigate  enter select  esc cancel\n",
    );
    screen.push_str("and it's shown at the bottom.\n");
    for i in 30..50 {
        screen.push_str(&format!("more conversation line {i}\n"));
    }
    // Tail: editor borders + footer (clean, no selector hint).
    screen.push_str("\n\u{2500}\u{2500}\u{2500}\n\n\u{2500}\u{2500}\u{2500}\n");
    screen.push_str("/tmp\n");
    screen.push_str("$0.000 (sub) 0.0%/262k (auto)");
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some(&screen), Some("proj"), Some(true)))
            .phase,
        Phase::Done,
        "a selector hint buried in conversation text must not block"
    );
}

#[test]
fn pi_idle_footer_does_not_read_as_working() {
    let p = pane(false, "pi", "\u{03C0} - proj");
    let idle = "\u{2500}\u{2500}\u{2500}\n\n\u{2500}\u{2500}\u{2500}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)  (kimi-coding) k3-256k \u{2022} high";
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some(idle), Some("proj"), Some(true)))
            .phase,
        Phase::Done,
        "the idle footer bullet must not look like a spinner"
    );
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some(idle), Some("proj"), Some(false)))
            .phase,
        Phase::Idle,
        "a fresh pi with no messages is idle"
    );
}

#[test]
fn pi_idle_via_banner_when_activity_metadata_is_absent() {
    let p = pane(false, "pi", "\u{03C0} - proj");
    let fresh = " pi v0.83.0\n escape interrupt \u{00B7} ctrl+c/ctrl+d clear/exit\n Press ctrl+o to show full startup help and loaded resources.";
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some(fresh), Some("proj"), None)).phase,
        Phase::Idle,
        "startup help => fresh => idle"
    );
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some("conversation output"), Some("proj"), None))
            .phase,
        Phase::Done,
        "no banner => a turn happened"
    );
}

#[test]
fn pi_label_comes_from_the_descriptor() {
    let p = pane(false, "pi", "\u{03C0} - Refactor auth module - proj");
    let r = Pi.read(&pi_ctx(
        &p,
        Some(""),
        Some("Refactor auth module"),
        Some(true),
    ));
    assert_eq!(r.label.as_deref(), Some("Refactor auth module"));
}

#[test]
fn kimi_moon_spinner_line_is_busy() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let cases = [
        "\u{1F315} \u{00B7} Tip: /tasks to check progress and status for background tasks",
        "\u{1F315}\u{FE0F} \u{00B7} Tip: /tasks to check progress and status for background tasks",
        "\u{1F312} \u{00B7} Tip: /sessions to browse and resume earlier sessions",
        "conversation output\n\u{1F314} \u{00B7} Tip: Try /dance for a hidden Easter egg",
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\n\u{1F313} \u{00B7} Tip: run /help",
    ];
    for screen in cases {
        assert_eq!(
            Kimi.read(&kimi_ctx(&p, Some(screen), Some("proj"), Some(true)))
                .phase,
            Phase::Busy,
            "screen: {screen:?}"
        );
    }
}

#[test]
fn kimi_moon_beyond_the_status_window_does_not_read_as_busy() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let mut screen = String::new();
    for i in 0..40 {
        screen.push_str(&format!("conversation line {i}\n"));
    }
    screen.push_str("\u{1F315} \u{00B7} Tip: a turn that mentioned the spinner\n");
    for i in 40..75 {
        screen.push_str(&format!("more conversation line {i}\n"));
    }
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(&screen), Some("proj"), Some(true)))
            .phase,
        Phase::Done,
        "a moon emoji scrolled far above the status area must not look live"
    );
}

#[test]
fn kimi_moon_led_conversation_line_is_not_busy() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let screen = "Assistant answer:\n\u{1F315} release status is green\n> ";
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(screen), Some("proj"), Some(true)))
            .phase,
        Phase::Done
    );
}

#[test]
fn kimi_idle_screen_is_done_after_a_turn_and_idle_when_fresh() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let idle = "\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}\nyolo  DeepSeek V4 Flash thinking: max  \u{2026}/qol-monorepo  main [\u{2191}5]";
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(idle), Some("proj"), Some(true)))
            .phase,
        Phase::Done,
        "a settled screen after a turn is your-turn"
    );
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(idle), Some("proj"), Some(false)))
            .phase,
        Phase::Idle,
        "a fresh session with no prompts is idle"
    );
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(idle), Some("proj"), None))
            .phase,
        Phase::Done,
        "without metadata a settled screen counts as a turn taken"
    );
}

#[test]
fn kimi_choice_picker_is_blocked() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let screen = "How should the uninstaller be invoked?\n\n1. gpui popup picker\n2. Keep terminal CLI\n3. Both: popup + CLI\n\nEnter to select \u{00B7} \u{2191}/\u{2193} to navigate \u{00B7} n to add notes \u{00B7} Esc to cancel";
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(screen), Some("proj"), Some(true)))
            .phase,
        Phase::Blocked,
        "an on-screen choice picker is needs-you even when the caret glyph is absent"
    );
}

#[test]
fn kimi_bare_moon_and_braille_spinners_are_working() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let boxed = |spinner: &str| {
        format!(
            "{spinner}\n\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]"
        )
    };
    let cases = [
        boxed("  \u{1F317}"),
        boxed("\u{1F312}\u{FE0F}"),
        boxed("\u{2819} thinking..."),
        boxed("ledger output\n\u{2826} thinking..."),
        boxed("\u{1F315} \u{00B7} Tip: /tasks to check progress"),
    ];
    for screen in cases {
        assert_eq!(
            Kimi.read(&kimi_ctx(&p, Some(&screen), Some("proj"), Some(true)))
                .phase,
            Phase::Busy,
            "screen: {screen:?}"
        );
    }
}

#[test]
fn kimi_bare_moon_in_settled_content_is_not_working() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let screen = "Here is the status:\n\u{1F315}\nThat's the full report.\n\n\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]";
    assert_eq!(
        Kimi.read(&kimi_ctx(&p, Some(screen), Some("proj"), Some(true)))
            .phase,
        Phase::Done,
        "a bare moon inside a completed answer is not a spinner"
    );
}

#[test]
fn pi_stable_hash_tracks_content_not_footer() {
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
    let hash = |s: &str| screen_hash(Pi.stable_screen_hash(s));
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
fn pi_stable_hash_requires_footer_rules_near_the_tail() {
    let rule = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";
    let a = format!(
        "streamed output\n{rule}\nchanging line A\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
    );
    let b = format!(
        "streamed output\n{rule}\nchanging line B\n\n{rule}\n\n{rule}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)"
    );
    let hash = |s: &str| screen_hash(Pi.stable_screen_hash(s));
    assert_ne!(
        hash(&a),
        hash(&b),
        "a rule-looking line inside streamed output must not hide movement below it"
    );
}

#[test]
fn kimi_stable_hash_tracks_content_not_status_bar() {
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
    let hash = |s: &str| screen_hash(Kimi.stable_screen_hash(s));
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
fn pi_streaming_collapsed_output_is_working_until_the_screen_settles() {
    let p = pane(false, "pi", "\u{03C0} - qol-monorepo");
    let screen = "\"WorkingStatusIndicator|start|.stop()|workingMessage\" $PI/dist/modes/interactive/interactive-mode.js | head -25\n ... (15 earlier lines, ctrl+o to expand)\n 3019:            this.ui.start();\n 3024:            this.ui.stop();\n 5065:            this.ui.stop();";
    let mut moving = pi_ctx(&p, Some(screen), Some("proj"), Some(true));
    moving.screen_changed = true;
    moving.prev = ran_since(50);
    assert_eq!(
        Pi.read(&moving).phase,
        Phase::Busy,
        "a session that was working, whose screen is still moving, has not settled"
    );
    let settled = pi_ctx(&p, Some(screen), Some("proj"), Some(true));
    assert_eq!(
        Pi.read(&settled).phase,
        Phase::Done,
        "the same frame once the screen settles is your turn"
    );
}

#[test]
fn moving_screen_from_working_reads_busy_for_every_tool() {
    let pi_pane = pane(false, "pi", "\u{03C0} - proj");
    let pi_settled =
        "conversation output\n\n\u{2500}\u{2500}\u{2500}\n/tmp\n$0.000 (sub) 0.0%/262k (auto)";
    let mut pi_moving = pi_ctx(&pi_pane, Some(pi_settled), Some("proj"), Some(true));
    pi_moving.screen_changed = true;
    pi_moving.prev = ran_since(50);
    assert_eq!(Pi.read(&pi_moving).phase, Phase::Busy);
    assert_eq!(
        Pi.read(&pi_ctx(
            &pi_pane,
            Some(pi_settled),
            Some("proj"),
            Some(true)
        ))
        .phase,
        Phase::Done
    );

    let kimi_pane = pane(false, "kimi-code", "qol-monorepo");
    let kimi_settled = "\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{2191}5]";
    let mut kimi_moving = kimi_ctx(&kimi_pane, Some(kimi_settled), Some("proj"), Some(true));
    kimi_moving.screen_changed = true;
    kimi_moving.prev = ran_since(50);
    assert_eq!(Kimi.read(&kimi_moving).phase, Phase::Busy);
    assert_eq!(
        Kimi.read(&kimi_ctx(
            &kimi_pane,
            Some(kimi_settled),
            Some("proj"),
            Some(true)
        ))
        .phase,
        Phase::Done
    );

    let claude_pane = pane(false, "claude", "\u{2733} Topic");
    let claude_settled = "\u{273B} Brewed for 2m 9s";
    let mut claude_moving = ctx(&claude_pane, Some(claude_settled), true, None, 0);
    claude_moving.prev = ran_since(50);
    assert_eq!(Claude.read(&claude_moving).phase, Phase::Busy);
    assert_eq!(
        Claude
            .read(&ctx(&claude_pane, Some(claude_settled), false, None, 0))
            .phase,
        Phase::Done
    );

    let codex_pane = pane(false, "codex", "qol-monorepo");
    let codex_settled = "What remains:\n1. Add committed golden parity tests\n2. Decide when to remove trace-py\n3. Remove the fallback flag once done\n4. Rename the WIP commit";
    let mut codex_moving = codex_ctx(&codex_pane, Some(codex_settled), Some("Topic"), Some(true));
    codex_moving.screen_changed = true;
    codex_moving.prev = ran_since(50);
    assert_eq!(Codex.read(&codex_moving).phase, Phase::Busy);
    assert_eq!(
        Codex
            .read(&codex_ctx(
                &codex_pane,
                Some(codex_settled),
                Some("Topic"),
                Some(true)
            ))
            .phase,
        Phase::Done
    );
}

#[test]
fn choice_prompt_on_a_moving_screen_from_working_is_busy_not_needs_you() {
    let p = pane(false, "pi", "\u{03C0} - proj");
    let screen = "Replace current session with /tmp/x.jsonl?\n\n\u{276F} Yes\n  No\n\n\u{2191}\u{2193} navigate  enter select  esc cancel";
    let mut moving = pi_ctx(&p, Some(screen), Some("proj"), Some(true));
    moving.screen_changed = true;
    moving.prev = ran_since(50);
    assert_eq!(
        Pi.read(&moving).phase,
        Phase::Busy,
        "a prompt still rendering while the session was working is not settled"
    );
    assert_eq!(
        Pi.read(&pi_ctx(&p, Some(screen), Some("proj"), Some(true)))
            .phase,
        Phase::Blocked,
        "the same prompt once settled needs the user"
    );
}

#[test]
fn idle_rename_keeps_your_turn_not_working() {
    let p = pane(false, "codex", "qol-monorepo");
    let screen = "What remains:\n1. Add committed golden parity tests\n2. Decide when to remove trace-py\n3. Remove the fallback flag once done\n4. Rename the WIP commit";
    let mut renamed = codex_ctx(&p, Some(screen), Some("Topic"), Some(true));
    renamed.screen_changed = true;
    renamed.prev = Some(Prev {
        status: Status::YourTurn,
        running_since: None,
    });
    assert_eq!(
        Codex.read(&renamed).phase,
        Phase::Done,
        "a /rename redraw on an idle session must not read as working"
    );
}

#[test]
fn acknowledged_turn_survives_a_cosmetic_redraw() {
    let p = pane(false, "codex", "qol-monorepo");
    let screen = "What remains:\n1. Add committed golden parity tests\n2. Decide when to remove trace-py\n3. Remove the fallback flag once done\n4. Rename the WIP commit";
    let mut redrawn = codex_ctx(&p, Some(screen), Some("Topic"), Some(true));
    redrawn.screen_changed = true;
    redrawn.prev = Some(Prev {
        status: Status::Acknowledged,
        running_since: None,
    });
    assert_eq!(
        status_for(Status::Acknowledged, Codex.read(&redrawn).phase),
        Status::Acknowledged,
        "acknowledging must not be re-armed by a cosmetic screen change"
    );
}

#[test]
fn kimi_label_comes_from_the_descriptor() {
    let p = pane(false, "kimi-code", "qol-monorepo");
    let r = Kimi.read(&kimi_ctx(
        &p,
        Some(""),
        Some("Refactor auth module"),
        Some(true),
    ));
    assert_eq!(r.label.as_deref(), Some("Refactor auth module"));
}

#[test]
fn codex_answer_ending_in_numbered_list_is_your_turn_not_blocked() {
    let p = pane(false, "codex", "qol-monorepo");
    let answer = "What remains:\n1. Add committed golden parity tests\n2. Decide when to remove trace-py\n3. Remove the fallback flag once done\n4. Rename the WIP commit";
    assert_eq!(
        Codex
            .read(&codex_ctx(&p, Some(answer), Some("Topic"), Some(true)))
            .phase,
        Phase::Done,
        "a finished turn whose answer ends in a numbered list is your-turn, not a blocking prompt"
    );
}

#[test]
fn codex_phase_and_label_from_shared_descriptor() {
    let p = pane(false, "codex", "qol-monorepo");

    let r = Codex.read(&codex_ctx(&p, Some(""), Some("Asasdsadasd"), Some(false)));
    assert_eq!(r.phase, Phase::Idle);
    assert_eq!(r.label.as_deref(), Some("Asasdsadasd"));

    assert_eq!(
        Codex
            .read(&codex_ctx(&p, Some(""), Some("Asasdsadasd"), Some(true),))
            .phase,
        Phase::Done
    );

    let busy_with_sizes = "  1.9G  /home/user/.cache\n\u{2022} Working (3m 56s \u{00B7} esc to interrupt)\n\u{203A} Use /skills";
    let r = Codex.read(&codex_ctx(
        &p,
        Some(busy_with_sizes),
        Some("proj"),
        Some(true),
    ));
    assert_eq!(
        r.phase,
        Phase::Busy,
        "size output on a working screen must not read as a menu/blocked"
    );
    assert_eq!(
        r.label.as_deref(),
        Some("proj"),
        "fall back to project without a name"
    );
}

#[test]
fn codex_working_title_overrides_scrolled_screen() {
    let scrolled = "previous answer\nDone for 12s\n\u{203A} Use /skills";
    let braille = pane(false, "codex", "\u{2810} Fix edge case");
    assert_eq!(
        Codex
            .read(&codex_ctx(
                &braille,
                Some(scrolled),
                Some("Fix edge case"),
                Some(true),
            ))
            .phase,
        Phase::Busy,
        "a live Codex working title beats scrolled-up non-working content"
    );

    let marker_like = "old question\n  [ ] Review diff\n  enter to confirm";
    let star = pane(false, "codex", "\u{2736} Fix edge case");
    assert_eq!(
        Codex
            .read(&codex_ctx(
                &star,
                Some(marker_like),
                Some("Fix edge case"),
                Some(true),
            ))
            .phase,
        Phase::Busy,
        "a live Codex working title beats scrolled-up prompt-like content"
    );
}

#[test]
fn codex_idle_via_banner_when_activity_metadata_is_absent() {
    let p = pane(false, "codex", "qol-monorepo");
    let fresh = Codex.read(&codex_ctx(
        &p,
        Some("\u{203A} >_ OpenAI Codex (v0.141.0)"),
        Some("proj"),
        None,
    ));
    assert_eq!(fresh.phase, Phase::Idle, "welcome banner => fresh => idle");
    let used = Codex.read(&codex_ctx(
        &p,
        Some("conversation output"),
        Some("proj"),
        None,
    ));
    assert_eq!(used.phase, Phase::Done, "no banner => a turn happened");
}
