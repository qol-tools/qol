use qol_hotkeys::macos_keycode::PhysicalLayout;
use serde_json::{json, Value};

use super::hotkeys::Hotkeys;
use super::key_codes;
use super::system_modifier_keys::DeviceModifiers;
use crate::platform::engine::config::{ModifierKeys, ModifierTarget};
use crate::platform::engine::remap::{self, Modifiers, ResolvedConfig, ResolvedKeyTarget};
use crate::platform::macos::layout::{KeyStroke, LayoutSnapshot};

pub(crate) const RULE_DESCRIPTION: &str = "qol-keyremap (managed, edit rules in qol Key Remap)";
pub(crate) const ACTIVE_VARIABLE: &str = "qol_keyremap_active";

#[derive(Debug)]
pub(crate) struct Compiled {
    pub(crate) rule: Value,
    pub(crate) simple_modifications: Vec<Value>,
    pub(crate) gaps: Vec<Gap>,
    pub(crate) device_modifications: Vec<DeviceModifiers>,
}

impl Compiled {
    pub(crate) fn with_system_modifier_keys(mut self, devices: Vec<DeviceModifiers>) -> Self {
        self.device_modifications = devices;
        self
    }

    pub(crate) fn with_hotkeys(mut self, hotkeys: &Hotkeys, physical: PhysicalLayout) -> Self {
        let (manipulators, gaps) = hotkeys.manipulators(physical);
        if let Some(all) = self.rule["manipulators"].as_array_mut() {
            all.extend(manipulators);
        }
        self.gaps.extend(gaps);
        self
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Gap {
    pub(crate) rule: String,
    pub(crate) reason: String,
}

pub(crate) fn compile(config: &ResolvedConfig, layout: &LayoutSnapshot) -> Compiled {
    let mut compiler = Compiler {
        layout,
        excluded: excluded_condition(config),
        manipulators: Vec::new(),
        gaps: Vec::new(),
    };
    for global in [true, false] {
        for rule in config
            .char_swap_rules
            .iter()
            .filter(|rule| rule.global == global)
        {
            compiler.char_swap(&rule.from_char, &rule.to_char, global);
        }
        for rule in config
            .char_rules
            .iter()
            .filter(|rule| rule.global == global)
        {
            let label = format!(
                "char rule {} -> {}",
                remap::rule_label(&rule.from_mods, rule.from_key),
                rule.to_char
            );
            compiler.key_to_text(&label, rule.from_mods, rule.from_key, &rule.to_char, global);
        }
        for rule in config.key_rules.iter().filter(|rule| rule.global == global) {
            compiler.key_rule(rule, global);
        }
    }
    Compiled {
        rule: json!({
            "description": RULE_DESCRIPTION,
            "manipulators": compiler.manipulators,
        }),
        simple_modifications: simple_modifications(&config.modifier_keys),
        gaps: compiler.gaps,
        device_modifications: Vec::new(),
    }
}

struct Compiler<'a> {
    layout: &'a LayoutSnapshot,
    excluded: Option<Value>,
    manipulators: Vec<Value>,
    gaps: Vec<Gap>,
}

impl Compiler<'_> {
    fn key_rule(&mut self, rule: &remap::ResolvedKeyRule, global: bool) {
        let source = remap::rule_label(&rule.from_mods, rule.from_key);
        match &rule.to {
            ResolvedKeyTarget::Key { mods, key } => {
                let label = format!("key rule {source} -> {}", remap::rule_label(mods, *key));
                let Some(from) = self.source_key(&label, rule.from_mods, rule.from_key) else {
                    return;
                };
                let Some(to) = self.target_key(&label, *mods, *key) else {
                    return;
                };
                self.push(from, vec![to], global, false);
            }
            ResolvedKeyTarget::Char { text } => {
                let label = format!("key rule {source} -> {text}");
                self.key_to_text(&label, rule.from_mods, rule.from_key, text, global);
            }
        }
    }

    fn key_to_text(&mut self, label: &str, mods: Modifiers, key: u16, text: &str, global: bool) {
        let Some(from) = self.source_key(label, mods, key) else {
            return;
        };
        let Some(to) = self.typing(label, text) else {
            return;
        };
        self.push(from, to, global, true);
    }

    fn char_swap(&mut self, from_char: &str, to_char: &str, global: bool) {
        let label = format!("char swap {from_char} -> {to_char}");
        let mut chars = from_char.chars();
        let stroke = match (chars.next(), chars.next()) {
            (Some(symbol), None) => self.layout.table.stroke_for(symbol),
            _ => None,
        };
        let Some(stroke) = stroke else {
            self.gap(
                &label,
                format!("no single key types {from_char:?} in this layout"),
            );
            return;
        };
        let Some(key_code) = self.key_code(&label, stroke.keycode) else {
            return;
        };
        let mut mandatory = Vec::new();
        if stroke.shift {
            mandatory.push("shift");
        }
        if stroke.option {
            mandatory.push("option");
        }
        let from = from_event(key_code, mandatory);
        let Some(to) = self.typing(&label, to_char) else {
            return;
        };
        self.push(from, to, global, true);
    }

    fn source_key(&mut self, label: &str, mods: Modifiers, key: u16) -> Option<Value> {
        let key_code = self.key_code(label, key)?;
        let mut mandatory = Vec::new();
        if mods.ctrl {
            mandatory.push("control");
        }
        if mods.shift {
            mandatory.push("shift");
        }
        if mods.ralt {
            mandatory.push("right_option");
        } else if mods.alt {
            mandatory.push("option");
        }
        if mods.cmd {
            mandatory.push("command");
        }
        Some(from_event(key_code, mandatory))
    }

    fn target_key(&mut self, label: &str, mods: Modifiers, key: u16) -> Option<Value> {
        let key_code = self.key_code(label, key)?;
        let mut modifiers = Vec::new();
        if mods.ctrl {
            modifiers.push("left_control");
        }
        if mods.shift {
            modifiers.push("left_shift");
        }
        if mods.alt {
            modifiers.push("left_option");
        }
        if mods.cmd {
            modifiers.push("left_command");
        }
        Some(to_event(key_code, modifiers))
    }

    fn typing(&mut self, label: &str, text: &str) -> Option<Vec<Value>> {
        let Some(strokes) = self.layout.table.sequence_for(text) else {
            self.gap(
                label,
                format!("{text:?} has no key sequence in this layout"),
            );
            return None;
        };
        strokes
            .into_iter()
            .map(|stroke| self.stroke(label, stroke))
            .collect()
    }

    fn stroke(&mut self, label: &str, stroke: KeyStroke) -> Option<Value> {
        let key_code = self.key_code(label, stroke.keycode)?;
        let mut modifiers = Vec::new();
        if stroke.shift {
            modifiers.push("left_shift");
        }
        if stroke.option {
            modifiers.push("left_option");
        }
        Some(to_event(key_code, modifiers))
    }

    fn key_code(&mut self, label: &str, keycode: u16) -> Option<&'static str> {
        let name = key_codes::for_keycode(keycode, self.layout.physical);
        if name.is_none() {
            self.gap(
                label,
                format!("Karabiner has no name for keycode {keycode:#04x}"),
            );
        }
        name
    }

    fn push(&mut self, from: Value, to: Vec<Value>, global: bool, layout_bound: bool) {
        let mut conditions = vec![json!({
            "type": "variable_if",
            "name": ACTIVE_VARIABLE,
            "value": 1,
        })];
        if !global {
            conditions.extend(self.excluded.clone());
        }
        if layout_bound && !self.layout.id.is_empty() {
            conditions.push(json!({
                "type": "input_source_if",
                "input_sources": [{ "input_source_id": exact(&self.layout.id) }],
            }));
        }
        self.manipulators.push(json!({
            "type": "basic",
            "from": from,
            "to": to,
            "conditions": conditions,
        }));
    }

    fn gap(&mut self, rule: &str, reason: String) {
        self.gaps.push(Gap {
            rule: rule.to_string(),
            reason,
        });
    }
}

fn from_event(key_code: &str, mandatory: Vec<&str>) -> Value {
    let mut modifiers = json!({ "optional": ["caps_lock"] });
    if !mandatory.is_empty() {
        modifiers["mandatory"] = json!(mandatory);
    }
    json!({ "key_code": key_code, "modifiers": modifiers })
}

fn to_event(key_code: &str, modifiers: Vec<&str>) -> Value {
    if modifiers.is_empty() {
        return json!({ "key_code": key_code });
    }
    json!({ "key_code": key_code, "modifiers": modifiers })
}

fn excluded_condition(config: &ResolvedConfig) -> Option<Value> {
    let mut apps: Vec<&String> = config.excluded_apps.iter().collect();
    apps.sort();
    (!apps.is_empty()).then(|| {
        json!({
            "type": "frontmost_application_unless",
            "bundle_identifiers": apps.into_iter().map(|app| exact(app)).collect::<Vec<_>>(),
        })
    })
}

fn exact(text: &str) -> String {
    let mut pattern = String::from("^");
    for symbol in text.chars() {
        if "\\.+*?()|[]{}^$".contains(symbol) {
            pattern.push('\\');
        }
        pattern.push(symbol);
    }
    pattern.push('$');
    pattern
}

fn simple_modifications(keys: &ModifierKeys) -> Vec<Value> {
    let sources = [
        (ModifierTarget::CapsLock, false, "caps_lock"),
        (ModifierTarget::Control, false, "left_control"),
        (ModifierTarget::Control, true, "right_control"),
        (ModifierTarget::Option, false, "left_option"),
        (ModifierTarget::Option, true, "right_option"),
        (ModifierTarget::Command, false, "left_command"),
        (ModifierTarget::Command, true, "right_command"),
        (ModifierTarget::Fn, false, "fn"),
    ];
    sources
        .into_iter()
        .filter_map(|(source, right, from)| {
            let target = keys.target_for(source);
            (target != source).then(|| {
                json!({
                    "from": { "key_code": from },
                    "to": [{ "key_code": modifier_key_code(target, right) }],
                })
            })
        })
        .collect()
}

fn modifier_key_code(target: ModifierTarget, right: bool) -> &'static str {
    match (target, right) {
        (ModifierTarget::CapsLock, _) => "caps_lock",
        (ModifierTarget::Control, false) => "left_control",
        (ModifierTarget::Control, true) => "right_control",
        (ModifierTarget::Option, false) => "left_option",
        (ModifierTarget::Option, true) => "right_option",
        (ModifierTarget::Command, false) => "left_command",
        (ModifierTarget::Command, true) => "right_command",
        (ModifierTarget::Fn, _) => "fn",
        (ModifierTarget::Escape, _) => "escape",
        (ModifierTarget::None, _) => "vk_none",
    }
}

#[cfg(test)]
mod tests {
    use qol_hotkeys::macos_keycode::{self as keycode, PhysicalLayout};
    use serde_json::json;

    use super::*;
    use crate::platform::engine::config::RemapConfig;
    use crate::platform::macos::layout::CharTable;

    fn layout() -> LayoutSnapshot {
        LayoutSnapshot {
            id: "com.apple.keylayout.Danish".into(),
            table: CharTable::from_translation(|code, state, _dead| {
                let option = state & qol_hotkeys::layout::OPTION_STATE != 0;
                let shift = state & qol_hotkeys::layout::SHIFT_STATE != 0;
                match (code, shift, option) {
                    (keycode::ANSI_C, false, false) => Some("c".into()),
                    (keycode::ANSI_2, false, true) => Some("@".into()),
                    (keycode::ANSI_4, true, false) => Some("¤".into()),
                    (keycode::ANSI_4, false, true) => Some("$".into()),
                    _ => None,
                }
            }),
            physical: PhysicalLayout::Iso,
        }
    }

    fn compiled(config: serde_json::Value) -> Compiled {
        let config: RemapConfig = serde_json::from_value(config).unwrap();
        compile(&remap::resolve(&config), &layout())
    }

    fn manipulators(compiled: &Compiled) -> &Vec<Value> {
        compiled.rule["manipulators"].as_array().unwrap()
    }

    #[test]
    fn a_key_rule_becomes_one_manipulator_per_key_behind_the_exclusions() {
        let compiled = compiled(json!({
            "excluded_apps": ["net.kovidgoyal.kitty"],
            "key_rules": [{ "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c", "v"] }],
        }));
        assert_eq!(compiled.rule["description"], RULE_DESCRIPTION);
        assert_eq!(
            manipulators(&compiled)[0],
            json!({
                "type": "basic",
                "from": { "key_code": "c", "modifiers": { "mandatory": ["control"], "optional": ["caps_lock"] } },
                "to": [{ "key_code": "c", "modifiers": ["left_command"] }],
                "conditions": [
                    { "type": "variable_if", "name": ACTIVE_VARIABLE, "value": 1 },
                    { "type": "frontmost_application_unless", "bundle_identifiers": ["^net\\.kovidgoyal\\.kitty$"] },
                ],
            })
        );
        assert_eq!(manipulators(&compiled)[1]["from"]["key_code"], "v");
        assert!(compiled.gaps.is_empty());
    }

    #[test]
    fn global_rules_come_first_and_skip_the_exclusions() {
        let compiled = compiled(json!({
            "excluded_apps": ["com.apple.Terminal"],
            "key_rules": [
                { "from_mods": ["ctrl"], "from_key": "y", "to_mods": ["cmd", "shift"], "to_key": "z" },
                { "from_mods": [], "from_key": "return", "to_mods": [], "to_key": "tab", "global": true },
            ],
        }));
        let all = manipulators(&compiled);
        assert_eq!(all[0]["from"]["key_code"], "return_or_enter");
        assert_eq!(all[0]["conditions"].as_array().unwrap().len(), 1);
        assert_eq!(all[1]["from"]["key_code"], "y");
        assert_eq!(
            all[1]["to"][0]["modifiers"],
            json!(["left_shift", "left_command"])
        );
    }

    #[test]
    fn a_right_option_char_rule_types_through_the_current_layout() {
        let compiled = compiled(json!({
            "char_rules": [{ "from_mods": ["ralt"], "from_key": "2", "to_char": "@", "global": true }],
        }));
        let manipulator = &manipulators(&compiled)[0];
        assert_eq!(
            manipulator["from"]["modifiers"]["mandatory"],
            json!(["right_option"])
        );
        assert_eq!(
            manipulator["to"],
            json!([{ "key_code": "2", "modifiers": ["left_option"] }])
        );
        assert_eq!(
            manipulator["conditions"][1],
            json!({ "type": "input_source_if", "input_sources": [{ "input_source_id": "^com\\.apple\\.keylayout\\.Danish$" }] })
        );
    }

    #[test]
    fn a_char_swap_starts_from_the_keys_that_type_it() {
        let compiled = compiled(json!({ "char_swaps": [["¤", "$"]] }));
        let all = manipulators(&compiled);
        assert_eq!(all[0]["from"]["modifiers"]["mandatory"], json!(["shift"]));
        assert_eq!(
            all[0]["to"],
            json!([{ "key_code": "4", "modifiers": ["left_option"] }])
        );
        assert_eq!(all[1]["from"]["modifiers"]["mandatory"], json!(["option"]));
        assert_eq!(
            all[1]["to"],
            json!([{ "key_code": "4", "modifiers": ["left_shift"] }])
        );
    }

    #[test]
    fn a_character_the_layout_cannot_type_is_reported_and_left_out() {
        let compiled = compiled(json!({
            "char_rules": [{ "from_mods": ["ralt"], "from_key": "3", "to_char": "£" }],
        }));
        assert!(manipulators(&compiled).is_empty());
        assert_eq!(compiled.gaps.len(), 1);
        assert!(compiled.gaps[0].rule.contains("£"), "{:?}", compiled.gaps);
    }

    #[test]
    fn modifier_keys_become_simple_modifications_that_keep_their_side() {
        let compiled = compiled(json!({
            "modifier_keys": { "caps_lock": "escape", "option": "command", "command": "option" },
        }));
        assert_eq!(
            compiled.simple_modifications,
            vec![
                json!({ "from": { "key_code": "caps_lock" }, "to": [{ "key_code": "escape" }] }),
                json!({ "from": { "key_code": "left_option" }, "to": [{ "key_code": "left_command" }] }),
                json!({ "from": { "key_code": "right_option" }, "to": [{ "key_code": "right_command" }] }),
                json!({ "from": { "key_code": "left_command" }, "to": [{ "key_code": "left_option" }] }),
                json!({ "from": { "key_code": "right_command" }, "to": [{ "key_code": "right_option" }] }),
            ]
        );
    }

    #[test]
    fn default_modifier_keys_add_nothing() {
        assert!(compiled(json!({})).simple_modifications.is_empty());
    }

    #[test]
    fn bundle_ids_are_escaped_and_anchored() {
        assert_eq!(
            exact("com.todesktop.230313mzl4w4u92"),
            "^com\\.todesktop\\.230313mzl4w4u92$"
        );
        assert_eq!(exact("a+b(c)"), "^a\\+b\\(c\\)$");
    }
}
