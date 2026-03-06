use super::parser::{parse_hotkey, parse_key_code};
use crate::plugins::manifest::is_valid_action_id;
use global_hotkey::hotkey::{Code, Modifiers};

#[test]
fn parse_key_code_letters() {
    for (input, expected) in [
        ("a", Code::KeyA),
        ("A", Code::KeyA),
        ("z", Code::KeyZ),
        ("Z", Code::KeyZ),
    ] {
        assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
    }
}

#[test]
fn parse_key_code_digits() {
    for (input, expected) in [
        ("0", Code::Digit0),
        ("5", Code::Digit5),
        ("9", Code::Digit9),
    ] {
        assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
    }
}

#[test]
fn parse_key_code_function_keys() {
    for (input, expected) in [
        ("f1", Code::F1),
        ("F1", Code::F1),
        ("f12", Code::F12),
        ("F12", Code::F12),
    ] {
        assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
    }
}

#[test]
fn parse_key_code_named_keys() {
    for (input, expected) in [
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
    ] {
        assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
    }
}

#[test]
fn parse_key_code_navigation() {
    for (input, expected) in [
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
    ] {
        assert_eq!(parse_key_code(input), Some(expected), "input: {}", input);
    }
}

#[test]
fn parse_key_code_invalid() {
    for input in ["unknown", "", "ctrl", "shift", "f0", "f13", "key", " ", "aa"] {
        assert_eq!(parse_key_code(input), None, "input: {:?}", input);
    }
}

fn assert_parses(input: &str, key: Code, mods: Modifiers) {
    let result = parse_hotkey(input);
    assert!(result.is_some(), "input: {:?} should parse", input);
    let hotkey = result.unwrap();
    assert_eq!(hotkey.key, key, "input: {:?} key mismatch", input);
    assert_eq!(hotkey.mods, mods, "input: {:?} mods mismatch", input);
}

#[test]
fn parse_hotkey_no_modifier() {
    for (input, key, mods) in [
        ("R", Code::KeyR, Modifiers::empty()),
        ("r", Code::KeyR, Modifiers::empty()),
        ("F1", Code::F1, Modifiers::empty()),
        ("Space", Code::Space, Modifiers::empty()),
    ] {
        assert_parses(input, key, mods);
    }
}

#[test]
fn parse_hotkey_single_modifier() {
    for (input, key, mods) in [
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
        ("Ctrl+F12", Code::F12, Modifiers::CONTROL),
        ("Alt+Tab", Code::Tab, Modifiers::ALT),
    ] {
        assert_parses(input, key, mods);
    }
}

#[test]
fn parse_hotkey_multi_modifier() {
    for (input, key, mods) in [
        ("Ctrl+Shift+R", Code::KeyR, Modifiers::CONTROL | Modifiers::SHIFT),
        ("Ctrl+Alt+R", Code::KeyR, Modifiers::CONTROL | Modifiers::ALT),
        ("Ctrl+Shift+Alt+R", Code::KeyR, Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT),
        ("Ctrl+Shift+Alt+Super+R", Code::KeyR, Modifiers::CONTROL | Modifiers::SHIFT | Modifiers::ALT | Modifiers::SUPER),
        ("Ctrl + Shift + R", Code::KeyR, Modifiers::CONTROL | Modifiers::SHIFT),
    ] {
        assert_parses(input, key, mods);
    }
}

#[test]
fn parse_hotkey_syntax_tolerance() {
    for (input, key, mods) in [
        ("  Ctrl  +  R  ", Code::KeyR, Modifiers::CONTROL),
        ("+R", Code::KeyR, Modifiers::empty()),
        ("Ctrl++R", Code::KeyR, Modifiers::CONTROL),
    ] {
        assert_parses(input, key, mods);
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
fn is_valid_action_id_accepts_valid() {
    for input in [
        "run", "toggle-feature", "action_name", "Action123",
        "a", "ABC", "a-b-c", "a_b_c", "123", "a1b2c3",
    ] {
        assert!(is_valid_action_id(input), "input: {:?}", input);
    }
    assert!(is_valid_action_id(&"a".repeat(64)), "max length 64");
}

#[test]
fn is_valid_action_id_rejects_invalid() {
    for input in [
        "", "-", "--help", "-v", "-flag",
        "foo bar", "foo\tbar", "foo;bar", "foo&bar", "foo|bar",
        "foo>bar", "foo<bar", "$(whoami)", "`whoami`",
        "foo\0bar", "foo\nbar", "foo/bar", "foo\\bar",
        "foo=bar", "foo'bar", "foo\"bar",
    ] {
        assert!(!is_valid_action_id(input), "input: {:?}", input);
    }
    assert!(!is_valid_action_id(&"a".repeat(65)), "max length + 1");
}
