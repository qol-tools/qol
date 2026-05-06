#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use super::super::super::de_bindings::normalize_combo;
use super::super::super::diagnosis::FixAction;
use super::{DetectedShadow, ShadowKind};
use std::collections::BTreeMap;
use std::process::Command;

pub(super) fn collect_shadows(qol_index: &BTreeMap<String, String>) -> Vec<DetectedShadow> {
    let mut reader = RegCli;
    collect_shadows_with_reader(qol_index, &mut reader)
}

pub(crate) trait RegistryReader {
    fn read_app_keys(&mut self) -> Vec<AppKeyEntry>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppKeyEntry {
    pub app_key: String,
    pub combo: Option<String>,
}

struct ReservedDefinition {
    label: &'static str,
    combo: &'static str,
    hint: &'static str,
}

const RESERVED: &[ReservedDefinition] = &[
    ReservedDefinition {
        label: "Windows input switcher",
        combo: "Win+Space",
        hint: "open Settings -> Time & language -> Language -> Advanced keyboard settings -> Input language hot keys and clear it",
    },
    ReservedDefinition {
        label: "Windows Task View",
        combo: "Win+Tab",
        hint: "owned by Windows shell; remap qol-tray to a different combo",
    },
    ReservedDefinition {
        label: "Windows Run dialog",
        combo: "Win+R",
        hint: "owned by Windows shell; remap qol-tray to a different combo",
    },
    ReservedDefinition {
        label: "Windows File Explorer",
        combo: "Win+E",
        hint: "owned by Windows shell; remap qol-tray to a different combo",
    },
    ReservedDefinition {
        label: "Windows Lock",
        combo: "Win+L",
        hint: "owned by Windows shell; cannot be reassigned",
    },
];

struct RegCli;

impl RegistryReader for RegCli {
    fn read_app_keys(&mut self) -> Vec<AppKeyEntry> {
        let output = Command::new("reg")
            .args([
                "query",
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\AppKey",
                "/s",
            ])
            .output()
            .ok();
        let Some(output) = output else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        parse_reg_query_output(&String::from_utf8_lossy(&output.stdout))
    }
}

pub(crate) fn collect_shadows_with_reader(
    qol_index: &BTreeMap<String, String>,
    reader: &mut dyn RegistryReader,
) -> Vec<DetectedShadow> {
    let mut reserved_combos: BTreeMap<String, &ReservedDefinition> = BTreeMap::new();
    for reserved in RESERVED {
        if let Some(norm) = normalize_combo(reserved.combo) {
            reserved_combos.insert(norm, reserved);
        }
    }
    let mut shadows = Vec::new();
    for entry in reader.read_app_keys() {
        let Some(combo) = entry.combo.as_ref() else {
            continue;
        };
        let Some(norm) = normalize_combo(combo) else {
            continue;
        };
        if reserved_combos.contains_key(&norm) {
            continue;
        }
        if let Some(qol_combo) = qol_index.get(&norm) {
            shadows.push(DetectedShadow {
                qol_combo: qol_combo.clone(),
                source_label: format!("AppKey/{}", entry.app_key),
                kind: ShadowKind::Fixable(FixAction::ClearWindowsAppKey {
                    app_key: entry.app_key,
                    qol_combo: qol_combo.clone(),
                }),
            });
        }
    }
    for (norm, reserved) in &reserved_combos {
        if let Some(qol_combo) = qol_index.get(norm) {
            shadows.push(DetectedShadow {
                qol_combo: qol_combo.clone(),
                source_label: reserved.label.to_string(),
                kind: ShadowKind::Reserved {
                    hint: reserved.hint.to_string(),
                },
            });
        }
    }
    shadows
}

pub(crate) fn parse_reg_query_output(text: &str) -> Vec<AppKeyEntry> {
    let mut entries = Vec::new();
    let mut current_key: Option<String> = None;
    let mut current_combo: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(key_path) = trimmed.strip_prefix("HKEY_CURRENT_USER\\") {
            push_entry(&mut entries, current_key.take(), current_combo.take());
            current_key = key_path
                .rsplit('\\')
                .next()
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string());
            continue;
        }
        let line = trimmed.trim_start();
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = parse_reg_value_line(line) {
            if name.eq_ignore_ascii_case("ShortcutKeys")
                || name.eq_ignore_ascii_case("ShellExecute")
            {
                current_combo = Some(value);
            }
        }
    }
    push_entry(&mut entries, current_key, current_combo);
    entries
}

fn push_entry(out: &mut Vec<AppKeyEntry>, key: Option<String>, combo: Option<String>) {
    if let Some(app_key) = key {
        out.push(AppKeyEntry { app_key, combo });
    }
}

fn parse_reg_value_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let name = parts.next()?.to_string();
    let _type = parts.next()?;
    let value: String = parts.collect::<Vec<_>>().join(" ");
    if name.is_empty() || value.is_empty() {
        return None;
    }
    Some((name, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubReader(Vec<AppKeyEntry>);

    impl RegistryReader for StubReader {
        fn read_app_keys(&mut self) -> Vec<AppKeyEntry> {
            self.0.clone()
        }
    }

    fn index(combos: &[&str]) -> BTreeMap<String, String> {
        combos
            .iter()
            .filter_map(|c| normalize_combo(c).map(|n| (n, (*c).to_string())))
            .collect()
    }

    #[test]
    fn registered_app_key_with_overlap_emits_clearable_fix() {
        let qol = index(&["Ctrl+Shift+E"]);
        let mut reader = StubReader(vec![AppKeyEntry {
            app_key: "17".to_string(),
            combo: Some("Ctrl+Shift+E".to_string()),
        }]);
        let shadows = collect_shadows_with_reader(&qol, &mut reader);
        let fixable = shadows
            .iter()
            .find(|s| matches!(s.kind, ShadowKind::Fixable(_)))
            .expect("fixable AppKey shadow");
        match &fixable.kind {
            ShadowKind::Fixable(FixAction::ClearWindowsAppKey { app_key, qol_combo }) => {
                assert_eq!(app_key, "17");
                assert_eq!(qol_combo, "Ctrl+Shift+E");
            }
            other => panic!("expected ClearWindowsAppKey, got {other:?}"),
        }
    }

    #[test]
    fn explorer_reserved_combos_emit_reserved_kind() {
        let cases = [
            ("Win+Space", "Windows input switcher"),
            ("Win+Tab", "Windows Task View"),
            ("Win+R", "Windows Run dialog"),
            ("Win+E", "Windows File Explorer"),
            ("Win+L", "Windows Lock"),
        ];
        for (combo, label) in cases {
            let qol = index(&[combo]);
            let mut reader = StubReader(Vec::new());
            let shadows = collect_shadows_with_reader(&qol, &mut reader);
            let reserved = shadows
                .iter()
                .find(|s| matches!(s.kind, ShadowKind::Reserved { .. }))
                .unwrap_or_else(|| panic!("expected reserved shadow for {combo}"));
            assert_eq!(reserved.source_label, label, "label mismatch for {combo}");
        }
    }

    #[test]
    fn no_qol_overlap_yields_no_shadows() {
        let qol = index(&["Ctrl+Alt+Shift+F12"]);
        let mut reader = StubReader(vec![AppKeyEntry {
            app_key: "17".to_string(),
            combo: Some("Ctrl+Shift+E".to_string()),
        }]);
        assert!(collect_shadows_with_reader(&qol, &mut reader).is_empty());
    }

    #[test]
    fn registry_overlap_on_reserved_combo_defers_to_reserved_only() {
        let qol = index(&["Win+E"]);
        let mut reader = StubReader(vec![AppKeyEntry {
            app_key: "17".to_string(),
            combo: Some("Win+E".to_string()),
        }]);
        let shadows = collect_shadows_with_reader(&qol, &mut reader);
        assert_eq!(
            shadows.len(),
            1,
            "registry entry on a reserved combo must not double-report; only the Reserved entry wins"
        );
        assert!(matches!(shadows[0].kind, ShadowKind::Reserved { .. }));
    }

    #[test]
    fn registry_and_reserved_on_distinct_combos_both_reported() {
        let qol = index(&["Ctrl+Shift+E", "Win+Space"]);
        let mut reader = StubReader(vec![AppKeyEntry {
            app_key: "17".to_string(),
            combo: Some("Ctrl+Shift+E".to_string()),
        }]);
        let shadows = collect_shadows_with_reader(&qol, &mut reader);
        let mut combos: Vec<&str> = shadows.iter().map(|s| s.qol_combo.as_str()).collect();
        combos.sort();
        assert_eq!(combos, ["Ctrl+Shift+E", "Win+Space"]);
    }

    #[test]
    fn parse_reg_query_extracts_subkeys_and_shortcut_value() {
        let sample = "\
HKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\AppKey
HKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\AppKey\\17
    ShortcutKeys    REG_SZ    Win+E
HKEY_CURRENT_USER\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\AppKey\\18
    ShortcutKeys    REG_SZ    Win+Q
";
        let entries = parse_reg_query_output(sample);
        let by_key: BTreeMap<String, AppKeyEntry> = entries
            .into_iter()
            .map(|e| (e.app_key.clone(), e))
            .collect();
        assert_eq!(
            by_key.get("17").and_then(|e| e.combo.as_deref()),
            Some("Win+E")
        );
        assert_eq!(
            by_key.get("18").and_then(|e| e.combo.as_deref()),
            Some("Win+Q")
        );
    }
}
