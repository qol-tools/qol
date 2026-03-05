use super::parser::{parse_hotkey, parse_key_code};
use global_hotkey::hotkey::{Code, Modifiers};

#[test]
fn parse_key_code_cases() {
    let valid = [
        ("a", Code::KeyA),
        ("A", Code::KeyA),
        ("z", Code::KeyZ),
        ("Z", Code::KeyZ),
        ("0", Code::Digit0),
        ("5", Code::Digit5),
        ("9", Code::Digit9),
        ("f1", Code::F1),
        ("F1", Code::F1),
        ("f12", Code::F12),
        ("F12", Code::F12),
        ("space", Code::Space),
        ("SPACE", Code::Space),
        ("return", Code::Enter),
        ("enter", Code::Enter),
        ("esc", Code::Escape),
        ("escape", Code::Escape),
        ("tab", Code::Tab),
        ("backspace", Code::Backspace),
        ("delete", Code::Delete),
        ("insert", Code::Insert),
        ("up", Code::ArrowUp),
        ("down", Code::ArrowDown),
        ("left", Code::ArrowLeft),
        ("right", Code::ArrowRight),
        ("home", Code::Home),
        ("end", Code::End),
        ("pageup", Code::PageUp),
        ("pgup", Code::PageUp),
        ("pagedown", Code::PageDown),
        ("pgdn", Code::PageDown),
    ];

    for (input, expected) in valid {
        assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
    }

    let invalid = [
        "unknown", "", "ctrl", "shift", "f0", "f13", "key", " ", "aa",
    ];
    for input in invalid {
        assert_eq!(parse_key_code(input), None, "input: {:?}", input);
    }
}

#[test]
fn parse_hotkey_valid_cases() {
    let cases: &[(&str, Code, Modifiers)] = &[
        ("R", Code::KeyR, Modifiers::empty()),
        ("r", Code::KeyR, Modifiers::empty()),
        ("F1", Code::F1, Modifiers::empty()),
        ("Space", Code::Space, Modifiers::empty()),
        ("Ctrl+R", Code::KeyR, Modifiers::CONTROL),
        ("ctrl+r", Code::KeyR, Modifiers::CONTROL),
        ("CTRL+R", Code::KeyR, Modifiers::CONTROL),
        ("Control+R", Code::KeyR, Modifiers::CONTROL),
        ("Alt+R", Code::KeyR, Modifiers::ALT),
        ("Shift+R", Code::KeyR, Modifiers::SHIFT),
        ("Super+R", Code::KeyR, Modifiers::SUPER),
        ("Win+R", Code::KeyR, Modifiers::SUPER),
        ("Meta+R", Code::KeyR, Modifiers::SUPER),
        ("Cmd+R", Code::KeyR, Modifiers::SUPER),
        (
            "Ctrl+Shift+R",
            Code::KeyR,
            Modifiers::CONTROL | Modifiers::SHIFT,
        ),
        (
            "Ctrl+Alt+R",
            Code::KeyR,
            Modifiers::CONTROL | Modifiers::ALT,
        ),
        (
            "Ctrl+Shift+Alt+R",
            Code::KeyR,
            Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT,
        ),
        (
            "Ctrl+Shift+Alt+Super+R",
            Code::KeyR,
            Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER,
        ),
        ("  Ctrl  +  R  ", Code::KeyR, Modifiers::CONTROL),
        (
            "Ctrl + Shift + R",
            Code::KeyR,
            Modifiers::CONTROL | Modifiers::SHIFT,
        ),
        ("+R", Code::KeyR, Modifiers::empty()),
        ("Ctrl++R", Code::KeyR, Modifiers::CONTROL),
        ("Ctrl+F12", Code::F12, Modifiers::CONTROL),
        ("Alt+Tab", Code::Tab, Modifiers::ALT),
    ];

    for (input, expected_key, expected_mods) in cases {
        let result = parse_hotkey(input);
        assert!(result.is_some(), "input: {:?} should parse", input);
        let hotkey = result.unwrap();
        assert_eq!(hotkey.key, *expected_key, "input: {:?} key mismatch", input);
        assert_eq!(
            hotkey.mods, *expected_mods,
            "input: {:?} mods mismatch",
            input
        );
    }
}

#[test]
fn parse_hotkey_invalid_cases() {
    let cases = [
        "",
        "   ",
        "+++",
        "Ctrl",
        "Ctrl+",
        "Ctrl+Shift",
        "Ctrl+Shift+",
        "+",
        "++",
        "Ctrl+InvalidKey",
        "Ctrl+Shift+Unknown",
        "NotAKey",
        "Ctrl+Alt+",
        "\t",
        "\n",
    ];

    for input in cases {
        assert!(
            parse_hotkey(input).is_none(),
            "input: {:?} should not parse",
            input
        );
    }
}

#[test]
fn is_valid_action_id_cases() {
    let cases = [
        ("run", true),
        ("toggle-feature", true),
        ("action_name", true),
        ("Action123", true),
        ("a", true),
        ("ABC", true),
        ("a-b-c", true),
        ("a_b_c", true),
        ("123", true),
        ("a1b2c3", true),
        (&"a".repeat(64), true),
        ("", false),
        ("-", false),
        ("--help", false),
        ("-v", false),
        ("-flag", false),
        ("foo bar", false),
        ("foo\tbar", false),
        ("foo;bar", false),
        ("foo&bar", false),
        ("foo|bar", false),
        ("foo>bar", false),
        ("foo<bar", false),
        ("$(whoami)", false),
        ("`whoami`", false),
        ("foo\0bar", false),
        ("foo\nbar", false),
        ("foo/bar", false),
        ("foo\\bar", false),
        (&"a".repeat(65), false),
        ("foo=bar", false),
        ("foo'bar", false),
        ("foo\"bar", false),
    ];

    for (input, expected) in cases {
        assert_eq!(
            crate::plugins::manifest::is_valid_action_id(input),
            expected,
            "input: {:?}",
            input
        );
    }
}
