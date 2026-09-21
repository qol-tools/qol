use super::super::de_bindings::normalize_combo;
use super::super::diagnosis::FixAction;
use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use crate::hotkeys::{HotkeyBinding, HotkeyManager, RegistrationError};
use std::collections::BTreeMap;

mod platform;

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
        let mut shadows = platform::collect_shadows(&qol_index);
        shadows.extend(registration_failure_shadows(
            &qol_index,
            &crate::hotkeys::get_registration_errors(),
        ));
        let report = diagnose(shadows);
        match crate::hotkeys::takeover::restart_advice() {
            None => report,
            Some(advice) => with_restart_advice(report, advice),
        }
    }
}

fn with_restart_advice(mut report: CheckReport, advice: String) -> CheckReport {
    report.summary = if report.summary.is_empty() {
        advice.clone()
    } else {
        format!("{} | {advice}", report.summary)
    };
    report.issues.push(DoctorIssue::new(
        ID,
        Severity::Warn,
        format!("stale desktop key grab: {advice}"),
    ));
    report.advice.push(advice);
    report
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetectedShadow {
    pub qol_combos: Vec<String>,
    pub source_label: String,
    pub kind: ShadowKind,
}

impl DetectedShadow {
    fn combos_label(&self) -> String {
        self.qol_combos.join(", ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShadowKind {
    Fixable(FixAction),
    Reserved { hint: String },
    RegistrationFailure { hint: String },
}

fn diagnose(shadows: Vec<DetectedShadow>) -> CheckReport {
    if shadows.is_empty() {
        return CheckReport::ok("no DE keybinding conflicts detected".to_string());
    }
    let (fixable, others): (Vec<_>, Vec<_>) = shadows
        .into_iter()
        .partition(|shadow| matches!(shadow.kind, ShadowKind::Fixable(_)));
    let (reserved, registration_failures): (Vec<_>, Vec<_>) = others
        .into_iter()
        .partition(|shadow| matches!(shadow.kind, ShadowKind::Reserved { .. }));

    if fixable.is_empty() && reserved.is_empty() {
        return registration_failure_report(&registration_failures);
    }
    if fixable.is_empty() {
        let mut report = CheckReport::error(format_reserved_message(&reserved), ID);
        if !registration_failures.is_empty() {
            report = append_registration_failures(report, &registration_failures);
        }
        return report;
    }

    let fixes: Vec<FixAction> = fixable
        .iter()
        .filter_map(|shadow| match &shadow.kind {
            ShadowKind::Fixable(action) => Some(action.clone()),
            ShadowKind::Reserved { .. } | ShadowKind::RegistrationFailure { .. } => None,
        })
        .collect();
    let mut message = format_fixable_message(&fixable);
    if !reserved.is_empty() {
        message.push_str(" | reserved (manual remap required): ");
        message.push_str(&format_reserved_message(&reserved));
    }
    let mut report = CheckReport::warn(message, ID, fixes);
    if !registration_failures.is_empty() {
        report = append_registration_failures(report, &registration_failures);
    }
    report
}

fn registration_failure_report(shadows: &[DetectedShadow]) -> CheckReport {
    let message = format_registration_failure_message(shadows);
    let advice = format_registration_failure_advice(shadows);
    CheckReport {
        summary: message.clone(),
        issues: vec![DoctorIssue::new(ID, Severity::Warn, message)],
        advice,
        fixes: Vec::new(),
    }
}

fn append_registration_failures(
    mut report: CheckReport,
    shadows: &[DetectedShadow],
) -> CheckReport {
    if !report.summary.is_empty() {
        report.summary.push_str(" | ");
    }
    report
        .summary
        .push_str(&format_registration_failure_message(shadows));
    report.issues.push(DoctorIssue::new(
        ID,
        Severity::Warn,
        format_registration_failure_message(shadows),
    ));
    report
        .advice
        .extend(format_registration_failure_advice(shadows));
    report
}

fn format_registration_failure_message(shadows: &[DetectedShadow]) -> String {
    let mut combos: Vec<String> = shadows.iter().flat_map(|s| s.qol_combos.clone()).collect();
    combos.sort_unstable();
    combos.dedup();
    format!(
        "hotkey registration failed: {} could not be grabbed by the active backend",
        combos.join(", ")
    )
}

fn format_registration_failure_advice(shadows: &[DetectedShadow]) -> Vec<String> {
    let mut hints: Vec<String> = shadows
        .iter()
        .filter_map(|shadow| match &shadow.kind {
            ShadowKind::RegistrationFailure { hint } => Some(hint.clone()),
            _ => None,
        })
        .collect();
    hints.sort_unstable();
    hints.dedup();
    hints
}

fn registration_failure_shadows(
    qol_index: &BTreeMap<String, String>,
    errors: &[RegistrationError],
) -> Vec<DetectedShadow> {
    errors
        .iter()
        .filter_map(|error| {
            let normalized = normalize_combo(&error.key)?;
            let combo = qol_index.get(&normalized)?;
            Some(DetectedShadow {
                qol_combos: vec![combo.clone()],
                source_label: "active hotkey backend".to_string(),
                kind: ShadowKind::RegistrationFailure {
                    hint: format!(
                        "{combo} could not be registered ({error}); stop or reconfigure another application that owns it, or restart the desktop compositor if the grab is stale",
                        error = error.error
                    ),
                },
            })
        })
        .collect()
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
        for combo in &shadow.qol_combos {
            by_combo
                .entry(combo.as_str())
                .or_default()
                .push(shadow.source_label.clone());
        }
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
                    shadow.combos_label(),
                    shadow.source_label
                )
            }
            ShadowKind::Fixable(_) | ShadowKind::RegistrationFailure { .. } => {
                shadow.combos_label()
            }
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
            plugin_uid: crate::plugins::PluginUid::new("test-plugin"),
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
            qol_combos: vec!["Super+Space".into()],
            source_label: "schema.key".into(),
            kind: ShadowKind::Fixable(FixAction::UnshadowDeBinding {
                dir: "schema".into(),
                key: "key".into(),
                qol_combos: vec!["Super+Space".into()],
                orphaned: false,
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
            qol_combos: vec!["Cmd+Tab".into()],
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
    fn diagnose_returns_warn_with_advice_when_only_registration_failures_remain() {
        let shadows = vec![DetectedShadow {
            qol_combos: vec!["Shift+Super+S".into()],
            source_label: "active hotkey backend".into(),
            kind: ShadowKind::RegistrationFailure {
                hint: "stop the application that owns the key".into(),
            },
        }];
        let report = diagnose(shadows);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert!(
            report.fixes.is_empty(),
            "a registration failure has no dconf fix"
        );
        assert!(
            report.summary.contains("Shift+Super+S")
                && report.advice == ["stop the application that owns the key"],
            "report must name the combo and preserve the backend advice: {:?}",
            report.summary
        );
    }

    #[test]
    fn registration_failures_include_only_enabled_configured_hotkeys() {
        let index = build_qol_index(&[binding("Shift+Super+S"), binding("Ctrl+Alt+T")]);
        let errors = vec![
            RegistrationError {
                key: "<Shift><Super>s".into(),
                error: "already registered".into(),
            },
            RegistrationError {
                key: "Super+Unconfigured".into(),
                error: "unsupported".into(),
            },
        ];

        let shadows = registration_failure_shadows(&index, &errors);

        assert_eq!(shadows.len(), 1, "only configured failures are actionable");
        assert_eq!(shadows[0].qol_combos, vec!["Shift+Super+S"]);
        assert!(matches!(
            shadows[0].kind,
            ShadowKind::RegistrationFailure { .. }
        ));
    }

    #[test]
    fn diagnose_mixes_fixable_and_reserved_into_warn_with_hint() {
        let shadows = vec![
            DetectedShadow {
                qol_combos: vec!["Super+Space".into()],
                source_label: "schema.key".into(),
                kind: ShadowKind::Fixable(FixAction::UnshadowDeBinding {
                    dir: "schema".into(),
                    key: "key".into(),
                    qol_combos: vec!["Super+Space".into()],
                    orphaned: false,
                }),
            },
            DetectedShadow {
                qol_combos: vec!["Cmd+Tab".into()],
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
