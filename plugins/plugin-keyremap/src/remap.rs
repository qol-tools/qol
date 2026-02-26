use std::collections::HashSet;

use crate::config::{KeyRule, MouseRule, RemapConfig, ScrollRule};
use crate::keycode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        cmd: false,
    };

    fn matches(&self, other: &Self) -> bool {
        self.ctrl == other.ctrl
            && self.shift == other.shift
            && self.alt == other.alt
            && self.cmd == other.cmd
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

pub struct ResolvedConfig {
    pub excluded_apps: HashSet<String>,
    pub key_rules: Vec<ResolvedKeyRule>,
    pub mouse_rules: Vec<ResolvedMouseRule>,
    pub scroll_rules: Vec<ResolvedScrollRule>,
}

pub struct ResolvedKeyRule {
    pub from_mods: Modifiers,
    pub from_key: u16,
    pub to_mods: Modifiers,
    pub to_key: u16,
}

pub struct ResolvedMouseRule {
    pub from_mods: Modifiers,
    pub button: MouseButton,
    pub to_mods: Modifiers,
}

pub struct ResolvedScrollRule {
    pub from_mods: Modifiers,
    pub to_mods: Modifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Passthrough,
    Remap { mods: Modifiers, key: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Passthrough,
    Remap { mods: Modifiers },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAction {
    Passthrough,
    Remap { mods: Modifiers },
}

pub fn resolve(config: &RemapConfig) -> ResolvedConfig {
    ResolvedConfig {
        excluded_apps: config.excluded_apps.iter().cloned().collect(),
        key_rules: config.key_rules.iter().filter_map(resolve_key_rule).collect(),
        mouse_rules: config.mouse_rules.iter().filter_map(resolve_mouse_rule).collect(),
        scroll_rules: config.scroll_rules.iter().filter_map(resolve_scroll_rule).collect(),
    }
}

pub fn process_key_event(
    config: &ResolvedConfig,
    mods: Modifiers,
    key: u16,
    bundle_id: &str,
) -> KeyAction {
    if config.excluded_apps.contains(bundle_id) {
        return KeyAction::Passthrough;
    }
    for rule in &config.key_rules {
        if rule.from_mods.matches(&mods) && rule.from_key == key {
            return KeyAction::Remap {
                mods: rule.to_mods,
                key: rule.to_key,
            };
        }
    }
    KeyAction::Passthrough
}

pub fn process_mouse_event(
    config: &ResolvedConfig,
    mods: Modifiers,
    button: MouseButton,
    bundle_id: &str,
) -> MouseAction {
    if config.excluded_apps.contains(bundle_id) {
        return MouseAction::Passthrough;
    }
    for rule in &config.mouse_rules {
        if rule.from_mods.matches(&mods) && rule.button == button {
            return MouseAction::Remap { mods: rule.to_mods };
        }
    }
    MouseAction::Passthrough
}

pub fn process_scroll_event(
    config: &ResolvedConfig,
    mods: Modifiers,
    bundle_id: &str,
) -> ScrollAction {
    if config.excluded_apps.contains(bundle_id) {
        return ScrollAction::Passthrough;
    }
    for rule in &config.scroll_rules {
        if rule.from_mods.matches(&mods) {
            return ScrollAction::Remap { mods: rule.to_mods };
        }
    }
    ScrollAction::Passthrough
}

fn parse_modifiers(mods: &[String]) -> Modifiers {
    let mut result = Modifiers::NONE;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => result.ctrl = true,
            "shift" => result.shift = true,
            "alt" | "option" | "opt" => result.alt = true,
            "cmd" | "command" | "super" => result.cmd = true,
            other => eprintln!("[keyremap] unknown modifier: {other}"),
        }
    }
    result
}

fn resolve_key_rule(rule: &KeyRule) -> Option<ResolvedKeyRule> {
    let from_key = keycode::parse_key(&rule.from_key)?;
    let to_key = keycode::parse_key(&rule.to_key)?;
    Some(ResolvedKeyRule {
        from_mods: parse_modifiers(&rule.from_mods),
        from_key,
        to_mods: parse_modifiers(&rule.to_mods),
        to_key,
    })
}

fn resolve_mouse_rule(rule: &MouseRule) -> Option<ResolvedMouseRule> {
    let button = match rule.button.to_ascii_lowercase().as_str() {
        "left" => MouseButton::Left,
        "right" => MouseButton::Right,
        other => {
            eprintln!("[keyremap] unknown mouse button: {other}");
            return None;
        }
    };
    Some(ResolvedMouseRule {
        from_mods: parse_modifiers(&rule.from_mods),
        button,
        to_mods: parse_modifiers(&rule.to_mods),
    })
}

fn resolve_scroll_rule(rule: &ScrollRule) -> Option<ResolvedScrollRule> {
    Some(ResolvedScrollRule {
        from_mods: parse_modifiers(&rule.from_mods),
        to_mods: parse_modifiers(&rule.to_mods),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn test_config() -> ResolvedConfig {
        let raw: RemapConfig =
            toml::from_str(include_str!("../config/default.toml")).unwrap();
        resolve(&raw)
    }

    fn arb_modifiers() -> impl Strategy<Value = Modifiers> {
        (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
            |(ctrl, shift, alt, cmd)| Modifiers {
                ctrl,
                shift,
                alt,
                cmd,
            },
        )
    }

    fn arb_keycode() -> impl Strategy<Value = u16> {
        (0u16..128)
    }

    fn arb_bundle_id() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("com.apple.Safari".to_string()),
            Just("com.microsoft.VSCode".to_string()),
            Just("com.jetbrains.rustrover".to_string()),
            Just("org.mozilla.firefox".to_string()),
            "[a-z]{3,10}\\.[a-z]{3,10}\\.[a-z]{3,10}",
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_excluded_apps_always_passthrough(
            mods in arb_modifiers(),
            key in arb_keycode(),
            app in prop_oneof![
                Just("com.microsoft.VSCode".to_string()),
                Just("com.jetbrains.rustrover".to_string()),
                Just("net.kovidgoyal.kitty".to_string()),
            ]
        ) {
            let config = test_config();
            prop_assert_eq!(
                process_key_event(&config, mods, key, &app),
                KeyAction::Passthrough
            );
            prop_assert_eq!(
                process_mouse_event(&config, mods, MouseButton::Left, &app),
                MouseAction::Passthrough
            );
            prop_assert_eq!(
                process_scroll_event(&config, mods, &app),
                ScrollAction::Passthrough
            );
        }

        #[test]
        fn prop_unmatched_events_passthrough(
            mods in arb_modifiers(),
            key in arb_keycode(),
            app in arb_bundle_id(),
        ) {
            let config = resolve(&RemapConfig::default());
            prop_assert_eq!(
                process_key_event(&config, mods, key, &app),
                KeyAction::Passthrough
            );
        }

        #[test]
        fn prop_ctrl_c_remaps_to_cmd_c_for_non_excluded(
            app in arb_bundle_id().prop_filter("not excluded", |a| {
                !["com.microsoft.VSCode", "net.kovidgoyal.kitty",
                  "com.jetbrains.rustrover", "com.jetbrains.intellij",
                  "com.jetbrains.WebStorm", "com.jetbrains.goland",
                  "com.jetbrains.pycharm", "com.jetbrains.rider",
                  "com.jetbrains.datagrip", "com.jetbrains.clion",
                  "com.apple.Terminal", "com.googlecode.iterm2",
                  "com.todesktop.230313mzl4w4u92",
                ].contains(&a.as_str())
            })
        ) {
            let config = test_config();
            let ctrl_only = Modifiers { ctrl: true, shift: false, alt: false, cmd: false };
            let result = process_key_event(&config, ctrl_only, crate::keycode::ANSI_C, &app);
            prop_assert_eq!(result, KeyAction::Remap {
                mods: Modifiers { ctrl: false, shift: false, alt: false, cmd: true },
                key: crate::keycode::ANSI_C,
            });
        }

        #[test]
        fn prop_first_match_wins(
            app in arb_bundle_id().prop_filter("not excluded", |a| {
                a != "com.microsoft.VSCode" && a != "net.kovidgoyal.kitty"
            })
        ) {
            let raw = RemapConfig {
                excluded_apps: vec![],
                key_rules: vec![
                    crate::config::KeyRule {
                        from_mods: vec!["ctrl".into()],
                        from_key: "c".into(),
                        to_mods: vec!["cmd".into()],
                        to_key: "c".into(),
                    },
                    crate::config::KeyRule {
                        from_mods: vec!["ctrl".into()],
                        from_key: "c".into(),
                        to_mods: vec!["alt".into()],
                        to_key: "x".into(),
                    },
                ],
                mouse_rules: vec![],
                scroll_rules: vec![],
            };
            let config = resolve(&raw);
            let ctrl_only = Modifiers { ctrl: true, shift: false, alt: false, cmd: false };
            let result = process_key_event(&config, ctrl_only, crate::keycode::ANSI_C, &app);
            prop_assert_eq!(result, KeyAction::Remap {
                mods: Modifiers { ctrl: false, shift: false, alt: false, cmd: true },
                key: crate::keycode::ANSI_C,
            });
        }
    }
}
