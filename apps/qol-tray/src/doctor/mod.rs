mod checks;
mod cli;
mod de_bindings;
mod diagnosis;
mod install_id;
mod platform;
mod report;

use anyhow::Result;
use diagnosis::{apply_fix, Diagnosis, FixAction};
pub use report::{FixReport, Outcome, OutcomeStatus, Report};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckId {
    PluginProcessLeaks,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixPolicy {
    pub apply_de_fixes: bool,
}

impl FixPolicy {
    pub fn safe() -> Self {
        Self::default()
    }

    pub fn with_de_fixes() -> Self {
        Self {
            apply_de_fixes: true,
        }
    }

    fn allows(&self, action: &FixAction) -> bool {
        if action.is_safe_to_auto_apply() {
            return true;
        }
        self.apply_de_fixes
    }
}

pub fn check() -> Report {
    let diagnoses = checks::collect_diagnoses();
    report::report(
        diagnoses
            .into_iter()
            .map(|diagnosis| diagnosis.outcome)
            .collect(),
    )
}

pub fn fix_safe() -> FixReport {
    fix_with_policy(FixPolicy::safe())
}

pub fn check_single(id: CheckId) -> Report {
    let diagnosis = checks::collect_diagnosis(id);
    report::report(vec![diagnosis.outcome])
}

pub fn fix_single(id: CheckId) -> FixReport {
    let diagnoses = vec![checks::collect_diagnosis(id)];
    let before = report_from_diagnoses(&diagnoses);
    let summary = apply_fixes(diagnoses, FixPolicy::safe());
    let after = check_single(id);
    FixReport {
        before,
        after,
        attempted: summary.attempted,
        applied: summary.applied,
        skipped: summary.skipped,
        failures: summary.failures,
    }
}

pub fn fix_with_policy(policy: FixPolicy) -> FixReport {
    let diagnoses = checks::collect_diagnoses();
    let before = report_from_diagnoses(&diagnoses);
    let summary = apply_fixes(diagnoses, policy);
    let after = check();
    FixReport {
        before,
        after,
        attempted: summary.attempted,
        applied: summary.applied,
        skipped: summary.skipped,
        failures: summary.failures,
    }
}

pub fn auto_fix_startup() -> FixReport {
    let report = fix_with_policy(FixPolicy::safe());
    log_fix_attempts(&report);
    log_fix_failures(&report);
    log_remaining_outcomes(&report.after);
    report
}

pub fn run_cli_from_env() -> Result<i32> {
    cli::run_cli_from_env()
}

fn report_from_diagnoses(diagnoses: &[Diagnosis]) -> Report {
    report::report(
        diagnoses
            .iter()
            .map(|diagnosis| diagnosis.outcome.clone())
            .collect(),
    )
}

fn apply_fixes(diagnoses: Vec<Diagnosis>, policy: FixPolicy) -> FixSummary {
    let mut summary = FixSummary::default();
    for diagnosis in diagnoses {
        apply_diagnosis_fixes(&mut summary, diagnosis, policy);
    }
    summary
}

fn apply_diagnosis_fixes(summary: &mut FixSummary, diagnosis: Diagnosis, policy: FixPolicy) {
    for action in diagnosis.fixes {
        if !policy.allows(&action) {
            summary.skipped += 1;
            continue;
        }
        summary.attempted += 1;
        if let Err(error) = apply_fix(&action) {
            summary
                .failures
                .push(format!("{}: {}", diagnosis.outcome.id, error));
            continue;
        }
        summary.applied += 1;
    }
}

fn log_fix_attempts(report: &FixReport) {
    if report.attempted == 0 {
        return;
    }
    log::info!(
        "doctor startup fixes attempted={}, applied={}",
        report.attempted,
        report.applied
    );
}

fn log_fix_failures(report: &FixReport) {
    for failure in &report.failures {
        log::warn!("doctor startup fix failed: {}", failure);
    }
}

fn log_remaining_outcomes(report: &Report) {
    for outcome in &report.outcomes {
        if matches!(outcome.status, OutcomeStatus::Ok) {
            continue;
        }
        log::warn!("doctor {}: {}", outcome.id, outcome.message);
    }
}

#[derive(Default)]
struct FixSummary {
    attempted: usize,
    applied: usize,
    skipped: usize,
    failures: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::diagnosis::{warn_outcome_with_fixes, FixAction};

    fn unshadow_diagnosis() -> Diagnosis {
        warn_outcome_with_fixes(
            "hotkey_shadows",
            "test shadow".into(),
            vec![FixAction::UnshadowDeBinding {
                schema: "org.cinnamon.desktop.keybindings.wm".into(),
                key: "switch-input-source".into(),
                qol_combo: "Super+Space".into(),
            }],
        )
    }

    #[test]
    fn safe_policy_skips_de_fixes_without_invoking_them() {
        let summary = apply_fixes(vec![unshadow_diagnosis()], FixPolicy::safe());
        assert_eq!(
            summary.attempted, 0,
            "auto-startup must not attempt DE fixes"
        );
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.skipped, 1);
        assert!(summary.failures.is_empty(), "got: {:?}", summary.failures);
    }

    #[test]
    fn fix_policy_allows_or_skips_per_variant() {
        let cases = [
            (
                FixAction::SetActiveInstallId("abc".into()),
                FixPolicy::safe(),
                true,
            ),
            (
                FixAction::EnsurePluginsDir {
                    path: std::path::PathBuf::from("/tmp/never-used"),
                },
                FixPolicy::safe(),
                true,
            ),
            (
                FixAction::WriteAutostartEntry {
                    binary_path: std::path::PathBuf::from("/usr/bin/qol-tray"),
                },
                FixPolicy::safe(),
                true,
            ),
            (
                FixAction::UnshadowDeBinding {
                    schema: "x".into(),
                    key: "y".into(),
                    qol_combo: "Super+Space".into(),
                },
                FixPolicy::safe(),
                false,
            ),
            (
                FixAction::UnshadowDeBinding {
                    schema: "x".into(),
                    key: "y".into(),
                    qol_combo: "Super+Space".into(),
                },
                FixPolicy::with_de_fixes(),
                true,
            ),
        ];
        for (action, policy, expected) in cases {
            assert_eq!(
                policy.allows(&action),
                expected,
                "policy={policy:?}, variant discriminant={:?}",
                std::mem::discriminant(&action)
            );
        }
    }
}
