#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::super::super::de_bindings::normalize_combo;
use super::super::super::diagnosis::FixAction;
use super::{DetectedShadow, ShadowKind};
use std::collections::BTreeMap;
use std::process::Command;

pub(super) fn collect_shadows(qol_index: &BTreeMap<String, String>) -> Vec<DetectedShadow> {
    collect_shadows_with_reader(qol_index, &mut DefaultsCli)
}

pub(crate) trait SymbolicHotkeyReader {
    fn read_enabled_combos(&mut self) -> Vec<EnabledHotkey>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnabledHotkey {
    pub id: u32,
    pub key: SymbolicKey,
    pub modifiers: u32,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SymbolicKey {
    Ascii(u32),
    Virtual(u32),
}

const APPLE_MOD_SHIFT: u32 = 0x20000;
const APPLE_MOD_CONTROL: u32 = 0x40000;
const APPLE_MOD_OPTION: u32 = 0x80000;
const APPLE_MOD_COMMAND: u32 = 0x100000;

const VK_SPACE: u32 = 49;
const VK_TAB: u32 = 48;
const VK_BACKTICK: u32 = 50;

pub(crate) struct ReservedDefinition {
    pub id: u32,
    pub label: &'static str,
    pub combo: &'static str,
    pub hint: &'static str,
}

pub(crate) const RESERVED: &[ReservedDefinition] = &[
    ReservedDefinition {
        id: 0,
        label: "macOS App Switcher",
        combo: "Cmd+Tab",
        hint: "Cmd+Tab is owned by macOS; remap qol-tray's Alt-Tab plugin to a different combo or accept the conflict",
    },
];

pub(crate) struct FixableDefinition {
    pub id: u32,
    pub label: &'static str,
    pub combo: &'static str,
}

pub(crate) const FIXABLE: &[FixableDefinition] = &[
    FixableDefinition {
        id: 64,
        label: "Spotlight",
        combo: "Cmd+Space",
    },
    FixableDefinition {
        id: 65,
        label: "Spotlight (Finder window)",
        combo: "Cmd+Opt+Space",
    },
    FixableDefinition {
        id: 60,
        label: "Previous input source",
        combo: "Ctrl+Space",
    },
    FixableDefinition {
        id: 61,
        label: "Next input source",
        combo: "Ctrl+Opt+Space",
    },
    FixableDefinition {
        id: 27,
        label: "Move focus to next window in app",
        combo: "Cmd+`",
    },
];

struct DefaultsCli;

impl SymbolicHotkeyReader for DefaultsCli {
    fn read_enabled_combos(&mut self) -> Vec<EnabledHotkey> {
        let output = Command::new("defaults")
            .args(["read", "com.apple.symbolichotkeys", "AppleSymbolicHotKeys"])
            .output()
            .ok();
        let Some(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        parse_defaults_output(&String::from_utf8_lossy(&output.stdout))
    }
}

pub(crate) fn collect_shadows_with_reader(
    qol_index: &BTreeMap<String, String>,
    reader: &mut dyn SymbolicHotkeyReader,
) -> Vec<DetectedShadow> {
    let enabled = reader.read_enabled_combos();
    let mut shadows = Vec::new();
    for hotkey in enabled.iter().filter(|h| h.enabled) {
        if let Some(reserved) = match_reserved(hotkey) {
            if let Some(qol_combo) =
                qol_index.get(&normalize_combo(reserved.combo).unwrap_or_default())
            {
                shadows.push(DetectedShadow {
                    qol_combo: qol_combo.clone(),
                    source_label: reserved.label.to_string(),
                    kind: ShadowKind::Reserved {
                        hint: reserved.hint.to_string(),
                    },
                });
            }
            continue;
        }
        if let Some(fixable) = match_fixable(hotkey) {
            if let Some(qol_combo) =
                qol_index.get(&normalize_combo(fixable.combo).unwrap_or_default())
            {
                shadows.push(DetectedShadow {
                    qol_combo: qol_combo.clone(),
                    source_label: fixable.label.to_string(),
                    kind: ShadowKind::Fixable(FixAction::DisableSymbolicHotkey {
                        hotkey_id: fixable.id,
                        qol_combo: qol_combo.clone(),
                    }),
                });
            }
        }
    }
    shadows
}

fn match_fixable(hotkey: &EnabledHotkey) -> Option<&'static FixableDefinition> {
    FIXABLE.iter().find(|def| def.id == hotkey.id)
}

fn match_reserved(hotkey: &EnabledHotkey) -> Option<&'static ReservedDefinition> {
    RESERVED.iter().find(|def| def.id == hotkey.id)
}

pub(crate) fn parse_defaults_output(text: &str) -> Vec<EnabledHotkey> {
    let mut hotkeys = Vec::new();
    for entry in split_top_level_entries(text) {
        if let Some(hotkey) = parse_one_entry(&entry) {
            hotkeys.push(hotkey);
        }
    }
    hotkeys
}

fn split_top_level_entries(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let inner = trimmed.trim_start_matches('{').trim_end_matches('}');
    let mut entries = Vec::new();
    let mut depth = 0u32;
    let mut current_id: Option<String> = None;
    let mut buf = String::new();
    let mut id_buf = String::new();
    let mut after_eq = false;
    for line in inner.lines() {
        let line = line.trim_end();
        if depth == 0 {
            if let Some(idx) = line.find('=') {
                id_buf.clear();
                id_buf.push_str(line[..idx].trim());
                current_id = Some(id_buf.clone());
                after_eq = true;
                let rest = line[idx + 1..].trim();
                if rest.starts_with('{') {
                    depth = 1;
                    buf.clear();
                    buf.push_str(&format!("ID={}\n{}\n", id_buf, rest));
                    continue;
                }
            }
            continue;
        }
        let _ = after_eq;
        let _ = current_id;
        buf.push_str(line);
        buf.push('\n');
        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
            } else if ch == '}' {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    entries.push(std::mem::take(&mut buf));
                    break;
                }
            }
        }
    }
    entries
}

fn parse_one_entry(text: &str) -> Option<EnabledHotkey> {
    let id = extract_id(text)?;
    let enabled = extract_enabled(text);
    let parameters = extract_parameters(text)?;
    if parameters.len() < 3 {
        return None;
    }
    let ascii = parameters[0];
    let virt = parameters[1];
    let modifiers = parameters[2];
    let key = if virt == 0xFFFF {
        SymbolicKey::Ascii(ascii)
    } else {
        SymbolicKey::Virtual(virt)
    };
    Some(EnabledHotkey {
        id,
        key,
        modifiers,
        enabled,
    })
}

fn extract_id(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("ID=").and_then(|v| v.trim().parse().ok())
    })
}

fn extract_enabled(text: &str) -> bool {
    text.lines()
        .find_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix("enabled")?.trim_start();
            let value = rest.strip_prefix('=')?.trim().trim_end_matches(';').trim();
            Some(matches!(value, "1" | "true" | "YES"))
        })
        .unwrap_or(true)
}

fn extract_parameters(text: &str) -> Option<Vec<u32>> {
    let mut iter = text.lines();
    while let Some(line) = iter.next() {
        let line = line.trim();
        if !line.starts_with("parameters") {
            continue;
        }
        let mut collected = String::new();
        let after_eq = line.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
        if !after_eq.is_empty() {
            collected.push_str(after_eq);
        }
        for next in iter.by_ref() {
            collected.push(' ');
            collected.push_str(next.trim());
            if collected.contains(')') {
                break;
            }
        }
        let inside = collected
            .split_once('(')
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(inside, _)| inside)?;
        let parts: Vec<u32> = inside
            .split(',')
            .filter_map(|tok| tok.trim().parse().ok())
            .collect();
        if parts.is_empty() {
            return None;
        }
        return Some(parts);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubReader(Vec<EnabledHotkey>);

    impl SymbolicHotkeyReader for StubReader {
        fn read_enabled_combos(&mut self) -> Vec<EnabledHotkey> {
            self.0.clone()
        }
    }

    fn enabled(id: u32) -> EnabledHotkey {
        EnabledHotkey {
            id,
            key: SymbolicKey::Virtual(0),
            modifiers: 0,
            enabled: true,
        }
    }

    fn index(combos: &[&str]) -> BTreeMap<String, String> {
        combos
            .iter()
            .filter_map(|c| normalize_combo(c).map(|n| (n, (*c).to_string())))
            .collect()
    }

    #[test]
    fn fixable_id_emits_disable_symbolic_hotkey_action() {
        let cases = [(64u32, "Cmd+Space"), (60, "Ctrl+Space"), (27, "Cmd+`")];
        for (id, qol_combo) in cases {
            let qol = index(&[qol_combo]);
            let mut reader = StubReader(vec![enabled(id)]);
            let shadows = collect_shadows_with_reader(&qol, &mut reader);
            assert_eq!(
                shadows.len(),
                1,
                "id {id} ({qol_combo}) should produce one shadow"
            );
            match &shadows[0].kind {
                ShadowKind::Fixable(FixAction::DisableSymbolicHotkey {
                    hotkey_id,
                    qol_combo: c,
                }) => {
                    assert_eq!(*hotkey_id, id);
                    assert_eq!(c, qol_combo);
                }
                other => panic!("expected DisableSymbolicHotkey for id {id}, got {other:?}"),
            }
        }
    }

    #[test]
    fn reserved_id_emits_reserved_kind_with_hint() {
        let qol = index(&["Cmd+Tab"]);
        let mut reader = StubReader(vec![enabled(0)]);
        let shadows = collect_shadows_with_reader(&qol, &mut reader);
        assert_eq!(shadows.len(), 1);
        match &shadows[0].kind {
            ShadowKind::Reserved { hint } => {
                assert!(
                    hint.contains("Cmd+Tab"),
                    "hint should mention combo: {hint}"
                );
            }
            other => panic!("expected Reserved, got {other:?}"),
        }
        assert_eq!(shadows[0].source_label, "macOS App Switcher");
    }

    #[test]
    fn no_shadow_when_qol_combo_not_in_index() {
        let qol = index(&["Ctrl+Alt+Shift+F12"]);
        let mut reader = StubReader(vec![enabled(64), enabled(0)]);
        assert!(collect_shadows_with_reader(&qol, &mut reader).is_empty());
    }

    #[test]
    fn disabled_hotkeys_are_ignored() {
        let qol = index(&["Cmd+Space"]);
        let mut hotkey = enabled(64);
        hotkey.enabled = false;
        let mut reader = StubReader(vec![hotkey]);
        assert!(collect_shadows_with_reader(&qol, &mut reader).is_empty());
    }

    #[test]
    fn unknown_id_is_ignored() {
        let qol = index(&["Cmd+Space", "Cmd+Tab"]);
        let mut reader = StubReader(vec![enabled(9999)]);
        assert!(collect_shadows_with_reader(&qol, &mut reader).is_empty());
    }

    #[test]
    fn parse_defaults_output_extracts_id_and_parameters() {
        let sample = r#"{
    27 =     {
        enabled = 1;
        value =         {
            parameters =             (
                96,
                50,
                1048576
            );
            type = standard;
        };
    };
    64 =     {
        enabled = 0;
        value =         {
            parameters =             (
                32,
                49,
                1048576
            );
            type = standard;
        };
    };
}"#;
        let parsed = parse_defaults_output(sample);
        let by_id: BTreeMap<u32, EnabledHotkey> = parsed.into_iter().map(|h| (h.id, h)).collect();
        let twenty_seven = by_id.get(&27).expect("id 27 parsed");
        assert!(twenty_seven.enabled);
        assert_eq!(twenty_seven.modifiers, 1048576);
        let sixty_four = by_id.get(&64).expect("id 64 parsed");
        assert!(!sixty_four.enabled, "enabled = 0 must be parsed false");
    }
}
