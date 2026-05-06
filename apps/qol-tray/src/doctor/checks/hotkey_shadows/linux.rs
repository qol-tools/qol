#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::super::super::de_bindings::{normalize_combo, parse_gsettings_list};
use super::super::super::diagnosis::FixAction;
use super::{DetectedShadow, ShadowKind};
use std::collections::BTreeMap;
use std::process::Command;

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

pub(super) fn collect_shadows(qol_index: &BTreeMap<String, String>) -> Vec<DetectedShadow> {
    collect_shadows_with_reader(qol_index, &mut GSettingsLookup)
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

pub(crate) fn collect_shadows_with_reader(
    qol_index: &BTreeMap<String, String>,
    reader: &mut dyn GSettingsReader,
) -> Vec<DetectedShadow> {
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
                shadows.push(DetectedShadow {
                    qol_combo: qol_combo.clone(),
                    source_label: format!("{schema}.{key}"),
                    kind: ShadowKind::Fixable(FixAction::UnshadowDeBinding {
                        schema: (*schema).into(),
                        key: (*key).into(),
                        qol_combo: qol_combo.clone(),
                    }),
                });
            }
        }
    }
    shadows
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubReader<'a>(&'a [(&'a str, &'a str, &'a str)]);

    impl GSettingsReader for StubReader<'_> {
        fn read(&mut self, schema: &str, key: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(s, k, _)| *s == schema && *k == key)
                .map(|(_, _, v)| (*v).to_string())
        }
    }

    fn index(combos: &[&str]) -> BTreeMap<String, String> {
        combos
            .iter()
            .filter_map(|c| normalize_combo(c).map(|n| (n, (*c).to_string())))
            .collect()
    }

    #[test]
    fn collect_finds_overlap_in_cinnamon_schema() {
        let qol = index(&["Super+Space", "Alt+Tab"]);
        let stub = [(
            "org.cinnamon.desktop.keybindings.wm",
            "switch-input-source",
            "['<Super>space', 'XF86Keyboard']",
        )];
        let shadows = collect_shadows_with_reader(&qol, &mut StubReader(&stub));
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].qol_combo, "Super+Space");
        assert_eq!(
            shadows[0].source_label,
            "org.cinnamon.desktop.keybindings.wm.switch-input-source"
        );
        assert!(matches!(
            &shadows[0].kind,
            ShadowKind::Fixable(FixAction::UnshadowDeBinding { schema, key, qol_combo })
                if schema == "org.cinnamon.desktop.keybindings.wm"
                    && key == "switch-input-source"
                    && qol_combo == "Super+Space"
        ));
    }

    #[test]
    fn collect_finds_ibus_overlap() {
        let qol = index(&["Super+Space"]);
        let stub = [(
            "org.freedesktop.ibus.general.hotkey",
            "triggers",
            "['<Super>space']",
        )];
        let shadows = collect_shadows_with_reader(&qol, &mut StubReader(&stub));
        assert_eq!(shadows.len(), 1);
        assert!(shadows[0]
            .source_label
            .starts_with("org.freedesktop.ibus.general.hotkey"));
    }

    #[test]
    fn collect_groups_multiple_sources_per_combo() {
        let qol = index(&["Super+Space"]);
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
        let shadows = collect_shadows_with_reader(&qol, &mut StubReader(&stub));
        assert_eq!(shadows.len(), 2);
    }

    #[test]
    fn collect_returns_empty_when_no_overlap() {
        let qol = index(&["Ctrl+Alt+Shift+F12"]);
        let stub = [(
            "org.cinnamon.desktop.keybindings.wm",
            "switch-input-source",
            "['<Super>space']",
        )];
        assert!(collect_shadows_with_reader(&qol, &mut StubReader(&stub)).is_empty());
    }
}
