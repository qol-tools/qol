use plugin_cli_sessions::host::{kitty_session_id, Pane};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::claude::Claude;
use plugin_cli_sessions::strategy::codex::Codex;
use plugin_cli_sessions::strategy::kimi::Kimi;
use plugin_cli_sessions::strategy::pi::Pi;
use plugin_cli_sessions::strategy::{
    running_since_for, status_for, Cli, Ctx, Phase, Prev, Strategy,
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
