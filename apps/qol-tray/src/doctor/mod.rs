mod checks;
mod cli;
mod de_bindings;
mod diagnosis;
mod framework;
mod install_id;
pub(crate) mod platform;
pub mod report;
pub mod trigger;

use anyhow::Result;
use diagnosis::{apply_fix, FixAction, FixApplicability};
use framework::{run_check, DoctorCheckResult, DoctorContext, Selector};
pub use report::{FixReport, Outcome, OutcomeStatus, Report};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixPolicy {
    max_applicability: FixApplicability,
}

impl FixPolicy {
    pub fn safe() -> Self {
        Self::default()
    }

    pub fn startup() -> Self {
        Self {
            max_applicability: FixApplicability::ReversibleHostMutation,
        }
    }

    fn allows(&self, action: &FixAction) -> bool {
        action.applicability() <= self.max_applicability
    }
}

pub fn check() -> Report {
    let ctx = DoctorContext::new();
    let results = run_selected(&Selector::All, &ctx);
    report::report(results)
}

pub fn fix_safe() -> FixReport {
    fix_with_policy(FixPolicy::safe())
}

pub fn check_single(id: &str) -> Report {
    let ctx = DoctorContext::new();
    let results = run_selected(&Selector::Id(id.to_string()), &ctx);
    report::report(results)
}

pub fn fix_single(id: &str) -> FixReport {
    fix_with_selector_and_policy(Selector::Id(id.to_string()), FixPolicy::safe())
}

pub fn fix_single_with_policy(id: &str, policy: FixPolicy) -> FixReport {
    fix_with_selector_and_policy(Selector::Id(id.to_string()), policy)
}

pub fn fix_with_policy(policy: FixPolicy) -> FixReport {
    fix_with_selector_and_policy(Selector::All, policy)
}

pub fn auto_fix_startup() -> FixReport {
    if let Some(trigger) = trigger::take() {
        run_triggered_check(&trigger);
    }
    let report = fix_with_policy(FixPolicy::startup());
    log_fix_attempts(&report);
    log_fix_failures(&report);
    log_remaining_outcomes(&report.after);
    report
}

fn run_triggered_check(trigger: &trigger::Trigger) {
    if !is_known_check_id(&trigger.check_id) {
        log::warn!(
            "doctor: triggered run skipped, unknown check_id={} (reason={})",
            trigger.check_id,
            trigger.reason
        );
        return;
    };
    log::info!(
        "doctor: triggered run for {} (reason={})",
        trigger.check_id,
        trigger.reason
    );
    let report =
        fix_with_selector_and_policy(Selector::Id(trigger.check_id.clone()), FixPolicy::startup());
    log_fix_attempts(&report);
    log_fix_failures(&report);
}

fn is_known_check_id(id: &str) -> bool {
    checks::registry().iter().any(|check| check.meta().id == id)
}

fn fix_with_selector_and_policy(selector: Selector, policy: FixPolicy) -> FixReport {
    let ctx = DoctorContext::new();
    let before_results = run_selected(&selector, &ctx);
    let before = report::report(before_results.clone());
    let summary = apply_fixes(before_results, policy);
    let after_ctx = DoctorContext::new();
    let after_results = run_selected(&selector, &after_ctx);
    let after = report::report(after_results);
    FixReport {
        before,
        after,
        attempted: summary.attempted,
        applied: summary.applied,
        skipped: summary.skipped,
        failures: summary.failures,
    }
}

fn run_selected(selector: &Selector, ctx: &DoctorContext) -> Vec<DoctorCheckResult> {
    checks::registry()
        .iter()
        .filter(|check| {
            let meta = check.meta();
            selector.matches(&meta)
                && meta.platform.matches_current()
                && check_enabled_for_build(meta.dev_only)
        })
        .map(|check| run_check(check.as_ref(), ctx))
        .collect()
}

#[cfg(feature = "dev")]
fn check_enabled_for_build(_dev_only: bool) -> bool {
    true
}

#[cfg(not(feature = "dev"))]
fn check_enabled_for_build(dev_only: bool) -> bool {
    !dev_only
}

pub fn run_cli_from_env() -> Result<i32> {
    cli::run_cli_from_env()
}

fn apply_fixes(results: Vec<DoctorCheckResult>, policy: FixPolicy) -> FixSummary {
    let mut summary = FixSummary::default();
    for result in results {
        apply_result_fixes(&mut summary, result, policy);
    }
    summary
}

fn apply_result_fixes(summary: &mut FixSummary, result: DoctorCheckResult, policy: FixPolicy) {
    for action in result.fixes {
        if !policy.allows(&action) {
            summary.skipped += 1;
            continue;
        }
        summary.attempted += 1;
        if let Err(error) = apply_fix(&action) {
            summary
                .failures
                .push(format!("{}: {}", result.outcome.id, error));
            continue;
        }
        summary.applied += 1;
        log_applied(&action);
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

fn log_applied(action: &FixAction) {
    match action {
        FixAction::UnshadowDeBinding {
            schema,
            key,
            qol_combo,
        } => {
            log::info!(
                "doctor: removed {} from {}.{} (qol-tray's hotkey takes priority)",
                qol_combo,
                schema,
                key
            );
        }
        FixAction::DisableSymbolicHotkey {
            hotkey_id,
            qol_combo,
        } => {
            log::info!(
                "doctor: disabled macOS symbolic hotkey id={} ({} now reaches qol-tray)",
                hotkey_id,
                qol_combo
            );
        }
        FixAction::ClearWindowsAppKey { app_key, qol_combo } => {
            log::info!(
                "doctor: cleared Windows AppKey {} ({} now reaches qol-tray)",
                app_key,
                qol_combo
            );
        }
        FixAction::SetActiveInstallId(_)
        | FixAction::WriteInstallMarker { .. }
        | FixAction::WriteAutostartEntry { .. }
        | FixAction::EnsurePluginsDir { .. }
        | FixAction::KillPluginProcessLeaks { .. }
        | FixAction::InstallShellHook => {}
        #[cfg(feature = "dev")]
        FixAction::RelocateDevLink { plugin_id, to } => {
            log::info!(
                "doctor: relocated dev-link for {} to {}",
                plugin_id,
                to.display()
            );
        }
        #[cfg(feature = "dev")]
        FixAction::PruneOrphanFingerprints { ids } => {
            log::info!(
                "doctor: pruned {} orphan build fingerprint(s): {}",
                ids.len(),
                ids.join(", ")
            );
        }
        #[cfg(feature = "dev")]
        FixAction::PruneReservedPlugins { ids } => {
            log::info!(
                "doctor: pruned {} reserved plugin id(s) from registry: {}",
                ids.len(),
                ids.join(", ")
            );
        }
    }
}

fn log_remaining_outcomes(report: &Report) {
    for outcome in report.outcomes() {
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
    use crate::doctor::diagnosis::FixAction;
    use crate::doctor::framework::{
        CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorCheckResult, DoctorContext,
        Severity,
    };

    fn unshadow_result() -> DoctorCheckResult {
        struct StubUnshadowCheck;
        impl DoctorCheck for StubUnshadowCheck {
            fn meta(&self) -> CheckMeta {
                CheckMeta::new(
                    "hotkey_shadows",
                    "Hotkey Shadows",
                    CheckCategory::HostSurface,
                )
            }
            fn run(&self, _: &DoctorContext) -> CheckReport {
                CheckReport::warn(
                    "test shadow".to_string(),
                    "hotkey_shadows",
                    vec![FixAction::UnshadowDeBinding {
                        schema: "org.cinnamon.desktop.keybindings.wm".into(),
                        key: "switch-input-source".into(),
                        qol_combo: "Super+Space".into(),
                    }],
                )
            }
        }
        run_check(&StubUnshadowCheck, &DoctorContext::new())
    }

    #[test]
    fn safe_policy_skips_host_mutations_without_invoking_them() {
        let summary = apply_fixes(vec![unshadow_result()], FixPolicy::safe());
        assert_eq!(
            summary.attempted, 0,
            "FixPolicy::safe (CLI default `fix` without --apply-host-fixes) must not attempt host mutations"
        );
        assert_eq!(summary.applied, 0);
        assert_eq!(summary.skipped, 1);
        assert!(summary.failures.is_empty(), "got: {:?}", summary.failures);
    }

    #[test]
    fn startup_policy_attempts_host_mutations() {
        let summary = apply_fixes(vec![unshadow_result()], FixPolicy::startup());
        assert_eq!(
            summary.attempted, 1,
            "auto_fix_startup uses FixPolicy::startup and must attempt the unshadow"
        );
        assert_eq!(
            summary.skipped, 0,
            "startup policy must not skip UnshadowDeBinding"
        );
        assert_eq!(
            summary.applied + summary.failures.len(),
            1,
            "exactly one outcome (apply or fail) must be recorded; got applied={}, failures={:?}",
            summary.applied,
            summary.failures
        );
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
                FixPolicy::startup(),
                true,
            ),
            (
                FixAction::DisableSymbolicHotkey {
                    hotkey_id: 64,
                    qol_combo: "Cmd+Space".into(),
                },
                FixPolicy::safe(),
                false,
            ),
            (
                FixAction::DisableSymbolicHotkey {
                    hotkey_id: 64,
                    qol_combo: "Cmd+Space".into(),
                },
                FixPolicy::startup(),
                true,
            ),
            (
                FixAction::ClearWindowsAppKey {
                    app_key: "17".into(),
                    qol_combo: "Win+E".into(),
                },
                FixPolicy::safe(),
                false,
            ),
            (
                FixAction::ClearWindowsAppKey {
                    app_key: "17".into(),
                    qol_combo: "Win+E".into(),
                },
                FixPolicy::startup(),
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

    #[test]
    fn registry_check_ids_are_unique() {
        let ids: Vec<&str> = checks::registry()
            .iter()
            .map(|check| check.meta().id)
            .collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "duplicate check ids: {ids:?}");
    }

    #[test]
    fn registry_includes_dev_loop_group_members() {
        let dev_loop_ids: Vec<&str> = checks::registry()
            .iter()
            .filter(|check| check.meta().groups.contains(&"dev-loop"))
            .map(|check| check.meta().id)
            .collect();
        assert!(
            dev_loop_ids.contains(&"plugin_process_leaks"),
            "plugin_process_leaks must be in dev-loop group, got {dev_loop_ids:?}"
        );
        #[cfg(feature = "dev")]
        {
            assert!(
                dev_loop_ids.contains(&"plugin_staleness"),
                "plugin_staleness must be in dev-loop group, got {dev_loop_ids:?}"
            );
            assert!(
                dev_loop_ids.contains(&"dev_link_paths"),
                "dev_link_paths must be in dev-loop group, got {dev_loop_ids:?}"
            );
        }
    }

    #[test]
    fn is_known_check_id_matches_registry_only() {
        assert!(is_known_check_id("plugin_process_leaks"));
        assert!(!is_known_check_id("nonexistent_check_xyz"));
    }

    #[test]
    fn crash_status_rolls_up_in_report() {
        struct PanicCheck;
        impl DoctorCheck for PanicCheck {
            fn meta(&self) -> CheckMeta {
                CheckMeta::new("panic_test", "Panic", CheckCategory::Runtime)
            }
            fn run(&self, _: &DoctorContext) -> CheckReport {
                panic!("simulated");
            }
        }
        let ctx = DoctorContext::new();
        let result = run_check(&PanicCheck, &ctx);
        assert_eq!(result.outcome.status, OutcomeStatus::Crash);
        assert!(result.issues.iter().any(|i| i.severity == Severity::Crash));
    }
}
