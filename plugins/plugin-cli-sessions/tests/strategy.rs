use plugin_cli_sessions::host::Pane;
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::claude::Claude;
use plugin_cli_sessions::strategy::codex::{Codex, CodexSession, CodexStore, NoCodexStore};
use plugin_cli_sessions::strategy::{
    running_since_for, status_for, Cli, Ctx, Phase, Prev, Strategy,
};

fn pane(at_prompt: bool, cmd: &str, title: &str) -> Pane {
    Pane {
        window_id: 1,
        root_pid: 1,
        cwd: "/a/proj".into(),
        title: title.into(),
        at_prompt,
        reported_cmd: Some(cmd.into()),
        foreground_basenames: vec![],
        foreground_pids: vec![],
    }
}

fn ctx<'a>(
    p: &'a Pane,
    screen: Option<&'a str>,
    changed: bool,
    prev: Option<Prev>,
    now: u64,
) -> Ctx<'a> {
    Ctx {
        pane: p,
        screen,
        screen_changed: changed,
        prev,
        now,
    }
}

fn ran_since(start: u64) -> Option<Prev> {
    Some(Prev {
        status: Status::Working,
        running_since: Some(start),
    })
}

struct FakeCodex {
    name: Option<String>,
    touched: bool,
}

impl CodexStore for FakeCodex {
    fn session(&self, _pane: &Pane) -> Option<CodexSession> {
        Some(CodexSession {
            name: self.name.clone(),
            touched: self.touched,
        })
    }
}

#[test]
fn status_for_maps_every_phase_with_ack_carry() {
    let cases = [
        (Status::Unknown, Phase::Busy, Status::Working),
        (Status::Unknown, Phase::Blocked, Status::NeedsYou),
        (Status::Working, Phase::Done, Status::YourTurn),
        (Status::Unknown, Phase::Idle, Status::Unknown),
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
    let p = pane(false, "claudedw", "\u{2733} Improve logging");
    let r = Claude.read(&ctx(&p, Some(""), false, None, 0));
    assert_eq!(r.label.as_deref(), Some("Improve logging"));
}

#[test]
fn codex_phase_and_label_from_store() {
    let p = pane(false, "codex", "qol-monorepo");

    let fresh = FakeCodex {
        name: Some("Asasdsadasd".into()),
        touched: false,
    };
    let r = Codex::new(&fresh).read(&ctx(&p, Some(""), false, None, 0));
    assert_eq!(r.phase, Phase::Idle);
    assert_eq!(r.label.as_deref(), Some("Asasdsadasd"));

    let used = FakeCodex {
        name: Some("Asasdsadasd".into()),
        touched: true,
    };
    assert_eq!(
        Codex::new(&used)
            .read(&ctx(&p, Some(""), false, None, 0))
            .phase,
        Phase::Done
    );

    let busy = FakeCodex {
        name: None,
        touched: true,
    };
    let busy_with_sizes = "  1.9G  /home/user/.cache\n\u{2022} Working (3m 56s \u{00B7} esc to interrupt)\n\u{203A} Use /skills";
    let r = Codex::new(&busy).read(&ctx(&p, Some(busy_with_sizes), false, None, 0));
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
    let store = FakeCodex {
        name: Some("Fix edge case".into()),
        touched: true,
    };
    let scrolled = "previous answer\nDone for 12s\n\u{203A} Use /skills";
    let braille = pane(false, "codex", "\u{2810} Fix edge case");
    assert_eq!(
        Codex::new(&store)
            .read(&ctx(&braille, Some(scrolled), false, None, 0))
            .phase,
        Phase::Busy,
        "a live Codex working title beats scrolled-up non-working content"
    );

    let marker_like = "old question\n  [ ] Review diff\n  enter to confirm";
    let star = pane(false, "codex", "\u{2736} Fix edge case");
    assert_eq!(
        Codex::new(&store)
            .read(&ctx(&star, Some(marker_like), false, None, 0))
            .phase,
        Phase::Busy,
        "a live Codex working title beats scrolled-up prompt-like content"
    );
}

#[test]
fn codex_idle_via_banner_when_store_absent() {
    let p = pane(false, "codex", "qol-monorepo");
    let fresh = Codex::new(&NoCodexStore).read(&ctx(
        &p,
        Some("\u{203A} >_ OpenAI Codex (v0.141.0)"),
        false,
        None,
        0,
    ));
    assert_eq!(fresh.phase, Phase::Idle, "welcome banner => fresh => idle");
    let used =
        Codex::new(&NoCodexStore).read(&ctx(&p, Some("conversation output"), false, None, 0));
    assert_eq!(used.phase, Phase::Done, "no banner => a turn happened");
}
