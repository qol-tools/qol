use super::super::framework::{
    CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext, DoctorIssue, Severity,
};
use crate::features::host_defaults::inspect::{inspect, CurrentValue, InspectedClaim, Inspection};
use qol_headless::DoctorCheckResult;

const ID: &str = "host_defaults";
const MIMEAPPS_STORE: &str = "mimeapps";
const FOLDER_OPEN: &str = "folder.open";

pub(super) struct HostDefaultsCheck;

impl DoctorCheck for HostDefaultsCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Host defaults", CheckCategory::HostSurface)
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        evaluate(&inspect())
    }
}

struct Finding {
    message: String,
    hint: String,
}

pub(in crate::doctor) fn headless_check() -> qol_headless::DoctorCheck {
    qol_headless::DoctorCheck::new(ID, "Host defaults match what qol claimed", || {
        Ok(headless_result(
            HostDefaultsCheck.run(&DoctorContext::new()),
        ))
    })
}

fn headless_result(report: CheckReport) -> DoctorCheckResult {
    if report.issues.is_empty() {
        return DoctorCheckResult::ok(ID, report.summary);
    }
    DoctorCheckResult::warn(ID, report.summary).with_fix(report.advice.join("; "))
}

fn summary_of(findings: &[Finding]) -> String {
    let described: Vec<String> = findings
        .iter()
        .map(|finding| format!("{} (fix: {})", finding.message, finding.hint))
        .collect();
    match described.as_slice() {
        [only] => only.clone(),
        many => format!("{} host default problems: {}", many.len(), many.join("; ")),
    }
}

fn release_hint(role: &str) -> String {
    format!("qol-tray defaults release {role}")
}

fn evaluate(inspection: &Inspection) -> CheckReport {
    if let Some(error) = &inspection.ledger_error {
        return report_of(vec![Finding {
            message: format!("the host defaults ledger could not be read: {error}"),
            hint: "inspect the claims with `qol-tray defaults list`".to_string(),
        }]);
    }
    let findings: Vec<Finding> = unknown_findings(inspection)
        .chain(gone_plugin_findings(inspection))
        .chain(intent_findings(inspection))
        .chain(changed_value_findings(inspection))
        .chain(opener_findings(inspection))
        .chain(dead_key_findings(inspection))
        .collect();
    if findings.is_empty() {
        return CheckReport::ok(ok_summary(&inspection.claims));
    }
    report_of(findings)
}

fn ok_summary(claims: &[InspectedClaim]) -> String {
    if claims.is_empty() {
        return "no host defaults claimed".to_string();
    }
    let mut roles: Vec<&str> = claims.iter().map(|claim| claim.role.as_str()).collect();
    roles.sort_unstable();
    roles.dedup();
    format!(
        "{} claim(s) across {} role(s) match the host",
        claims.len(),
        roles.len()
    )
}

fn report_of(findings: Vec<Finding>) -> CheckReport {
    let summary = summary_of(&findings);
    let issues = findings
        .iter()
        .map(|finding| DoctorIssue::new(ID, Severity::Warn, finding.message.clone()))
        .collect();
    let mut advice: Vec<String> = findings.into_iter().map(|finding| finding.hint).collect();
    advice.sort_unstable();
    advice.dedup();
    CheckReport {
        summary,
        issues,
        advice,
        fixes: Vec::new(),
    }
}

fn unknown_findings(inspection: &Inspection) -> impl Iterator<Item = Finding> + '_ {
    inspection.unknown_probes.iter().map(|probe| Finding {
        message: format!(
            "{} could not be checked on this computer ({}); declared by {}",
            probe.role,
            probe.reason,
            probe.plugins.join(", ")
        ),
        hint: format!("qol-tray defaults probe {}", probe.role),
    })
}

fn gone_plugin_findings(inspection: &Inspection) -> impl Iterator<Item = Finding> + '_ {
    inspection
        .claims
        .iter()
        .filter(|claim| !inspection.installed.contains(&claim.plugin_id))
        .map(|claim| Finding {
            message: format!(
                "{} is held by {}, which is no longer installed",
                claim.role, claim.plugin_id
            ),
            hint: release_hint(&claim.role),
        })
}

fn intent_findings(inspection: &Inspection) -> impl Iterator<Item = Finding> + '_ {
    let unreadable = inspection.intent_error.iter().map(|error| Finding {
        message: format!("your chosen host defaults could not be read: {error}"),
        hint: "fix or delete default-roles.json in the active profile's core folder".to_string(),
    });
    let gone = inspection
        .intents
        .iter()
        .filter(|intent| !inspection.installed.contains(&intent.plugin_id))
        .map(|intent| Finding {
            message: format!(
                "{} is switched on for {}, which is no longer installed, so it was not taken at start",
                intent.role, intent.plugin_id
            ),
            hint: release_hint(&intent.role),
        });
    unreadable.chain(gone)
}

fn changed_value_findings(inspection: &Inspection) -> impl Iterator<Item = Finding> + '_ {
    inspection
        .claims
        .iter()
        .filter(|claim| claim.store == MIMEAPPS_STORE)
        .filter_map(|claim| {
            let now = match &claim.current {
                CurrentValue::Value(value) if *value == claim.applied => return None,
                CurrentValue::Value(value) => format!("now {value}"),
                CurrentValue::Absent => "now unset".to_string(),
                CurrentValue::Unread(error) => {
                    return Some(Finding {
                        message: format!(
                            "{} {} could not be read back: {error}",
                            claim.role, claim.key
                        ),
                        hint: release_hint(&claim.role),
                    })
                }
                CurrentValue::NotRead => return None,
            };
            Some(Finding {
                message: format!(
                    "{} {} {now}; qol wrote {} and the value was changed since",
                    claim.role, claim.key, claim.applied
                ),
                hint: release_hint(&claim.role),
            })
        })
}

fn opener_findings(inspection: &Inspection) -> impl Iterator<Item = Finding> + '_ {
    inspection.opener_missing.then(|| Finding {
        message: "qol-open.desktop is missing while a folder.open claim exists, so the system cannot reach qol"
            .to_string(),
        hint: release_hint(FOLDER_OPEN),
    })
    .into_iter()
}

fn dead_key_findings(inspection: &Inspection) -> impl Iterator<Item = Finding> + '_ {
    inspection.dead_keys.iter().map(|dead| Finding {
        message: format!(
            "{} does nothing: {} no longer has it and no qol hotkey is on it ({})",
            dead.key, dead.owner, dead.role
        ),
        hint: release_hint(&dead.role),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::host_defaults::inspect::{IntendedRole, UnknownProbe};

    fn claim(role: &str, plugin: &str, store: &str, current: CurrentValue) -> InspectedClaim {
        InspectedClaim {
            role: role.to_string(),
            plugin_id: plugin.to_string(),
            store: store.to_string(),
            key: "inode/directory".to_string(),
            applied: "qol-open.desktop".to_string(),
            current,
        }
    }

    fn installed(ids: &[&str]) -> std::collections::BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn matching() -> Inspection {
        Inspection {
            installed: installed(&["qol-explorer"]),
            claims: vec![claim(
                FOLDER_OPEN,
                "qol-explorer",
                MIMEAPPS_STORE,
                CurrentValue::Value("qol-open.desktop".to_string()),
            )],
            ..Inspection::default()
        }
    }

    #[test]
    fn a_dead_route_key_warns_with_the_release_command() {
        let inspection = Inspection {
            dead_keys: vec![crate::features::host_defaults::inspect::DeadKey {
                role: "slot.window_switcher".to_string(),
                key: "Alt+Tab".to_string(),
                owner: "Cinnamon".to_string(),
            }],
            ..Inspection::default()
        };
        let report = evaluate(&inspection);
        assert_eq!(
            report.issues[0].message,
            "Alt+Tab does nothing: Cinnamon no longer has it and no qol hotkey is on it (slot.window_switcher)"
        );
        assert_eq!(
            report.advice,
            ["qol-tray defaults release slot.window_switcher"]
        );
    }

    #[test]
    fn matching_claims_are_ok_with_a_summary() {
        let report = evaluate(&matching());
        assert!(report.issues.is_empty());
        assert_eq!(report.summary, "1 claim(s) across 1 role(s) match the host");
    }

    #[test]
    fn no_claims_is_ok_and_says_so() {
        let report = evaluate(&Inspection::default());
        assert!(report.issues.is_empty());
        assert_eq!(report.summary, "no host defaults claimed");
    }

    #[test]
    fn an_unknown_probe_on_a_declared_role_warns_with_a_probe_hint() {
        let inspection = Inspection {
            unknown_probes: vec![UnknownProbe {
                role: "slot.screenshot".to_string(),
                plugins: vec!["qol-shot".to_string()],
                reason: "dconf missing".to_string(),
            }],
            ..Inspection::default()
        };
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert!(report.issues[0].message.contains("dconf missing"));
        assert_eq!(report.advice, ["qol-tray defaults probe slot.screenshot"]);
    }

    #[test]
    fn a_claim_whose_plugin_is_gone_warns_with_the_release_command() {
        let mut inspection = matching();
        inspection.installed.clear();
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].message.contains("no longer installed"));
        assert_eq!(report.advice, ["qol-tray defaults release folder.open"]);
    }

    fn intended(role: &str, plugin: &str) -> IntendedRole {
        IntendedRole {
            role: role.to_string(),
            plugin_id: plugin.to_string(),
        }
    }

    #[test]
    fn a_choice_for_an_installed_plugin_is_ok() {
        let mut inspection = matching();
        inspection.intents = vec![intended(FOLDER_OPEN, "qol-explorer")];
        assert!(evaluate(&inspection).issues.is_empty());
    }

    #[test]
    fn a_choice_for_a_plugin_that_is_gone_warns_with_the_release_command() {
        let inspection = Inspection {
            intents: vec![intended("slot.screenshot", "qol-shot")],
            ..Inspection::default()
        };
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].severity, Severity::Warn);
        assert!(report.issues[0].message.contains("qol-shot"));
        assert!(report.issues[0].message.contains("not taken at start"));
        assert_eq!(report.advice, ["qol-tray defaults release slot.screenshot"]);
    }

    #[test]
    fn unreadable_choices_warn_with_the_error() {
        let inspection = Inspection {
            intent_error: Some("bad json".to_string()),
            ..Inspection::default()
        };
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].message.contains("bad json"));
    }

    #[test]
    fn a_changed_mimeapps_value_warns_and_names_the_new_value() {
        let mut inspection = matching();
        inspection.claims[0].current = CurrentValue::Value("nemo.desktop".to_string());
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].message.contains("now nemo.desktop"));
    }

    #[test]
    fn an_unset_mimeapps_value_warns() {
        let mut inspection = matching();
        inspection.claims[0].current = CurrentValue::Absent;
        let report = evaluate(&inspection);
        assert!(report.issues[0].message.contains("now unset"));
    }

    #[test]
    fn an_unreadable_mimeapps_value_warns_with_the_error() {
        let mut inspection = matching();
        inspection.claims[0].current = CurrentValue::Unread("permission denied".to_string());
        let report = evaluate(&inspection);
        assert!(report.issues[0].message.contains("permission denied"));
    }

    #[test]
    fn stores_other_than_mimeapps_are_not_compared() {
        let mut inspection = matching();
        inspection.claims[0].store = "desktop_entry".to_string();
        inspection.claims[0].current = CurrentValue::Value("other".to_string());
        assert!(evaluate(&inspection).issues.is_empty());
    }

    #[test]
    fn a_missing_opener_warns_with_the_folder_open_release_command() {
        let mut inspection = matching();
        inspection.opener_missing = true;
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].message.contains("qol-open.desktop"));
        assert_eq!(report.advice, ["qol-tray defaults release folder.open"]);
    }

    #[test]
    fn an_unreadable_ledger_warns_instead_of_claiming_health() {
        let inspection = Inspection {
            ledger_error: Some("bad json".to_string()),
            ..Inspection::default()
        };
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].message.contains("bad json"));
    }

    #[test]
    fn the_summary_carries_the_fix_hint_for_the_human_output() {
        let mut inspection = matching();
        inspection.opener_missing = true;
        let report = evaluate(&inspection);
        assert!(
            report
                .summary
                .ends_with("(fix: qol-tray defaults release folder.open)"),
            "{}",
            report.summary
        );
        assert!(!report.summary.contains("problem(s)"));
    }

    #[test]
    fn several_findings_are_counted_and_each_keeps_its_hint() {
        let mut inspection = matching();
        inspection.installed.clear();
        inspection.opener_missing = true;
        let report = evaluate(&inspection);
        assert!(report.summary.starts_with("2 host default problems: "));
        assert_eq!(
            report
                .summary
                .matches("(fix: qol-tray defaults release folder.open)")
                .count(),
            2
        );
    }

    #[test]
    fn the_headless_result_warns_and_attaches_the_fix() {
        let mut inspection = matching();
        inspection.opener_missing = true;
        let result = headless_result(evaluate(&inspection));
        assert_eq!(result.id, ID);
        assert_eq!(result.status, qol_headless::DoctorStatus::Warn);
        assert_eq!(
            result.fix.as_deref(),
            Some("qol-tray defaults release folder.open")
        );
        let healthy = headless_result(evaluate(&matching()));
        assert_eq!(healthy.status, qol_headless::DoctorStatus::Ok);
    }

    #[test]
    fn findings_add_up_and_advice_is_deduplicated() {
        let mut inspection = matching();
        inspection.installed.clear();
        inspection.opener_missing = true;
        inspection.claims[0].current = CurrentValue::Value("nemo.desktop".to_string());
        let report = evaluate(&inspection);
        assert_eq!(report.issues.len(), 3);
        assert_eq!(report.advice, ["qol-tray defaults release folder.open"]);
    }
}
