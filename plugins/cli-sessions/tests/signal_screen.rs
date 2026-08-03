use plugin_cli_sessions::signal::screen::{
    claude_working, has_input_request, has_numbered_choice_prompt, has_prompt_markers, screen_hash,
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
fn numbered_choice_prompt_needs_numbers_and_an_affordance() {
    let picker = "How should the uninstaller be invoked?\n\n1. gpui popup picker\n2. Keep terminal CLI\n3. Both: popup + CLI\n\nEnter to select \u{00B7} \u{2191}/\u{2193} to navigate \u{00B7} n to add notes \u{00B7} Esc to cancel";
    let search_browser = "Discover plugins (1/221)\n\u{276F} o code-review\n  o github\n  Type to search \u{00B7} Space to toggle \u{00B7} Enter to view";
    let cases = [
        (picker, true),
        ("  \u{276F} 1. Yes\n    2. No\n  enter to confirm", true),
        // a search/discovery browser has affordances but no numbered options
        (search_browser, false),
        // a prose numbered list has numbers but no selection affordance
        ("Here are the steps:\n1. clone\n2. build\n3. run", false),
        ("Overwrite file? [y/N]", false),
        ("\u{2733} Brewed for 2m 32s", false),
        ("Welcome to Claude Code\n\u{276F} ", false),
        ("", false),
    ];
    for (text, expected) in cases {
        assert_eq!(
            has_numbered_choice_prompt(text),
            expected,
            "numbered_choice_prompt: {text:?}"
        );
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
fn kimi_working_detects_moon_dot_bare_moon_and_braille_shapes() {
    use plugin_cli_sessions::signal::screen::kimi_working;
    let boxed = |spinner: &str| {
        format!(
            "{spinner}\n\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]"
        )
    };
    let bare_moon = boxed("  \u{1F317}");
    let bare_moon_fe0f = boxed("\u{1F312}\u{FE0F}");
    let braille = boxed("\u{2819} thinking...");
    let braille_nested = boxed("ledger output\n\u{2826} Working... (esc to interrupt)");
    let moon_in_answer = boxed("Here is the status:\n\u{1F315}\nThat's the full report.");
    let cases = [
        (
            "\u{1F315} \u{00B7} Tip: /tasks to check progress and status for background tasks",
            true,
        ),
        (
            "\u{1F315}\u{FE0F} \u{00B7} Tip: /sessions to browse and resume earlier sessions",
            true,
        ),
        (bare_moon.as_str(), true),
        (bare_moon_fe0f.as_str(), true),
        (braille.as_str(), true),
        (braille_nested.as_str(), true),
        ("\u{1F315} release status is green", false),
        (
            "Assistant answer:\n\u{1F315} release status is green\n> ",
            false,
        ),
        (moon_in_answer.as_str(), false),
        (
            "yolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]",
            false,
        ),
        ("", false),
    ];
    for (t, expected) in cases {
        assert_eq!(kimi_working(t), expected, "kimi_working: {t:?}");
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
