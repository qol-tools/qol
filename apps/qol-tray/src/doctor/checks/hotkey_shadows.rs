use super::super::diagnosis::{ok_outcome, warn_outcome, Diagnosis};
use crate::hotkeys::{HotkeyBinding, HotkeyManager};
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

const ID: &str = "hotkey_shadows";

const GSETTINGS_PROBES: &[(&str, &str)] = &[
    ("org.cinnamon.desktop.keybindings.wm", "switch-input-source"),
    (
        "org.cinnamon.desktop.keybindings.wm",
        "switch-input-source-backward",
    ),
    (
        "org.cinnamon.desktop.keybindings.wm",
        "activate-window-menu",
    ),
    ("org.cinnamon.desktop.keybindings.wm", "panel-main-menu"),
    ("org.gnome.desktop.wm.keybindings", "switch-input-source"),
    (
        "org.gnome.desktop.wm.keybindings",
        "switch-input-source-backward",
    ),
    ("org.gnome.desktop.wm.keybindings", "panel-main-menu"),
    ("org.freedesktop.ibus.general.hotkey", "triggers"),
];

pub(super) fn check() -> Diagnosis {
    let bindings = match enabled_bindings() {
        Ok(bindings) => bindings,
        Err(error) => {
            return ok_outcome(ID, format!("could not load hotkey config: {error}"));
        }
    };
    if bindings.is_empty() {
        return ok_outcome(ID, "no hotkeys configured".into());
    }

    let shadows = collect_shadows(&bindings, &mut GSettingsLookup);
    if shadows.is_empty() {
        return ok_outcome(ID, "no DE keybinding conflicts detected".into());
    }
    warn_outcome(ID, format_message(&shadows), None)
}

fn enabled_bindings() -> anyhow::Result<Vec<HotkeyBinding>> {
    let manager = HotkeyManager::new()?;
    let config = manager.load_config()?;
    Ok(config.hotkeys.into_iter().filter(|h| h.enabled).collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shadow {
    pub qol_combo: String,
    pub schema: String,
    pub key: String,
    pub conflicting_value: String,
}

pub(crate) trait GSettingsReader {
    fn read(&mut self, schema: &str, key: &str) -> Option<String>;
}

struct GSettingsLookup;

impl GSettingsReader for GSettingsLookup {
    fn read(&mut self, schema: &str, key: &str) -> Option<String> {
        let output = Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

pub(crate) fn collect_shadows(
    bindings: &[HotkeyBinding],
    reader: &mut dyn GSettingsReader,
) -> Vec<Shadow> {
    let qol_index: BTreeMap<String, String> = bindings
        .iter()
        .filter_map(|b| normalize_combo(&b.key).map(|n| (n, b.key.clone())))
        .collect();

    let mut shadows = Vec::new();
    for (schema, key) in GSETTINGS_PROBES {
        let Some(raw) = reader.read(schema, key) else {
            continue;
        };
        for value in parse_gsettings_list(&raw) {
            let Some(norm) = normalize_combo(&value) else {
                continue;
            };
            if let Some(qol_combo) = qol_index.get(&norm) {
                shadows.push(Shadow {
                    qol_combo: qol_combo.clone(),
                    schema: (*schema).into(),
                    key: (*key).into(),
                    conflicting_value: value,
                });
            }
        }
    }
    shadows
}

pub(crate) fn parse_gsettings_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "@as []" || trimmed == "[]" {
        return Vec::new();
    }
    let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|tok| tok.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub(crate) fn normalize_combo(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let tokens = tokenize(&lower);

    let mut mods = BTreeSet::new();
    let mut keys = Vec::new();
    for token in tokens {
        match token.as_str() {
            "ctrl" | "control" | "primary" => {
                mods.insert("ctrl");
            }
            "shift" => {
                mods.insert("shift");
            }
            "alt" | "mod1" => {
                mods.insert("alt");
            }
            "super" | "mod4" | "meta" | "win" => {
                mods.insert("super");
            }
            "" => {}
            _ => keys.push(token),
        }
    }
    if keys.is_empty() {
        return None;
    }
    let mods_str: Vec<&str> = mods.into_iter().collect();
    let key_str = keys.join("+");
    Some(if mods_str.is_empty() {
        key_str
    } else {
        format!("{}+{}", mods_str.join("+"), key_str)
    })
}

fn tokenize(lower: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut in_brackets = false;
    for c in lower.chars() {
        match c {
            '<' => {
                push_if_nonempty(&mut buf, &mut tokens);
                in_brackets = true;
            }
            '>' if in_brackets => {
                push_if_nonempty(&mut buf, &mut tokens);
                in_brackets = false;
            }
            '+' if !in_brackets => push_if_nonempty(&mut buf, &mut tokens),
            ' ' | '\t' => {}
            _ => buf.push(c),
        }
    }
    push_if_nonempty(&mut buf, &mut tokens);
    tokens
}

fn push_if_nonempty(buf: &mut String, out: &mut Vec<String>) {
    let token = std::mem::take(buf);
    let trimmed = token.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
}

fn format_message(shadows: &[Shadow]) -> String {
    let mut by_combo: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for shadow in shadows {
        by_combo
            .entry(shadow.qol_combo.as_str())
            .or_default()
            .push(format!("{}.{}", shadow.schema, shadow.key));
    }
    let parts: Vec<String> = by_combo
        .into_iter()
        .map(|(combo, sources)| format!("{combo} also bound in {}", sources.join(", ")))
        .collect();
    format!(
        "hotkey shadow detected (qol-tray's grab may silently lose to a desktop-environment shortcut): {}",
        parts.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_handles_qol_format() {
        let cases = [
            ("Super+Space", Some("super+space")),
            ("Shift+Super+R", Some("shift+super+r")),
            ("Ctrl+Alt+Shift+Down", Some("alt+ctrl+shift+down")),
            ("Alt+Tab", Some("alt+tab")),
            ("Super+Down", Some("super+down")),
        ];
        for (input, want) in cases {
            assert_eq!(
                normalize_combo(input).as_deref(),
                want,
                "qol input: {input}"
            );
        }
    }

    #[test]
    fn normalize_handles_gtk_format() {
        let cases = [
            ("<Super>space", Some("super+space")),
            ("<Shift><Super>R", Some("shift+super+r")),
            ("<Primary><Alt>Delete", Some("alt+ctrl+delete")),
            ("<Mod4>F1", Some("super+f1")),
            ("<Alt>Tab", Some("alt+tab")),
        ];
        for (input, want) in cases {
            assert_eq!(
                normalize_combo(input).as_deref(),
                want,
                "gtk input: {input}"
            );
        }
    }

    #[test]
    fn qol_and_gtk_match_on_canonical_form() {
        let pairs = [
            ("Super+Space", "<Super>space"),
            ("Shift+Super+R", "<Shift><Super>R"),
            ("Ctrl+Alt+Delete", "<Primary><Alt>Delete"),
        ];
        for (qol, gtk) in pairs {
            assert_eq!(normalize_combo(qol), normalize_combo(gtk), "{qol} vs {gtk}");
        }
    }

    #[test]
    fn normalize_returns_none_for_modifier_only_or_special() {
        assert_eq!(normalize_combo("<Super>"), None);
        assert_eq!(normalize_combo("Shift+Ctrl"), None);
        assert_eq!(normalize_combo(""), None);
    }

    #[test]
    fn normalize_treats_xf86_keyboard_as_distinct_token() {
        assert_eq!(
            normalize_combo("XF86Keyboard").as_deref(),
            Some("xf86keyboard")
        );
        assert!(normalize_combo("XF86Keyboard") != normalize_combo("Super+Space"));
    }

    #[test]
    fn parse_gsettings_list_handles_common_shapes() {
        let cases = [
            (
                "['<Super>space', 'XF86Keyboard']",
                vec!["<Super>space", "XF86Keyboard"],
            ),
            ("['<Super>space']", vec!["<Super>space"]),
            ("@as []", Vec::<&str>::new()),
            ("[]", Vec::<&str>::new()),
            ("", Vec::<&str>::new()),
        ];
        for (raw, want) in cases {
            let got: Vec<String> = parse_gsettings_list(raw);
            let want_owned: Vec<String> = want.into_iter().map(String::from).collect();
            assert_eq!(got, want_owned, "raw: {raw}");
        }
    }

    struct StubReader<'a>(&'a [(&'a str, &'a str, &'a str)]);

    impl GSettingsReader for StubReader<'_> {
        fn read(&mut self, schema: &str, key: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(s, k, _)| *s == schema && *k == key)
                .map(|(_, _, v)| (*v).to_string())
        }
    }

    fn binding(key: &str) -> HotkeyBinding {
        HotkeyBinding {
            id: format!("hk-{key}"),
            key: key.to_string(),
            plugin_id: "test-plugin".into(),
            action: "open".into(),
            enabled: true,
        }
    }

    #[test]
    fn collect_shadows_finds_overlap_in_cinnamon_schema() {
        let bindings = vec![binding("Super+Space"), binding("Alt+Tab")];
        let stub = [(
            "org.cinnamon.desktop.keybindings.wm",
            "switch-input-source",
            "['<Super>space', 'XF86Keyboard']",
        )];
        let mut reader = StubReader(&stub);
        let shadows = collect_shadows(&bindings, &mut reader);
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].qol_combo, "Super+Space");
        assert_eq!(shadows[0].schema, "org.cinnamon.desktop.keybindings.wm");
        assert_eq!(shadows[0].key, "switch-input-source");
        assert_eq!(shadows[0].conflicting_value, "<Super>space");
    }

    #[test]
    fn collect_shadows_finds_ibus_overlap() {
        let bindings = vec![binding("Super+Space")];
        let stub = [(
            "org.freedesktop.ibus.general.hotkey",
            "triggers",
            "['<Super>space']",
        )];
        let mut reader = StubReader(&stub);
        let shadows = collect_shadows(&bindings, &mut reader);
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].schema, "org.freedesktop.ibus.general.hotkey");
    }

    #[test]
    fn collect_shadows_groups_multiple_sources_per_combo() {
        let bindings = vec![binding("Super+Space")];
        let stub = [
            (
                "org.cinnamon.desktop.keybindings.wm",
                "switch-input-source",
                "['<Super>space']",
            ),
            (
                "org.freedesktop.ibus.general.hotkey",
                "triggers",
                "['<Super>space']",
            ),
        ];
        let mut reader = StubReader(&stub);
        let shadows = collect_shadows(&bindings, &mut reader);
        assert_eq!(shadows.len(), 2);
        let message = format_message(&shadows);
        assert!(message.contains("Super+Space also bound in"));
        assert!(message.contains("org.cinnamon.desktop.keybindings.wm.switch-input-source"));
        assert!(message.contains("org.freedesktop.ibus.general.hotkey.triggers"));
    }

    #[test]
    fn collect_shadows_returns_empty_when_no_overlap() {
        let bindings = vec![binding("Ctrl+Alt+Shift+F12")];
        let stub = [(
            "org.cinnamon.desktop.keybindings.wm",
            "switch-input-source",
            "['<Super>space']",
        )];
        let mut reader = StubReader(&stub);
        assert!(collect_shadows(&bindings, &mut reader).is_empty());
    }

    #[test]
    fn collect_shadows_ignores_disabled_bindings() {
        let mut disabled = binding("Super+Space");
        disabled.enabled = false;
        let bindings = [disabled];
        let qol_index: BTreeMap<String, String> = bindings
            .iter()
            .filter(|b| b.enabled)
            .filter_map(|b| normalize_combo(&b.key).map(|n| (n, b.key.clone())))
            .collect();
        assert!(qol_index.is_empty());
    }
}
