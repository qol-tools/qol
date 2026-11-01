use super::parser::{parse_hotkey, parse_key_code};
use super::planning::plan_registrations;
use super::types::{HotkeyBinding, HotkeyConfig};
use crate::plugins::manifest::is_valid_action_id;
use crate::plugins::PluginUid;
use global_hotkey::hotkey::{Code, Modifiers};
use std::collections::{BTreeMap, HashMap, HashSet};

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
    for input in [
        "unknown", "", "ctrl", "shift", "f0", "f13", "key", " ", "aa",
    ] {
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
        (
            "Ctrl + Shift + R",
            Code::KeyR,
            Modifiers::CONTROL | Modifiers::SHIFT,
        ),
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
        "run",
        "toggle-feature",
        "action_name",
        "Action123",
        "a",
        "ABC",
        "a-b-c",
        "a_b_c",
        "123",
        "a1b2c3",
    ] {
        assert!(is_valid_action_id(input), "input: {:?}", input);
    }
    assert!(is_valid_action_id(&"a".repeat(64)), "max length 64");
}

#[test]
fn is_valid_action_id_rejects_invalid() {
    for input in [
        "",
        "-",
        "--help",
        "-v",
        "-flag",
        "foo bar",
        "foo\tbar",
        "foo;bar",
        "foo&bar",
        "foo|bar",
        "foo>bar",
        "foo<bar",
        "$(whoami)",
        "`whoami`",
        "foo\0bar",
        "foo\nbar",
        "foo/bar",
        "foo\\bar",
        "foo=bar",
        "foo'bar",
        "foo\"bar",
    ] {
        assert!(!is_valid_action_id(input), "input: {:?}", input);
    }
    assert!(!is_valid_action_id(&"a".repeat(65)), "max length + 1");
}

fn make_available(uid: &str, actions: &[&str]) -> super::catalog::AvailableActions {
    let mut map = HashMap::new();
    map.insert(
        PluginUid::new(uid),
        actions
            .iter()
            .map(|s| (s.to_string(), false))
            .collect::<BTreeMap<_, _>>(),
    );
    map
}

fn binding(uid: &str, action: &str, key: &str, enabled: bool) -> HotkeyBinding {
    HotkeyBinding {
        id: format!("{uid}-{action}"),
        key: key.to_string(),
        plugin_uid: PluginUid::new(uid),
        action: action.to_string(),
        enabled,
        os_only: false,
        hold_ms: None,
    }
}

#[test]
fn capture_bindings_inherit_continuous_action_metadata() {
    let uid = PluginUid::new("uid-window-actions");
    let config = HotkeyConfig {
        hotkeys: vec![
            binding(uid.as_str(), "glide-left", "Ctrl+Alt+Shift+Left", true),
            binding(uid.as_str(), "center", "Ctrl+Alt+C", true),
        ],
    };
    let continuous = HashSet::from([(uid, "glide-left".to_string())]);

    let bindings = super::build_capture_bindings(config, &continuous);

    assert!(bindings[0].continuous);
    assert!(!bindings[1].continuous);
}

#[test]
fn plan_registrations_includes_binding_when_uid_and_action_match() {
    let available = make_available("uid-foo", &["run"]);
    let config = HotkeyConfig {
        hotkeys: vec![binding("uid-foo", "run", "Ctrl+R", true)],
    };
    let plan = plan_registrations(&config, &available);
    assert_eq!(
        plan.len(),
        1,
        "matching uid+action must produce one registration"
    );
    assert_eq!(plan[0].action.plugin_uid.as_str(), "uid-foo");
    assert_eq!(plan[0].action.action, "run");
    assert!(!plan[0].action.continuous);
}

#[test]
fn plan_registrations_preserves_continuous_action_metadata() {
    let mut available = make_available("uid-window-actions", &["glide-left"]);
    *available
        .get_mut(&PluginUid::new("uid-window-actions"))
        .unwrap()
        .get_mut("glide-left")
        .unwrap() = true;
    let config = HotkeyConfig {
        hotkeys: vec![binding(
            "uid-window-actions",
            "glide-left",
            "Ctrl+Shift+Super+Left",
            true,
        )],
    };

    let plan = plan_registrations(&config, &available);

    assert_eq!(plan.len(), 1);
    assert!(plan[0].action.continuous);
}

#[test]
fn plan_registrations_skips_binding_when_uid_not_in_available_actions() {
    let available = make_available("uid-foo", &["run"]);
    let config = HotkeyConfig {
        hotkeys: vec![binding("uid-ghost", "run", "Ctrl+R", true)],
    };
    let plan = plan_registrations(&config, &available);
    assert!(
        plan.is_empty(),
        "binding with unknown uid must be skipped, not panicked; got {} registrations",
        plan.len()
    );
}

#[test]
fn plan_registrations_skips_disabled_binding() {
    let available = make_available("uid-foo", &["run"]);
    let config = HotkeyConfig {
        hotkeys: vec![binding("uid-foo", "run", "Ctrl+R", false)],
    };
    let plan = plan_registrations(&config, &available);
    assert!(plan.is_empty(), "disabled binding must not be registered");
}

#[test]
fn plan_registrations_skips_when_action_not_in_uid_entry() {
    let available = make_available("uid-foo", &["run"]);
    let config = HotkeyConfig {
        hotkeys: vec![binding("uid-foo", "nonexistent-action", "Ctrl+R", true)],
    };
    let plan = plan_registrations(&config, &available);
    assert!(
        plan.is_empty(),
        "binding with wrong action for the uid must be skipped"
    );
}

#[test]
fn hotkey_binding_deserializes_legacy_plugin_id_field_as_plugin_uid() {
    let cases = [
        (
            r#"{"id":"hk-1","key":"Ctrl+A","plugin_id":"plugin-x","action":"run","enabled":true}"#,
            "plugin-x",
            "old-schema plugin_id field",
        ),
        (
            r#"{"id":"hk-2","key":"Ctrl+B","plugin_uid":"plugin-y","action":"run","enabled":false}"#,
            "plugin-y",
            "new-schema plugin_uid field",
        ),
    ];
    for (json, expected_uid, label) in cases {
        let binding: HotkeyBinding = serde_json::from_str(json).unwrap();
        assert_eq!(binding.plugin_uid.as_str(), expected_uid, "case: {label}");
    }
}

#[test]
fn hotkey_binding_serializes_as_plugin_uid_not_plugin_id() {
    let binding = HotkeyBinding {
        id: "hk-1".to_string(),
        key: "Ctrl+A".to_string(),
        plugin_uid: PluginUid::new("plugin-x"),
        action: "run".to_string(),
        enabled: true,
        os_only: false,
        hold_ms: None,
    };
    let value = serde_json::to_value(&binding).unwrap();
    assert_eq!(
        value["plugin_uid"], "plugin-x",
        "must serialize as plugin_uid"
    );
    assert!(value.get("plugin_id").is_none(), "must not emit plugin_id");
}

fn hold_binding(uid: &str, action: &str, key: &str, hold_ms: u32) -> HotkeyBinding {
    HotkeyBinding {
        id: format!("{uid}-{action}-hold"),
        hold_ms: Some(hold_ms),
        ..binding(uid, action, key, true)
    }
}

fn shot_config(bindings: Vec<HotkeyBinding>) -> HotkeyConfig {
    HotkeyConfig { hotkeys: bindings }
}

#[test]
fn a_chord_may_hold_one_tap_and_one_hold_but_never_two_of_a_kind() {
    let tap = binding("uid-shot", "screenshot-monitor", "Shift+Super+S", true);
    let hold = hold_binding("uid-shot", "screenshot", "shift+super+s", 1000);
    let other_tap = binding("uid-shot", "record", "SHIFT+SUPER+S", true);
    let other_hold = hold_binding("uid-shot", "stop", "Shift+Super+S", 500);
    let elsewhere = hold_binding("uid-shot", "stop", "Shift+Super+R", 500);

    let cases = [
        (vec![tap.clone()], None),
        (vec![hold.clone()], None),
        (vec![tap.clone(), hold.clone()], None),
        (vec![hold.clone(), tap.clone()], None),
        (vec![tap.clone(), other_tap.clone()], Some("SHIFT+SUPER+S")),
        (
            vec![hold.clone(), other_hold.clone()],
            Some("Shift+Super+S"),
        ),
        (
            vec![tap.clone(), hold.clone(), other_tap.clone()],
            Some("SHIFT+SUPER+S"),
        ),
        (
            vec![tap.clone(), hold.clone(), other_hold.clone()],
            Some("Shift+Super+S"),
        ),
        (vec![tap.clone(), elsewhere.clone(), hold.clone()], None),
    ];
    for (bindings, expected) in cases {
        let ids: Vec<_> = bindings.iter().map(|b| b.id.clone()).collect();
        assert_eq!(
            super::duplicate_enabled_chord(&shot_config(bindings)),
            expected.map(str::to_string),
            "{ids:?}"
        );
    }
}

#[test]
fn a_disabled_tap_does_not_block_an_enabled_tap_beside_a_hold() {
    let mut off = binding("uid-shot", "record", "Shift+Super+S", false);
    off.id = "off".into();
    let config = shot_config(vec![
        off,
        binding("uid-shot", "screenshot-monitor", "Shift+Super+S", true),
        hold_binding("uid-shot", "screenshot", "Shift+Super+S", 1000),
    ]);
    assert_eq!(super::duplicate_enabled_chord(&config), None);
}

#[test]
fn capture_bindings_fold_a_tap_and_a_hold_on_one_chord_into_one_timed_binding() {
    let config = shot_config(vec![
        binding("uid-shot", "screenshot-monitor", "Shift+Super+S", true),
        binding("uid-shot", "record", "Shift+Super+R", true),
        hold_binding("uid-shot", "screenshot", "shift+super+s", 1500),
    ]);

    let bindings = super::build_capture_bindings(config, &HashSet::new());

    assert_eq!(bindings.len(), 2);
    let timed = &bindings[0];
    let plan = timed.hold.as_ref().expect("timed binding");
    assert_eq!(plan.hold_ms, 1500);
    assert_eq!(plan.hold.action, "screenshot");
    assert_eq!(
        plan.tap.as_ref().map(|tap| tap.action.as_str()),
        Some("screenshot-monitor")
    );
    assert_eq!(timed.action, "screenshot-monitor");
    assert!(!timed.continuous);
    assert!(timed.reports_release());
    assert_eq!(bindings[1].action, "record");
    assert!(bindings[1].hold.is_none());
    assert!(!bindings[1].reports_release());
}

#[test]
fn capture_bindings_keep_a_lone_hold_and_a_lone_tap_each_on_their_own() {
    let config = shot_config(vec![
        hold_binding("uid-shot", "screenshot", "Shift+Super+S", 800),
        binding("uid-shot", "record", "Shift+Super+R", true),
    ]);

    let bindings = super::build_capture_bindings(config, &HashSet::new());

    assert_eq!(bindings.len(), 2);
    let plan = bindings[0].hold.as_ref().unwrap();
    assert_eq!(plan.tap, None);
    assert_eq!(bindings[0].action, "screenshot");
    assert!(bindings[1].hold.is_none());
}

#[test]
fn capture_bindings_without_a_hold_stay_exactly_as_they_were() {
    let config = shot_config(vec![
        binding("uid-shot", "screenshot-monitor", "Shift+Super+S", true),
        binding("uid-shot", "record", "Shift+Super+R", true),
        binding("uid-shot", "disabled", "Shift+Super+D", false),
    ]);

    let bindings = super::build_capture_bindings(config, &HashSet::new());

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().all(|binding| binding.hold.is_none()));
    assert!(bindings.iter().all(|binding| !binding.reports_release()));
}

#[test]
fn capture_bindings_ignore_a_hold_time_on_a_continuous_chord() {
    let uid = PluginUid::new("uid-window-actions");
    let config = shot_config(vec![
        hold_binding(uid.as_str(), "glide-left", "Ctrl+Alt+Left", 1000),
        binding(uid.as_str(), "center", "Ctrl+Alt+Left", true),
    ]);
    let continuous = HashSet::from([(uid, "glide-left".to_string())]);

    let bindings = super::build_capture_bindings(config, &continuous);

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().all(|binding| binding.hold.is_none()));
    assert!(bindings[0].continuous);
}

#[test]
fn capture_bindings_clamp_a_hold_time_outside_the_limits() {
    for (asked, expected) in [(5, 100), (1000, 1000), (90_000, 10_000)] {
        let config = shot_config(vec![hold_binding(
            "uid-shot",
            "screenshot",
            "Shift+Super+S",
            asked,
        )]);
        let bindings = super::build_capture_bindings(config, &HashSet::new());
        assert_eq!(bindings[0].hold.as_ref().unwrap().hold_ms, expected);
    }
}

#[test]
fn plan_registrations_fold_a_tap_and_a_hold_into_one_grab() {
    let available = make_available("uid-shot", &["screenshot", "screenshot-monitor", "record"]);
    let config = shot_config(vec![
        hold_binding("uid-shot", "screenshot", "Shift+Super+S", 1000),
        binding("uid-shot", "record", "Shift+Super+R", true),
        binding("uid-shot", "screenshot-monitor", "shift+super+s", true),
    ]);

    let plan = plan_registrations(&config, &available);

    assert_eq!(plan.len(), 2);
    let timed = plan
        .iter()
        .find(|registration| registration.hold.is_some())
        .expect("timed grab");
    let hold = timed.hold.as_ref().unwrap();
    assert_eq!(hold.hold.action, "screenshot");
    assert_eq!(
        hold.tap.as_ref().map(|tap| tap.action.as_str()),
        Some("screenshot-monitor")
    );
    assert_eq!(timed.action.action, "screenshot-monitor");
    assert!(!timed.action.continuous);
    assert_eq!(timed.binding_key, "Shift+Super+S");
    let plain = plan
        .iter()
        .find(|registration| registration.hold.is_none())
        .expect("plain grab");
    assert_eq!(plain.action.action, "record");
}

#[test]
fn plan_registrations_keep_a_continuous_chord_plain_even_with_a_hold_beside_it() {
    let mut available = make_available("uid-window-actions", &["glide-left", "center"]);
    *available
        .get_mut(&PluginUid::new("uid-window-actions"))
        .unwrap()
        .get_mut("glide-left")
        .unwrap() = true;
    let config = shot_config(vec![
        binding("uid-window-actions", "glide-left", "Ctrl+Alt+Left", true),
        hold_binding("uid-window-actions", "center", "Ctrl+Alt+Left", 1000),
    ]);

    let plan = plan_registrations(&config, &available);

    assert_eq!(plan.len(), 2);
    assert!(plan.iter().all(|registration| registration.hold.is_none()));
}

#[test]
fn plan_registrations_run_a_hold_alone_when_its_tap_partner_is_unavailable() {
    let available = make_available("uid-shot", &["screenshot"]);
    let config = shot_config(vec![
        binding("uid-shot", "gone", "Shift+Super+S", true),
        hold_binding("uid-shot", "screenshot", "Shift+Super+S", 1000),
    ]);

    let plan = plan_registrations(&config, &available);

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].hold.as_ref().unwrap().tap, None);
}

#[test]
fn the_hold_time_survives_a_save_and_load_through_both_layers() {
    let dir = tempfile::tempdir().unwrap();
    let layers = super::HotkeyLayers {
        core: dir.path().join("core.json"),
        os: dir.path().join("os.json"),
    };
    let mut os_hold = hold_binding("uid-shot", "screenshot", "Shift+Super+S", 1200);
    os_hold.os_only = true;
    let config = shot_config(vec![
        binding("uid-shot", "screenshot-monitor", "Shift+Super+S", true),
        os_hold,
    ]);

    layers.save(&config).unwrap();
    let loaded = layers.load().unwrap();

    assert_eq!(loaded.hotkeys.len(), 2);
    let tap = loaded.hotkeys.iter().find(|b| b.hold_ms.is_none()).unwrap();
    let hold = loaded.hotkeys.iter().find(|b| b.hold_ms.is_some()).unwrap();
    assert_eq!(tap.action, "screenshot-monitor");
    assert_eq!(hold.hold_ms, Some(1200));
    assert!(hold.os_only);
    let core: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&layers.core).unwrap()).unwrap();
    assert!(core["hotkeys"][0].get("hold_ms").is_none());
    let os: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&layers.os).unwrap()).unwrap();
    assert_eq!(os["hotkeys"][0]["hold_ms"], serde_json::json!(1200));
}

#[test]
fn hotkey_binding_keeps_hold_ms_through_a_json_round_trip() {
    let cases = [(None, false), (Some(1000), true), (Some(250), true)];
    for (hold_ms, written) in cases {
        let mut original = binding("uid-shot", "screenshot", "Shift+Super+S", true);
        original.hold_ms = hold_ms;
        let value = serde_json::to_value(&original).unwrap();
        assert_eq!(value.get("hold_ms").is_some(), written, "{hold_ms:?}");
        let back: HotkeyBinding = serde_json::from_value(value).unwrap();
        assert_eq!(back.hold_ms, hold_ms);
    }
}
