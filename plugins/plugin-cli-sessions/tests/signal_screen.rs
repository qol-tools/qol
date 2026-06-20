use plugin_cli_sessions::signal::screen::{
    claude_working, has_input_request, has_prompt_markers, screen_hash,
};

#[test]
fn claude_working_detects_live_counter_only() {
    let cases = [
        (
            "\u{273B} Processing\u{2026} (2m 30s \u{00B7} \u{2193} 10.2k tokens)",
            true,
        ),
        (
            "\u{2736} Proposing approaches\u{2026} (1m 55s \u{00B7} \u{2193} 5.8k tokens)",
            true,
        ),
        ("  Thinking (esc to interrupt)", true),
        ("\u{273B} Cogitated for 2m 9s", false),
        ("\u{2733} Brewed for 2m 32s", false),
        ("  ~/repos/project \u{00B7} Opus 4.8 (1M con\u{2026}", false),
        ("\u{276F} ", false),
        ("new task? /clear to save 324.2k tokens", false),
    ];
    for (t, expected) in cases {
        assert_eq!(claude_working(t), expected, "claude_working: {t:?}");
    }
}

#[test]
fn screen_hash_changes_with_content() {
    assert_eq!(screen_hash("same"), screen_hash("same"));
    assert_ne!(
        screen_hash("a"),
        screen_hash("b"),
        "different text hashes differ"
    );
}

#[test]
fn prompt_markers_detect_structured_choices() {
    let positives = [
        "  \u{276F} 1. Coffee\n    2. Tea\n  Enter to select \u{00B7} arrows to navigate",
        "  \u{203A} 1. Yes\n    2. No\n  tab to add notes | enter to submit answer",
        "Overwrite file? [y/N]",
        "  [x] run-state\n  Press space to toggle; enter to confirm",
        "  1) one\n  2) two\n  3) three",
    ];
    for t in positives {
        assert!(has_prompt_markers(t), "should be a choice: {t:?}");
    }
    let negatives = [
        "\u{2733} Brewed for 2m 32s",
        "Compiling plugin-cli-sessions v0.1.0",
        "~/repos/project main",
        "Downloading: 45%",
        "\u{276F} ",
        "\u{276F} Seems like it is working",
        "\u{276F} \n  \u{23F5}\u{23F5} bypass permissions on (shift+tab to cycle) \u{00B7} \u{2190} for agents",
        "Enter your name: ",
        "Continue?",
        "    1.9G    /home/user/.cache/github-copilot",
        "   53G    /home/user/repos/project/target",
        "v2.3.1 released",
        "",
    ];
    for t in negatives {
        assert!(!has_prompt_markers(t), "caret/prose is not a choice: {t:?}");
    }
}

#[test]
fn claude_done_detects_completion_marker_only() {
    use plugin_cli_sessions::signal::screen::claude_done;
    let cases = [
        ("\u{273B} Brewed for 2m 32s", true),
        ("\u{273B} Cogitated for 2m 9s", true),
        ("\u{2736} Distilled for 45s", true),
        (
            "\u{273B} Processing\u{2026} (5s \u{00B7} \u{2193} 1k tokens)",
            false,
        ),
        ("Welcome to Claude Code", false),
        ("waiting for the build", false),
        ("\u{276F} ", false),
    ];
    for (t, expected) in cases {
        assert_eq!(claude_done(t), expected, "claude_done: {t:?}");
    }
}

#[test]
fn codex_signals_distinguish_working_and_fresh() {
    use plugin_cli_sessions::signal::screen::{codex_banner, codex_working};
    assert!(codex_working("Working (1s \u{00B7} esc to interrupt)"));
    assert!(!codex_working("\u{00B7} Ready \u{00B7} Full Access"));
    assert!(codex_banner("\u{203A} >_ OpenAI Codex (v0.141.0)"));
    assert!(codex_banner("Tip: Try the Codex App"));
    assert!(!codex_banner("some normal conversation output"));
}

#[test]
fn input_request_detects_trailing_colon_or_question() {
    let cases = [
        ("Enter your name: ", true),
        ("Password:", true),
        ("Continue?", true),
        ("\u{276F} Seems like it is working", false),
        ("Compiling project", false),
        ("\u{2733} Brewed for 2m", false),
    ];
    for (t, expected) in cases {
        assert_eq!(has_input_request(t), expected, "input_request: {t:?}");
    }
}
