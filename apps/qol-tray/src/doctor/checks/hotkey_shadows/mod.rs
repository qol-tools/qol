use super::super::de_bindings::normalize_combo;
use super::super::diagnosis::FixAction;
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::hotkeys::{HotkeyBinding, HotkeyManager};
use std::collections::BTreeMap;

mod linux;
mod macos;
mod windows;

#[cfg(target_os = "linux")]
use linux as platform_impl;
#[cfg(target_os = "macos")]
use macos as platform_impl;
#[cfg(target_os = "windows")]
use windows as platform_impl;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as platform_impl;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback {
    use super::DetectedShadow;
    use std::collections::BTreeMap;

    pub(super) fn collect_shadows(_qol_index: &BTreeMap<String, String>) -> Vec<DetectedShadow> {
        Vec::new()
    }
}

const ID: &str = "hotkey_shadows";

pub(super) struct HotkeyShadowsCheck;

impl DoctorCheck for HotkeyShadowsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Hotkey shadows", CheckCategory::HostSurface)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        let bindings = match enabled_bindings() {
            Ok(bindings) => bindings,
            Err(error) => {
                return CheckReport::ok(format!("could not load hotkey config: {error}"));
            }
        };
        if bindings.is_empty() {
            return CheckReport::ok("no hotkeys configured".to_string());
        }

        let qol_index = build_qol_index(&bindings);
        let shadows = platform_impl::collect_shadows(&qol_index);
        diagnose(shadows)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetectedShadow {
    pub qol_combo: String,
    pub source_label: String,
    pub kind: ShadowKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShadowKind {
    Fixable(FixAction),
    Reserved { hint: String },
}

fn diagnose(shadows: Vec<DetectedShadow>) -> CheckReport {
    if shadows.is_empty() {
        return CheckReport::ok("no DE keybinding conflicts detected".to_string());
    }
    let (fixable, reserved): (Vec<_>, Vec<_>) = shadows
        .into_iter()
        .partition(|shadow| matches!(shadow.kind, ShadowKind::Fixable(_)));

    if fixable.is_empty() {
        return CheckReport::error(format_reserved_message(&reserved), ID);
    }

    let fixes: Vec<FixAction> = fixable
        .iter()
        .filter_map(|shadow| match &shadow.kind {
            ShadowKind::Fixable(action) => Some(action.clone()),
            ShadowKind::Reserved { .. } => None,
        })
        .collect();
    let mut message = format_fixable_message(&fixable);
    if !reserved.is_empty() {
        message.push_str(" | reserved (manual remap required): ");
        message.push_str(&format_reserved_message(&reserved));
    }
    CheckReport::warn(message, ID, fixes)
}

pub(crate) fn build_qol_index(bindings: &[HotkeyBinding]) -> BTreeMap<String, String> {
    bindings
        .iter()
        .filter(|b| b.enabled)
        .filter_map(|b| normalize_combo(&b.key).map(|n| (n, b.key.clone())))
        .collect()
}

fn enabled_bindings() -> anyhow::Result<Vec<HotkeyBinding>> {
    let manager = HotkeyManager::new()?;
    let config = manager.load_config()?;
    Ok(config.hotkeys.into_iter().filter(|h| h.enabled).collect())
}

fn format_fixable_message(shadows: &[DetectedShadow]) -> String {
    let mut by_combo: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for shadow in shadows {
        by_combo
            .entry(shadow.qol_combo.as_str())
            .or_default()
            .push(shadow.source_label.clone());
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

fn format_reserved_message(shadows: &[DetectedShadow]) -> String {
    let parts: Vec<String> = shadows
        .iter()
        .map(|shadow| match &shadow.kind {
            ShadowKind::Reserved { hint } => {
                format!(
                    "{} owned by {} ({hint})",
                    shadow.qol_combo, shadow.source_label
                )
            }
            ShadowKind::Fixable(_) => shadow.qol_combo.clone(),
        })
        .collect();
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::super::super::framework::Severity;
    use super::*;

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
    fn diagnose_returns_ok_when_no_shadows() {
        let report = diagnose(Vec::new());
        assert!(
            report.issues.is_empty(),
            "no shadows must produce no issues"
        );
    }

    #[test]
    fn diagnose_returns_warn_with_fix_when_fixable_only() {
        let shadows = vec![DetectedShadow {
            qol_combo: "Super+Space".into(),
            source_label: "schema.key".into(),
            kind: ShadowKind::Fixable(FixAction::UnshadowDeBinding {
                schema: "schema".into(),
                key: "key".into(),
                qol_combo: "Super+Space".into(),
            }),
        }];
        let report = diagnose(shadows);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert_eq!(report.fixes.len(), 1);
    }

    #[test]
    fn diagnose_returns_error_when_only_reserved_combos_clash() {
        let shadows = vec![DetectedShadow {
            qol_combo: "Cmd+Tab".into(),
            source_label: "macOS App Switcher".into(),
            kind: ShadowKind::Reserved {
                hint: "remap qol-tray's plugin to a different combo".into(),
            },
        }];
        let report = diagnose(shadows);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Error);
        assert!(report.fixes.is_empty());
        assert!(report.summary.contains("Cmd+Tab"));
        assert!(report.summary.contains("App Switcher"));
    }

    #[test]
    fn diagnose_mixes_fixable_and_reserved_into_warn_with_hint() {
        let shadows = vec![
            DetectedShadow {
                qol_combo: "Super+Space".into(),
                source_label: "schema.key".into(),
                kind: ShadowKind::Fixable(FixAction::UnshadowDeBinding {
                    schema: "schema".into(),
                    key: "key".into(),
                    qol_combo: "Super+Space".into(),
                }),
            },
            DetectedShadow {
                qol_combo: "Cmd+Tab".into(),
                source_label: "macOS App Switcher".into(),
                kind: ShadowKind::Reserved {
                    hint: "manual remap required".into(),
                },
            },
        ];
        let report = diagnose(shadows);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert_eq!(report.fixes.len(), 1, "only fixable shadow yields a fix");
        assert!(
            report.summary.contains("reserved"),
            "summary must call out reserved combos: {}",
            report.summary
        );
    }

    #[test]
    fn build_qol_index_drops_disabled_bindings() {
        let mut disabled = binding("Super+Space");
        disabled.enabled = false;
        let bindings = vec![disabled, binding("Alt+Tab")];
        let index = build_qol_index(&bindings);
        assert_eq!(index.len(), 1);
        assert!(index.values().any(|v| v == "Alt+Tab"));
    }
}
