use std::collections::HashSet;

use crate::config::{CharRule, KeyRule, MouseRule, RemapConfig, ScrollRule};
use crate::keycode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
    pub ralt: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        cmd: false,
        ralt: false,
    };

    fn matches(&self, other: &Self) -> bool {
        self.ctrl == other.ctrl
            && self.shift == other.shift
            && self.alt == other.alt
            && self.cmd == other.cmd
            && (!self.ralt || other.ralt)
    }

    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl { parts.push("ctrl"); }
        if self.shift { parts.push("shift"); }
        if self.alt { parts.push("alt"); }
        if self.cmd { parts.push("cmd"); }
        if self.ralt { parts.push("ralt"); }
        if parts.is_empty() { return "(none)".into(); }
        parts.join("+")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

pub struct ResolvedConfig {
    pub enabled: bool,
    pub excluded_apps: HashSet<String>,
    pub char_rules: Vec<ResolvedCharRule>,
    pub key_rules: Vec<ResolvedKeyRule>,
    pub mouse_rules: Vec<ResolvedMouseRule>,
    pub scroll_rules: Vec<ResolvedScrollRule>,
}

pub struct ResolvedCharRule {
    pub from_mods: Modifiers,
    pub from_key: u16,
    pub to_char: String,
    pub global: bool,
}

#[derive(Clone)]
pub struct ResolvedKeyRule {
    pub from_mods: Modifiers,
    pub from_key: u16,
    pub to_mods: Modifiers,
    pub to_key: u16,
    pub global: bool,
}

pub struct ResolvedMouseRule {
    pub from_mods: Modifiers,
    pub button: MouseButton,
    pub to_mods: Modifiers,
    pub global: bool,
}

pub struct ResolvedScrollRule {
    pub from_mods: Modifiers,
    pub to_mods: Modifiers,
    pub global: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    Passthrough,
    Remap { mods: Modifiers, key: u16 },
    Char { text: String },
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
    let key_rules: Vec<ResolvedKeyRule> = config.key_rules.iter().flat_map(resolve_key_rule).collect();
    warn_shadowed_rules(&key_rules);
    ResolvedConfig {
        enabled: config.enabled,
        excluded_apps: config.excluded_apps.iter().cloned().collect(),
        char_rules: config.char_rules.iter().filter_map(resolve_char_rule).collect(),
        key_rules,
        mouse_rules: config.mouse_rules.iter().filter_map(resolve_mouse_rule).collect(),
        scroll_rules: config.scroll_rules.iter().filter_map(resolve_scroll_rule).collect(),
    }
}

fn rule_label(mods: &Modifiers, key: u16) -> String {
    format!("{}+{}", mods.label(), keycode::key_name(key))
}

fn warn_shadowed_rules(rules: &[ResolvedKeyRule]) {
    let mut seen: Vec<(Modifiers, u16)> = Vec::new();
    for rule in rules {
        let pair = (rule.from_mods, rule.from_key);
        if seen.contains(&pair) {
            eprintln!(
                "[keyremap] warning: shadowed rule — {} appears multiple times, only first match fires",
                rule_label(&rule.from_mods, rule.from_key),
            );
        } else {
            seen.push(pair);
        }
    }
}

pub fn diff_key_rules(old: &[ResolvedKeyRule], new: &[ResolvedKeyRule]) -> Vec<String> {
    let mut warnings = Vec::new();
    for old_rule in old {
        let new_match = new.iter().find(|r| r.from_mods == old_rule.from_mods && r.from_key == old_rule.from_key);
        match new_match {
            Some(new_rule) if new_rule.to_mods != old_rule.to_mods || new_rule.to_key != old_rule.to_key => {
                warnings.push(format!(
                    "rule changed — {}: was {}+{}, now {}+{}",
                    rule_label(&old_rule.from_mods, old_rule.from_key),
                    old_rule.to_mods.label(), keycode::key_name(old_rule.to_key),
                    new_rule.to_mods.label(), keycode::key_name(new_rule.to_key),
                ));
            }
            None => {
                warnings.push(format!(
                    "rule removed — {} (was {}+{})",
                    rule_label(&old_rule.from_mods, old_rule.from_key),
                    old_rule.to_mods.label(), keycode::key_name(old_rule.to_key),
                ));
            }
            _ => {}
        }
    }
    warnings
}

pub fn process_key_event(
    config: &ResolvedConfig,
    mods: Modifiers,
    key: u16,
    bundle_id: &str,
) -> KeyAction {
    // Phase 1: global char_rules (bypass excluded apps)
    for rule in &config.char_rules {
        if rule.global && rule.from_mods.matches(&mods) && rule.from_key == key {
            return KeyAction::Char { text: rule.to_char.clone() };
        }
    }

    // Phase 2: global key_rules (bypass excluded apps)
    for rule in &config.key_rules {
        if rule.global && rule.from_mods.matches(&mods) && rule.from_key == key {
            return KeyAction::Remap {
                mods: rule.to_mods,
                key: rule.to_key,
            };
        }
    }

    // Phase 3: excluded apps check
    if config.excluded_apps.contains(bundle_id) {
        return KeyAction::Passthrough;
    }

    // Phase 4: non-global char_rules
    for rule in &config.char_rules {
        if !rule.global && rule.from_mods.matches(&mods) && rule.from_key == key {
            return KeyAction::Char { text: rule.to_char.clone() };
        }
    }

    // Phase 5: non-global key_rules
    for rule in &config.key_rules {
        if !rule.global && rule.from_mods.matches(&mods) && rule.from_key == key {
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
    for rule in &config.mouse_rules {
        if rule.global && rule.from_mods.matches(&mods) && rule.button == button {
            return MouseAction::Remap { mods: rule.to_mods };
        }
    }
    if config.excluded_apps.contains(bundle_id) {
        return MouseAction::Passthrough;
    }
    for rule in &config.mouse_rules {
        if !rule.global && rule.from_mods.matches(&mods) && rule.button == button {
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
    for rule in &config.scroll_rules {
        if rule.global && rule.from_mods.matches(&mods) {
            return ScrollAction::Remap { mods: rule.to_mods };
        }
    }
    if config.excluded_apps.contains(bundle_id) {
        return ScrollAction::Passthrough;
    }
    for rule in &config.scroll_rules {
        if !rule.global && rule.from_mods.matches(&mods) {
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
            "ralt" | "altgr" => {
                result.ralt = true;
                result.alt = true;
            }
            other => eprintln!("[keyremap] unknown modifier: {other}"),
        }
    }
    result
}

fn resolve_char_rule(rule: &CharRule) -> Option<ResolvedCharRule> {
    let from_key = keycode::parse_key(&rule.from_key)?;
    Some(ResolvedCharRule {
        from_mods: parse_modifiers(&rule.from_mods),
        from_key,
        to_char: rule.to_char.clone(),
        global: rule.global,
    })
}

fn resolve_key_rule(rule: &KeyRule) -> Vec<ResolvedKeyRule> {
    match rule {
        KeyRule::Batch { from_mods, to_mods, keys, global } => {
            let from = parse_modifiers(from_mods);
            let to = parse_modifiers(to_mods);
            let global = *global;
            keys.iter()
                .filter_map(|k| {
                    let code = keycode::parse_key(k)?;
                    Some(ResolvedKeyRule {
                        from_mods: from,
                        from_key: code,
                        to_mods: to,
                        to_key: code,
                        global,
                    })
                })
                .collect()
        }
        KeyRule::Single { from_mods, from_key, to_mods, to_key, global } => {
            let Some(fk) = keycode::parse_key(from_key) else { return vec![] };
            let Some(tk) = keycode::parse_key(to_key) else { return vec![] };
            vec![ResolvedKeyRule {
                from_mods: parse_modifiers(from_mods),
                from_key: fk,
                to_mods: parse_modifiers(to_mods),
                to_key: tk,
                global: *global,
            }]
        }
    }
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
        global: rule.global,
    })
}

fn resolve_scroll_rule(rule: &ScrollRule) -> Option<ResolvedScrollRule> {
    Some(ResolvedScrollRule {
        from_mods: parse_modifiers(&rule.from_mods),
        to_mods: parse_modifiers(&rule.to_mods),
        global: rule.global,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn test_config() -> ResolvedConfig {
        let raw: RemapConfig =
            serde_json::from_str(include_str!("../config/default.json")).unwrap();
        resolve(&raw)
    }

    fn arb_modifiers() -> impl Strategy<Value = Modifiers> {
        (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
            |(ctrl, shift, alt, cmd, ralt)| Modifiers {
                ctrl,
                shift,
                alt,
                cmd,
                ralt,
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
            let empty = RemapConfig {
                enabled: true,
                excluded_apps: vec![],
                char_rules: vec![],
                key_rules: vec![],
                mouse_rules: vec![],
                scroll_rules: vec![],
            };
            let config = resolve(&empty);
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
            let ctrl_only = Modifiers { ctrl: true, shift: false, alt: false, cmd: false, ralt: false };
            let result = process_key_event(&config, ctrl_only, crate::keycode::ANSI_C, &app);
            prop_assert_eq!(result, KeyAction::Remap {
                mods: Modifiers { ctrl: false, shift: false, alt: false, cmd: true, ralt: false },
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
                enabled: true,
                excluded_apps: vec![],
                key_rules: vec![
                    crate::config::KeyRule::Batch {
                        from_mods: vec!["ctrl".into()],
                        to_mods: vec!["cmd".into()],
                        keys: vec!["c".into()],
                    },
                    crate::config::KeyRule::Single {
                        from_mods: vec!["ctrl".into()],
                        from_key: "c".into(),
                        to_mods: vec!["alt".into()],
                        to_key: "x".into(),
                    },
                ],
                char_rules: vec![],
                mouse_rules: vec![],
                scroll_rules: vec![],
            };
            let config = resolve(&raw);
            let ctrl_only = Modifiers { ctrl: true, shift: false, alt: false, cmd: false, ralt: false };
            let result = process_key_event(&config, ctrl_only, crate::keycode::ANSI_C, &app);
            prop_assert_eq!(result, KeyAction::Remap {
                mods: Modifiers { ctrl: false, shift: false, alt: false, cmd: true, ralt: false },
                key: crate::keycode::ANSI_C,
            });
        }
    }
}
