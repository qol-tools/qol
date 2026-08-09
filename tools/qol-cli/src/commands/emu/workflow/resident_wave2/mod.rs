use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use super::{Run, Verdict};
use crate::progress::{step_label, StepKind};

pub mod bundle;

const STICK_DEVICE: &str = "/dev/sda";
const STICK_WAIT_ATTEMPTS: u8 = 40;
const STICK_WAIT_INTERVAL_SECS: f32 = 0.25;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(900);
pub(crate) const SCENARIO: &str = include_str!("scenario.sh");

const APT_PREFERENCES_WORKFLOW: &str = "resident-wave2-apt-preferences";
const PACKAGE_CONTRACT_WORKFLOW: &str = "resident-wave2-package-contract";

const PHASE1_CASES: [&str; 35] = [
    "deb-setup",
    "journal-direct-cycle",
    "journal-operator-neighbor",
    "journal-stage-collision",
    "collision-no-clobber",
    "collision-marker-preserved",
    "adopt-pins",
    "module-version-snapshotted",
    "status-as-user",
    "upgrade-pinned",
    "full-upgrade-pinned",
    "unattended-pinned",
    "control-advances",
    "drift-observed",
    "drift-release-no-deletion",
    "release",
    "ownership",
    "owner-release-order",
    "owner-idempotence",
    "cross-process-lock",
    "update-continuity",
    "raw-artifact-gate",
    "journal-valid-stage-recovery",
    "interrupted-preparing-journal",
    "interrupted-staged-write",
    "interrupted-staged-link",
    "interrupted-publish",
    "interrupted-release",
    "publish-fsync-unwind",
    "release-fsync-evidence",
    "exact-copy-staged-preserved",
    "journal-stage-wrong-inode",
    "dangling-fragment-unjournaled",
    "nofollow-dir-swap-refused",
    "interrupted-reboot",
];

const PHASE2_CASES: [&str; 5] = [
    "reboot-resume",
    "staged-fault-recovery",
    "deb-lifecycle",
    "post-release-update",
    "final-residue",
];

const CONTRACT_CASES: [&str; 6] = [
    "contract-fixture",
    "package-contract",
    "package-contract-active",
    "contract-fail-closed",
    "contract-lifecycle",
    "final-residue",
];

pub(super) fn run(run: &mut Run<'_>) -> Result<Verdict> {
    bundle::prepare_results_stick(run.stick)?;
    run.insert()?;
    run.serial.run_command(&wait_for_stick(), COMMAND_TIMEOUT)?;
    run.serial
        .run_command("mount /dev/sda /mnt", COMMAND_TIMEOUT)?;
    run.serial.run_command(
        &bundle::scenario_command("phase1", APT_PREFERENCES_WORKFLOW)?,
        SCENARIO_TIMEOUT,
    )?;
    let phase1_text = run
        .serial
        .run_command("cat /mnt/results-phase1.json", COMMAND_TIMEOUT)?;
    run.serial.run_command("umount /mnt", COMMAND_TIMEOUT)?;
    run.reboot()?;
    run.serial
        .run_command("mount /dev/sda /mnt", COMMAND_TIMEOUT)?;
    run.serial.run_command(
        &bundle::scenario_command("phase2", APT_PREFERENCES_WORKFLOW)?,
        SCENARIO_TIMEOUT,
    )?;
    let phase2_text = run
        .serial
        .run_command("cat /mnt/results.json", COMMAND_TIMEOUT)?;
    run.serial.run_command("umount /mnt", COMMAND_TIMEOUT)?;
    run.pull()?;
    let traces = qol_owned_residue(run.list_traces()?);
    let phase1_json = extract_json(&phase1_text).context("phase1 scenario results not found")?;
    let phase2_json = extract_json(&phase2_text).context("phase2 scenario results not found")?;
    let phase1 = parse_results(phase1_json, APT_PREFERENCES_WORKFLOW, &PHASE1_CASES)
        .context("failed to parse phase1 results")?;
    let phase2 = parse_results(phase2_json, APT_PREFERENCES_WORKFLOW, &PHASE2_CASES)
        .context("failed to parse phase2 results")?;
    let results = merge_phases(phase1, phase2);
    let results_json =
        serde_json::to_string(&results).context("failed to serialize merged results")?;
    let case_dir = run
        .stick
        .parent()
        .context("workflow stick has no run directory")?;
    let results_artifact = case_dir.join("wave2-results.json");
    std::fs::write(&results_artifact, results_json)
        .context("failed to persist scenario results")?;
    let image_sha256 = image_identity(run.image_path)?;
    let identity = Identity {
        os_id: results.os.as_ref().map(|os| os.id.clone()),
        os_version: results.os.as_ref().map(|os| os.version.clone()),
        image_path: run.image_path.display().to_string(),
        image_sha256: image_sha256.clone(),
    };
    let identity_artifact = case_dir.join("wave2-identity.json");
    std::fs::write(&identity_artifact, serde_json::to_string_pretty(&identity)?)
        .context("failed to persist guest identity evidence")?;
    let pass = verdict_of(&results) && identity.binds_supported_os() && traces.is_empty();
    step_label(
        "wave2",
        StepKind::Info,
        &format!(
            "{} cases, {} passed, os {} {}, image sha256 {}, residue traces: {}",
            results.cases.len(),
            results.summary.passed,
            identity.os_id.as_deref().unwrap_or("unknown"),
            identity.os_version.as_deref().unwrap_or("unknown"),
            &image_sha256[..12.min(image_sha256.len())],
            traces.len()
        ),
    );
    Ok(Verdict {
        pass,
        traces,
        artifacts: vec![results_artifact, identity_artifact],
    })
}

pub(super) fn run_package_contract(run: &mut Run<'_>) -> Result<Verdict> {
    bundle::prepare_results_stick(run.stick)?;
    run.insert()?;
    run.serial.run_command(&wait_for_stick(), COMMAND_TIMEOUT)?;
    run.serial
        .run_command("mount /dev/sda /mnt", COMMAND_TIMEOUT)?;
    run.serial.run_command(
        &bundle::scenario_command("contract", PACKAGE_CONTRACT_WORKFLOW)?,
        SCENARIO_TIMEOUT,
    )?;
    let results_text = run
        .serial
        .run_command("cat /mnt/results.json", COMMAND_TIMEOUT)?;
    run.serial.run_command("umount /mnt", COMMAND_TIMEOUT)?;
    run.pull()?;
    let traces = qol_owned_residue(run.list_traces()?);
    let results = parse_results(
        extract_json(&results_text).context("scenario results not found")?,
        PACKAGE_CONTRACT_WORKFLOW,
        &CONTRACT_CASES,
    )
    .context("failed to parse scenario results")?;
    let results_json =
        serde_json::to_string(&results).context("failed to serialize scenario results")?;
    let case_dir = run
        .stick
        .parent()
        .context("workflow stick has no run directory")?;
    let results_artifact = case_dir.join("contract-results.json");
    std::fs::write(&results_artifact, results_json)
        .context("failed to persist scenario results")?;
    let image_sha256 = image_identity(run.image_path)?;
    let identity = Identity {
        os_id: results.os.as_ref().map(|os| os.id.clone()),
        os_version: results.os.as_ref().map(|os| os.version.clone()),
        image_path: run.image_path.display().to_string(),
        image_sha256: image_sha256.clone(),
    };
    let identity_artifact = case_dir.join("contract-identity.json");
    std::fs::write(&identity_artifact, serde_json::to_string_pretty(&identity)?)
        .context("failed to persist guest identity evidence")?;
    let pass = verdict_of(&results) && identity.binds_supported_os() && traces.is_empty();
    step_label(
        "contract",
        StepKind::Info,
        &format!(
            "{} cases, {} passed, os {} {}, image sha256 {}, residue traces: {}",
            results.cases.len(),
            results.summary.passed,
            identity.os_id.as_deref().unwrap_or("unknown"),
            identity.os_version.as_deref().unwrap_or("unknown"),
            &image_sha256[..12.min(image_sha256.len())],
            traces.len()
        ),
    );
    Ok(Verdict {
        pass,
        traces,
        artifacts: vec![results_artifact, identity_artifact],
    })
}

fn qol_owned_residue(traces: Vec<String>) -> Vec<String> {
    traces
        .into_iter()
        .filter(|path| {
            path == "/var/lib/qol-resident-policy-nvidia-driver-version-pin.json"
                || path == "/var/lib/.qol-resident-policy-nvidia-driver-version-pin.json.stage"
                || path.starts_with("/etc/apt/preferences.d/90qol-")
                || path.contains("qol-stage-")
        })
        .collect()
}

fn merge_phases(phase1: Wave2Results, phase2: Wave2Results) -> Wave2Results {
    let os = phase2.os.or_else(|| phase1.os.clone());
    let mut cases = phase1.cases;
    cases.extend(phase2.cases);
    let total = cases.len();
    let passed = cases.iter().filter(|case| case.pass).count();
    Wave2Results {
        workflow: phase1.workflow,
        completed: phase1.completed && phase2.completed,
        os,
        cases,
        summary: Summary { total, passed },
    }
}

fn image_identity(path: &Path) -> Result<String> {
    qol_dev_env::cached_image_sha256(path).with_context(|| {
        format!(
            "failed to hash guest image {} for identity binding",
            path.display()
        )
    })
}

fn wait_for_stick() -> String {
    format!(
        "i=0; while [ $i -lt {STICK_WAIT_ATTEMPTS} ]; do [ -b {STICK_DEVICE} ] && break; \
         i=$((i+1)); sleep {STICK_WAIT_INTERVAL_SECS}; done; \
         [ -b {STICK_DEVICE} ] || (lsblk; false)"
    )
}

fn extract_json(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaseResult {
    id: String,
    pass: bool,
    detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct OsIdentity {
    id: String,
    version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Wave2Results {
    workflow: String,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    os: Option<OsIdentity>,
    #[serde(default)]
    cases: Vec<CaseResult>,
    summary: Summary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Summary {
    total: usize,
    passed: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct Identity {
    os_id: Option<String>,
    os_version: Option<String>,
    image_path: String,
    image_sha256: String,
}

impl Identity {
    fn binds_supported_os(&self) -> bool {
        matches!(self.os_id.as_deref(), Some("debian" | "ubuntu"))
    }
}

fn parse_results(
    text: &str,
    expected_workflow: &str,
    expected_cases: &[&str],
) -> Result<Wave2Results> {
    let results: Wave2Results =
        serde_json::from_str(text).context("scenario results are not valid JSON")?;
    if results.workflow != expected_workflow {
        bail!(
            "scenario workflow `{}` does not match expected `{expected_workflow}`",
            results.workflow
        );
    }
    validate_cases(&results.cases, expected_cases)?;
    if results.summary.total != results.cases.len() {
        bail!(
            "scenario summary total {} disagrees with {} recorded cases",
            results.summary.total,
            results.cases.len()
        );
    }
    let passed = results.cases.iter().filter(|case| case.pass).count();
    if results.summary.passed != passed {
        bail!(
            "scenario summary passed {} disagrees with {} passing cases",
            results.summary.passed,
            passed
        );
    }
    Ok(results)
}

fn validate_cases(cases: &[CaseResult], expected: &[&str]) -> Result<()> {
    if cases.len() != expected.len() {
        bail!(
            "scenario recorded {} cases, expected {}",
            cases.len(),
            expected.len()
        );
    }
    for (index, (case, expected_id)) in cases.iter().zip(expected).enumerate() {
        if case.id != *expected_id {
            bail!(
                "scenario case {index} is `{}`, expected `{}`",
                case.id,
                expected_id
            );
        }
    }
    Ok(())
}

fn verdict_of(results: &Wave2Results) -> bool {
    results.completed && results.cases.iter().all(|case| case.pass)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CASES: [&str; 2] = ["collision-no-clobber", "release"];

    fn sample_cases() -> Vec<CaseResult> {
        vec![
            CaseResult {
                id: "collision-no-clobber".into(),
                pass: true,
                detail: "ok".into(),
            },
            CaseResult {
                id: "release".into(),
                pass: true,
                detail: "ok".into(),
            },
        ]
    }

    fn results_text(workflow: &str, cases: Vec<CaseResult>, total: usize, passed: usize) -> String {
        serde_json::to_string(&serde_json::json!({
            "workflow": workflow,
            "completed": true,
            "os": { "id": "ubuntu", "version": "24.04" },
            "cases": cases,
            "summary": { "total": total, "passed": passed },
        }))
        .unwrap()
    }

    #[test]
    fn scenario_unattended_pattern_accepts_the_wave2_origin_and_keeps_the_real_command() {
        let unattended = SCENARIO
            .lines()
            .skip_while(|line| !line.contains("case_unattended"))
            .take_while(|line| !line.contains("case_control_advances"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            unattended.contains("Unattended-Upgrade::Origins-Pattern"),
            "the unattended case must configure an origins pattern"
        );
        assert!(
            unattended.contains("pattern='origin=*'"),
            "with every non-wave2 source removed, the apt pin is the protection boundary and \
             the origins pattern must accept the local wave2 origin via origin=*"
        );
        assert!(
            !unattended.contains("archive=now"),
            "the pattern must not depend on the Release suite token matching in the guest"
        );
        assert!(
            unattended.contains("unattended-upgrade -d"),
            "the real unattended-upgrade command must stay in use when it exists"
        );
        assert!(
            unattended.contains("/var/log/unattended-upgrades/unattended-upgrades.log"),
            "a nonzero real unattended-upgrade exit must retain one bounded final line from its log"
        );
        assert!(
            unattended.contains("tail -n 1 \"$LOG\""),
            "the retained evidence must fall back to the scenario stderr log when the real log is \
             absent or empty"
        );
        assert!(
            unattended.contains("evidence=$evidence"),
            "the retained evidence must ride the existing sanitized record detail"
        );
    }

    #[test]
    fn apt_health_harness_proves_retention_bounding_propagation_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = apt_health_harness(dir.path());
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "apt-health harness failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let calls = std::fs::read_to_string(dir.path().join("fail-hard.calls")).unwrap();
        assert!(
            calls.contains("apt-get check failed"),
            "fail_hard must propagate the apt-get check failure: {calls}"
        );
        let captured =
            std::fs::read_to_string(dir.path().join("fail-stage/apt-check.log")).unwrap();
        assert!(
            captured.contains("apt stdout line 0"),
            "the first stdout line must be retained in the exact stage file: {captured}"
        );
        assert!(
            captured.contains("apt stdout line 19"),
            "the last stdout line must be retained: {captured}"
        );
        assert!(
            captured.contains("apt stderr unmet-package line"),
            "stderr must be retained alongside stdout: {captured}"
        );
        let log = std::fs::read_to_string(dir.path().join("fail.log")).unwrap();
        assert!(
            !log.contains("apt stdout line 0"),
            "early stdout lines must not reach the scenario log: {log}"
        );
        assert!(
            !log.contains("apt stdout line 15"),
            "lines outside the bounded tail must not reach the scenario log: {log}"
        );
        assert!(
            log.contains("apt stdout line 16"),
            "the first bounded tail line must reach the scenario log: {log}"
        );
        assert!(
            log.contains("apt stdout line 19"),
            "the last bounded tail line must reach the scenario log: {log}"
        );
        assert!(
            log.contains("apt stderr unmet-package line"),
            "the final stderr line must reach the scenario log: {log}"
        );
        let tail_lines = log
            .lines()
            .skip_while(|line| !line.starts_with("w2 apt-check tail:"))
            .skip(1)
            .count();
        assert_eq!(
            tail_lines, 5,
            "only the bounded five-line tail must be appended: {log}"
        );
        assert!(
            !dir.path().join("ok-stage/apt-check.log").exists(),
            "the successful check must remove the exact apt-check file"
        );
    }

    #[test]
    fn parse_accepts_the_scenario_shape_with_os_identity() {
        let text = results_text("resident-wave2-apt-preferences", sample_cases(), 2, 2);
        let results = parse_results(
            text.as_str(),
            "resident-wave2-apt-preferences",
            &SAMPLE_CASES,
        )
        .unwrap();
        assert_eq!(results.cases.len(), 2);
        assert_eq!(results.os.as_ref().map(|os| os.id.as_str()), Some("ubuntu"));
        assert!(verdict_of(&results));
    }

    #[test]
    fn parse_tolerates_a_missing_os_identity() {
        let text = serde_json::to_string(&serde_json::json!({
            "workflow": "resident-wave2-apt-preferences",
            "completed": true,
            "cases": sample_cases(),
            "summary": { "total": 2, "passed": 2 },
        }))
        .unwrap();
        assert!(parse_results(
            text.as_str(),
            "resident-wave2-apt-preferences",
            &SAMPLE_CASES
        )
        .unwrap()
        .os
        .is_none());
    }

    #[test]
    fn parse_rejects_wrong_missing_duplicate_extra_and_reordered_cases() {
        let base = "resident-wave2-apt-preferences";
        let wrong_workflow = results_text("resident-wave2-package-contract", sample_cases(), 2, 2);
        assert!(parse_results(wrong_workflow.as_str(), base, &SAMPLE_CASES).is_err());
        let mut missing = sample_cases();
        missing.pop();
        assert!(parse_results(
            results_text(base, missing, 1, 1).as_str(),
            base,
            &SAMPLE_CASES
        )
        .is_err());
        let mut duplicate = sample_cases();
        duplicate[1] = duplicate[0].clone();
        assert!(parse_results(
            results_text(base, duplicate, 2, 2).as_str(),
            base,
            &SAMPLE_CASES
        )
        .is_err());
        let mut extra = sample_cases();
        extra.push(CaseResult {
            id: "extra-case".into(),
            pass: true,
            detail: "ok".into(),
        });
        assert!(parse_results(
            results_text(base, extra, 3, 3).as_str(),
            base,
            &SAMPLE_CASES
        )
        .is_err());
        let mut reordered = sample_cases();
        reordered.reverse();
        assert!(parse_results(
            results_text(base, reordered, 2, 2).as_str(),
            base,
            &SAMPLE_CASES
        )
        .is_err());
    }

    #[test]
    fn parse_rejects_summary_disagreement() {
        let base = "resident-wave2-apt-preferences";
        let wrong_total = results_text(base, sample_cases(), 3, 2);
        assert!(parse_results(wrong_total.as_str(), base, &SAMPLE_CASES).is_err());
        let wrong_passed = results_text(base, sample_cases(), 2, 1);
        assert!(parse_results(wrong_passed.as_str(), base, &SAMPLE_CASES).is_err());
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        let text = serde_json::to_string(&serde_json::json!({
            "workflow": "resident-wave2-apt-preferences",
            "completed": true,
            "unexpected": true,
            "cases": sample_cases(),
            "summary": { "total": 2, "passed": 2 },
        }))
        .unwrap();
        assert!(parse_results(
            text.as_str(),
            "resident-wave2-apt-preferences",
            &SAMPLE_CASES
        )
        .is_err());
    }

    #[test]
    fn identity_binds_only_supported_os_families() {
        let base = Identity {
            os_id: Some("ubuntu".into()),
            os_version: Some("24.04".into()),
            image_path: "/tmp/img.qcow2".into(),
            image_sha256: "abc".into(),
        };
        assert!(base.binds_supported_os());
        let debian = Identity {
            os_id: Some("debian".into()),
            ..base.clone()
        };
        assert!(debian.binds_supported_os());
        for unsupported in ["fedora", "arch", "unknown"] {
            let other = Identity {
                os_id: Some(unsupported.into()),
                ..base.clone()
            };
            assert!(!other.binds_supported_os(), "{unsupported}");
        }
        assert!(!base.binds_supported_os() || base.os_id.is_some());
        let missing = Identity {
            os_id: None,
            ..base
        };
        assert!(!missing.binds_supported_os());
    }

    #[test]
    fn parse_rejects_malformed_json() {
        assert!(parse_results("not json at all", "x", &[]).is_err());
        assert!(parse_results("{\"completed\":true}", "x", &[]).is_err());
    }

    #[test]
    fn verdict_fails_on_any_failing_case_or_incomplete_run() {
        let mut cases = sample_cases();
        cases[1].pass = false;
        let results = Wave2Results {
            workflow: "resident-wave2-apt-preferences".into(),
            completed: true,
            os: None,
            cases,
            summary: Summary {
                total: 2,
                passed: 1,
            },
        };
        assert!(!verdict_of(&results));
        let mut incomplete = sample_cases();
        incomplete[0].pass = true;
        let results = Wave2Results {
            workflow: "resident-wave2-apt-preferences".into(),
            completed: false,
            os: None,
            cases: incomplete,
            summary: Summary {
                total: 2,
                passed: 2,
            },
        };
        assert!(!verdict_of(&results));
    }

    #[test]
    fn every_invoked_case_function_is_defined_in_the_scenario() {
        let definitions: std::collections::BTreeSet<&str> = SCENARIO
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.strip_suffix("() {")
                    .filter(|name| name.starts_with("case_"))
            })
            .collect();
        let mut undefined = Vec::new();
        for line in SCENARIO.lines() {
            let line = line.trim();
            if !line.starts_with("case_") || line.contains('(') {
                continue;
            }
            let name = line.split_whitespace().next().unwrap();
            if !definitions.contains(name) {
                undefined.push(name);
            }
        }
        assert!(
            undefined.is_empty(),
            "the scenario invokes undefined case functions: {undefined:?}"
        );
    }

    fn record_ids_in_function(function: &str) -> Vec<String> {
        let marker = format!("{function}() {{");
        let start = SCENARIO.find(&marker).unwrap() + marker.len();
        let mut ids = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut depth = 1usize;
        for line in SCENARIO[start..].lines() {
            depth = depth
                .saturating_add(line.matches('{').count())
                .saturating_sub(line.matches('}').count());
            if depth == 0 {
                break;
            }
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("record ") {
                if let Some(id) = rest.split_whitespace().next() {
                    let id = id.trim_matches('"').to_string();
                    if seen.insert(id.clone()) {
                        ids.push(id);
                    }
                }
            }
        }
        ids
    }

    fn phase_invocations(block: &str) -> Vec<String> {
        block
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("case_"))
            .map(|line| {
                line.split_whitespace()
                    .next()
                    .unwrap()
                    .trim_start_matches("case_")
                    .to_string()
            })
            .collect()
    }

    fn expected_ids_for(block: &str) -> Vec<String> {
        phase_invocations(block)
            .iter()
            .flat_map(|function| record_ids_in_function(function))
            .collect()
    }

    #[test]
    fn every_crash_hook_case_runs_after_sandbox_install_and_before_production_restore() {
        let start = SCENARIO
            .find("if [ \"$PHASE\" = contract ]; then")
            .expect("the dispatch block must exist");
        let end = start
            + SCENARIO[start..]
                .find("os_id=")
                .expect("the envelope must follow");
        let dispatch = &SCENARIO[start..end];
        let phase1_marker = "elif [ \"$PHASE\" = phase1 ]; then";
        let (contract_block, rest) = dispatch.split_once(phase1_marker).unwrap();
        let (phase1_block, phase2_block) = rest.split_once("else\n").unwrap();
        let install_line = phase1_block
            .lines()
            .position(|line| line.trim() == "install_sandbox_adapter")
            .expect("the sandbox adapter installation must exist in phase1");
        let restore_line = phase2_block
            .lines()
            .position(|line| line.trim() == "restore_production_adapter")
            .expect("the production adapter restoration must exist in phase2");
        let mut crash_cases = Vec::new();
        for line in SCENARIO.lines() {
            let line = line.trim();
            if !line.starts_with("case_") || !line.ends_with("() {") {
                continue;
            }
            let function = line
                .trim_start_matches("case_")
                .trim_end_matches("() {")
                .to_string();
            let body = function_text(SCENARIO, &function);
            if body.contains("QOL_RESIDENT_CRASH_POINT") || body.contains("QOL_RESIDENT_FAIL_NEXT")
            {
                crash_cases.push(function);
            }
        }
        assert!(
            !crash_cases.is_empty(),
            "the audit must find at least one crash-hook case"
        );
        let contract_invocations = phase_invocations(contract_block);
        let phase1_invocations = phase_invocations(phase1_block);
        let phase2_invocations = phase_invocations(phase2_block);
        for function in &crash_cases {
            assert!(
                !contract_invocations.contains(function),
                "crash-hook case `{function}` must never run in the contract phase"
            );
            let in_phase1 = phase1_invocations.iter().filter(|f| *f == function).count();
            let in_phase2 = phase2_invocations.iter().filter(|f| *f == function).count();
            assert_eq!(
                in_phase1 + in_phase2,
                1,
                "crash-hook case `{function}` must be invoked exactly once across phase1 and phase2"
            );
            let invocation = if in_phase1 == 1 {
                phase1_block
                    .lines()
                    .position(|line| line.trim() == format!("case_{function}"))
                    .unwrap()
            } else {
                phase2_block
                    .lines()
                    .position(|line| line.trim() == format!("case_{function}"))
                    .unwrap()
            };
            if in_phase1 == 1 {
                assert!(
                    invocation > install_line,
                    "crash-hook case `{function}` must run after the sandbox adapter installation"
                );
            } else {
                assert!(
                    invocation < restore_line,
                    "crash-hook case `{function}` must run before the production adapter is restored"
                );
            }
        }
    }

    #[test]
    fn scenario_phase_invocations_match_the_expected_ordered_case_lists() {
        let start = SCENARIO
            .find("if [ \"$PHASE\" = contract ]; then")
            .expect("the dispatch block must exist");
        let end = start
            + SCENARIO[start..]
                .find("os_id=")
                .expect("the envelope must follow");
        let dispatch = &SCENARIO[start..end];
        let phase1_marker = "elif [ \"$PHASE\" = phase1 ]; then";
        let (contract_block, rest) = dispatch.split_once(phase1_marker).unwrap();
        let (phase1_block, phase2_block) = rest.split_once("else\n").unwrap();
        assert_eq!(
            expected_ids_for(contract_block),
            CONTRACT_CASES,
            "contract invocation order must match CONTRACT_CASES"
        );
        assert_eq!(
            expected_ids_for(phase1_block),
            PHASE1_CASES,
            "phase1 invocation order must match PHASE1_CASES"
        );
        assert_eq!(
            expected_ids_for(phase2_block),
            PHASE2_CASES,
            "phase2 invocation order must match PHASE2_CASES"
        );
        let stage_start = SCENARIO
            .find("case \"$PHASE\" in")
            .expect("the phase bootstrap section must exist");
        let stage_end = stage_start + SCENARIO[stage_start..].find("\nesac\n").unwrap();
        let stage_section = &SCENARIO[stage_start..stage_end];
        assert_eq!(
            stage_section.matches("mount_payload").count(),
            3,
            "every phase must mount and validate the payload"
        );
        assert!(
            !stage_section
                .lines()
                .any(|line| line.trim().starts_with("case_")),
            "no case may run before the payload bootstrap"
        );
    }

    fn function_text(script: &str, name: &str) -> String {
        let marker = format!("{name}() {{");
        let start = script
            .find(&marker)
            .unwrap_or_else(|| panic!("{name} must be defined"));
        let mut depth = 1usize;
        let mut end = start + marker.len() + 1;
        for line in script[start..].lines().skip(1) {
            depth = depth
                .saturating_add(line.matches('{').count())
                .saturating_sub(line.matches('}').count());
            end += line.len() + 1;
            if depth == 0 {
                break;
            }
        }
        script[start..end].to_string()
    }

    #[test]
    fn staged_collision_cases_assert_the_fail_closed_contract() {
        for (name, record_id, finally_releases) in [
            (
                "case_exact_copy_staged_collision",
                "exact-copy-staged-preserved",
                true,
            ),
            ("case_staged_fault_recovery", "staged-fault-recovery", false),
        ] {
            let body = function_text(SCENARIO, name);
            let first_disable = body
                .find("rp_disable")
                .unwrap_or_else(|| panic!("{name} must call disable"));
            assert!(
                body[first_disable..].contains("rp_enable"),
                "{name} must enable fresh only after the retire disable"
            );
            assert!(
                body.contains("release-failed"),
                "{name} must assert the ReleaseFailed collision state"
            );
            assert!(
                body.contains("-f \"$JOURNAL\""),
                "{name} must require journal evidence"
            );
            assert!(
                body.contains("! -f \"$FRAGMENT\""),
                "{name} must require no fragment"
            );
            assert!(
                body.contains("rm -f \"$staged\""),
                "{name} must remove its scenario-owned staged entry"
            );
            assert!(
                body.contains("= absent"),
                "{name} must retire the journal to Absent"
            );
            let fresh_enable = body.rfind("rp_enable").unwrap();
            let pass = body
                .rfind(&format!("record {record_id} 1"))
                .unwrap_or_else(|| panic!("{name} must record its passing contract"));
            let between = &body[fresh_enable..pass];
            if finally_releases {
                assert!(
                    between.contains("rp_disable"),
                    "{name} must verify the final release before recording pass"
                );
            } else {
                assert!(
                    !between.contains("rp_disable"),
                    "{name} must stay Active through its passing contract for deb-lifecycle"
                );
            }
        }
    }

    #[test]
    fn production_fixtures_never_drift_to_sandbox_only_names_or_probes() {
        const FAMILIES: [&str; 6] = [
            "nvidia-driver",
            "nvidia-kernel-",
            "nvidia-dkms-",
            "nvidia-headless-",
            "linux-modules-nvidia-",
            "nvidia-open-",
        ];
        for forbidden in [
            "fixture-drv",
            "QOL_RESIDENT_TARGET_PATTERNS",
            "QOL_RESIDENT_MODULE_VERSION",
        ] {
            assert!(
                !SCENARIO.contains(forbidden),
                "the production oracle must not rely on {forbidden}"
            );
        }
        let mut fixture_names = std::collections::BTreeSet::new();
        for line in SCENARIO.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("build_deb ") {
                if let Some(name) = rest.split_whitespace().next() {
                    let name = name.trim_matches('"');
                    if name == "$CONTROL" {
                        fixture_names.insert("fixture-ctl".to_string());
                    } else if !name.starts_with('$') {
                        fixture_names.insert(name.to_string());
                    }
                }
            }
            if let Some(rest) = line.strip_prefix("for pkg in ") {
                for name in rest.split_whitespace().take_while(|token| *token != "do") {
                    fixture_names.insert(name.trim_matches('"').trim_end_matches(';').to_string());
                }
            }
        }
        assert!(
            fixture_names.contains("nvidia-driver-fixture-a")
                && fixture_names.contains("nvidia-driver-fixture-b"),
            "the pinned fixtures must live inside the fixed NVIDIA family: {fixture_names:?}"
        );
        for name in &fixture_names {
            if name == "fixture-ctl" {
                assert!(
                    !FAMILIES.iter().any(|family| name.starts_with(family)),
                    "the control package must stay outside every guarded family"
                );
                continue;
            }
            assert!(
                FAMILIES.iter().any(|family| name.starts_with(family)),
                "fixture {name} must match a production guard family"
            );
        }
        let fixture_script = function_text(SCENARIO, "install_modinfo_fixture");
        assert!(
            fixture_script.contains("/usr/sbin/modinfo"),
            "the fixture must live at a fixed absolute system path"
        );
        assert!(
            fixture_script.contains("modinfo.original"),
            "the fixture must record and restore any pre-existing entry"
        );
        assert!(
            fixture_script.contains("-n)"),
            "the fixture must answer the path query"
        );
        assert!(
            fixture_script.contains("-F)"),
            "the fixture must answer the version query"
        );
        assert!(
            fixture_script.contains("580.159.02"),
            "the fixture must return the module version"
        );
        let invocation = SCENARIO
            .find("\ninstall_modinfo_fixture\n")
            .expect("the fixture must be invoked, not merely defined");
        let dispatch = SCENARIO.find("if [ \"$PHASE\" = contract ]; then").unwrap();
        assert!(
            invocation < dispatch,
            "the fixture invocation must precede the phase dispatch"
        );
    }

    #[test]
    fn the_modinfo_fixture_answers_probes_with_fixed_absolute_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("modinfo");
        let fixture_source = {
            let start = SCENARIO.find("<<'FIXEOF'").unwrap() + "<<'FIXEOF'".len();
            let end = start + SCENARIO[start..].find("FIXEOF").unwrap();
            SCENARIO[start..end].trim_start()
        };
        std::fs::write(&fixture, fixture_source).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path_probe = std::process::Command::new(&fixture)
            .arg("-n")
            .arg("nvidia")
            .output()
            .unwrap();
        assert!(
            !path_probe.status.success(),
            "the path query must report no module path"
        );
        assert!(
            path_probe.stdout.is_empty(),
            "the path query must not leak a module path"
        );
        let version_probe = std::process::Command::new(&fixture)
            .args(["-F", "version", "nvidia"])
            .output()
            .unwrap();
        assert!(version_probe.status.success());
        assert_eq!(
            String::from_utf8_lossy(&version_probe.stdout).trim(),
            "580.159.02",
            "the version query must return the fixture module version"
        );
    }

    fn modinfo_fixture_harness(
        dir: &std::path::Path,
    ) -> (
        std::process::Command,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let stage = dir.join("stage");
        let live = dir.join("live");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        for stub in ["dpkg-query", "ls"] {
            std::fs::write(bin.join(stub), "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(
                bin.join(stub),
                std::os::unix::fs::PermissionsExt::from_mode(0o755),
            )
            .unwrap();
        }
        let live_modinfo = live.join("modinfo");
        let harness = dir.join("fixture-harness.sh");
        let substitute = |text: String| text.replace("/usr/sbin/modinfo", "\"$LIVE\"");
        let mut content = String::from(
            "#!/bin/sh\nset -u\nSTAGE=$1\nLIVE=$2\nFRAGMENT=$STAGE/fragment\nJOURNAL=$STAGE/journal\nJOURNAL_DIR=$STAGE\nPROVIDER=qol-headless-deps\nJOURNAL_STAGE=$STAGE/journal.stage\n",
        );
        content.push_str("fail_hard() { echo \"fixture failure: $*\" >&2; exit 1; }\n");
        content.push_str("remove_owned_stage() { return 0; }\n");
        content.push_str("record() { printf '%s' \"$3\" > \"$STAGE/residue.txt\"; }\n");
        content.push_str(&substitute(function_text(
            SCENARIO,
            "restore_modinfo_fixture",
        )));
        content.push('\n');
        content.push_str(&substitute(function_text(
            SCENARIO,
            "install_modinfo_fixture",
        )));
        content.push('\n');
        content.push_str(&substitute(function_text(SCENARIO, "case_final_residue")));
        content.push_str(
            "\ncase \"$3\" in\nfresh)\ninstall_modinfo_fixture\ntouch -t 202001010000 \"$LIVE\"\nchmod 640 \"$LIVE\"\nstat -c '%a %Y' \"$LIVE\" > \"$STAGE/live-before.txt\"\ninstall_modinfo_fixture\nstat -c '%a %Y' \"$LIVE\" > \"$STAGE/live-after.txt\"\n;;\n             replaced)\ninstall_modinfo_fixture\necho \"operator replaced the live entry\" > \"$LIVE\"\n             if (install_modinfo_fixture 2> \"$STAGE/fail.txt\"); then exit 92; fi\n             if (restore_modinfo_fixture 2> \"$STAGE/restore-fail.txt\"); then exit 93; fi\n             case_final_residue\n;;\n             symlink)\ninstall_modinfo_fixture\nrm -f \"$LIVE\"\nln -s /bin/true \"$LIVE\"\n             if (install_modinfo_fixture 2> \"$STAGE/fail.txt\"); then exit 92; fi\n             [ -L \"$LIVE\" ] || exit 95\n;;\n             absent)\ninstall_modinfo_fixture\nrm -f \"$LIVE\"\n             if (install_modinfo_fixture 2> \"$STAGE/fail.txt\"); then exit 92; fi\n             [ -f \"$STAGE/modinfo.fixture\" ] || exit 96\n             [ -f \"$STAGE/modinfo.fixture-script\" ] || exit 97\n;;\n             missing-evidence)\ninstall_modinfo_fixture\nrm -f \"$STAGE/modinfo.fixture-script\"\n             if (install_modinfo_fixture 2> \"$STAGE/fail.txt\"); then exit 92; fi\n             [ -f \"$LIVE\" ] || exit 98\n;;\n             *) exit 99 ;;\nesac\n",
        );
        std::fs::write(&harness, content).unwrap();
        let mut command = std::process::Command::new("sh");
        command.arg(&harness).env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        );
        (command, stage, live_modinfo)
    }

    #[test]
    fn the_modinfo_fixture_is_collision_safe_across_phases_on_the_host() {
        for case in ["fresh", "replaced", "symlink", "absent", "missing-evidence"] {
            let dir = tempfile::tempdir().unwrap();
            let (mut command, stage, live) = modinfo_fixture_harness(dir.path());
            let output = command.arg(&stage).arg(&live).arg(case).output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{case}: {stderr}");
            let fail_detail = std::fs::read_to_string(stage.join("fail.txt")).unwrap_or_default();
            match case {
                "fresh" => {
                    let fixture =
                        std::fs::read_to_string(stage.join("modinfo.fixture-script")).unwrap();
                    assert_eq!(
                        std::fs::read_to_string(&live).unwrap(),
                        fixture,
                        "the fresh and repeated install must leave the live entry byte-identical to the fixture"
                    );
                    assert!(stage.join("modinfo.fixture").exists());
                    let before = std::fs::read_to_string(stage.join("live-before.txt")).unwrap();
                    let after = std::fs::read_to_string(stage.join("live-after.txt")).unwrap();
                    assert_eq!(
                        before, after,
                        "the repeated install must be a true no-op that preserves mode and mtime"
                    );
                    assert!(
                        before.starts_with("640 "),
                        "the recorded mode must survive the repeated install"
                    );
                }
                "replaced" => {
                    let operator = "operator replaced the live entry";
                    assert_eq!(std::fs::read_to_string(&live).unwrap().trim_end(), operator);
                    assert!(
                        fail_detail.contains("no longer matches"),
                        "the second install must refuse the operator replacement: {fail_detail}"
                    );
                    assert!(stage.join("modinfo.fixture").exists());
                    assert!(stage.join("modinfo.fixture-script").exists());
                    assert_eq!(
                        std::fs::read_to_string(stage.join("residue.txt")).unwrap(),
                        "residue: modinfo",
                        "the operator replacement must be reported as residue"
                    );
                    assert_eq!(
                        std::fs::read_to_string(&live).unwrap().trim_end(),
                        operator,
                        "the operator bytes must survive the failed restore byte for byte"
                    );
                }
                "symlink" => {
                    assert!(
                        fail_detail.contains("symlink"),
                        "a symlinked live entry must fail closed: {fail_detail}"
                    );
                    assert!(
                        std::fs::symlink_metadata(&live)
                            .unwrap()
                            .file_type()
                            .is_symlink(),
                        "the symlink must stay untouched"
                    );
                }
                "absent" => {
                    assert!(
                        fail_detail.contains("absent"),
                        "an absent live entry with a marker must fail closed: {fail_detail}"
                    );
                    assert!(stage.join("modinfo.fixture").exists());
                    assert!(stage.join("modinfo.fixture-script").exists());
                }
                "missing-evidence" => {
                    assert!(
                        fail_detail.contains("evidence is incomplete"),
                        "missing evidence must fail closed: {fail_detail}"
                    );
                    assert!(live.exists(), "the live entry must stay untouched");
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn the_production_deb_is_never_installed_with_force_options() {
        assert!(
            !SCENARIO.contains("--force-depends"),
            "the production deb must install through a plain dpkg -i after the guest-only provider"
        );
        assert!(
            SCENARIO.contains("dpkg -i \"$PAYLOAD_ROOT/qol-tray.deb\""),
            "the production deb must be installed with an unforced dpkg -i"
        );
        assert!(
            SCENARIO.contains("Provides: ${provides_list%, }"),
            "the provider fixture must carry the parsed dependency Provides"
        );
        assert!(
            SCENARIO.contains("install ok installed"),
            "the residue gate must detect a still-installed provider fixture"
        );
    }

    fn headless_dep_harness(
        dir: &std::path::Path,
        depends: &str,
    ) -> (std::process::Command, std::path::PathBuf) {
        let bin = dir.join("bin");
        let stage = dir.join("stage");
        let repo = dir.join("repo");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(repo.join("pool")).unwrap();
        std::fs::write(
            bin.join("dpkg-deb"),
            "#!/bin/sh\nif [ \"$1\" = -f ]; then cat \"$DEPS_FIXTURE\"; echo; exit 0; fi\nif [ \"$1\" = --build ]; then : > \"$3\"; printf '%s\\n' \"$3\" >> \"$STAGE/dpkg-deb-outputs.txt\"; exit 0; fi\nexit 0\n",
        )
        .unwrap();
        std::fs::write(bin.join("dpkg"), "#!/bin/sh\nexit 0\n").unwrap();
        for stub in ["dpkg-deb", "dpkg"] {
            std::fs::set_permissions(
                bin.join(stub),
                std::os::unix::fs::PermissionsExt::from_mode(0o755),
            )
            .unwrap();
        }
        let fixture = dir.join("depends.txt");
        std::fs::write(&fixture, depends).unwrap();
        let harness = dir.join("harness.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\nexport STAGE=$1\nREPO=$2\nPAYLOAD_ROOT=$3\nPROVIDER=qol-headless-deps\nLOG=$STAGE/log\nexport DEPS_FIXTURE=$4\n{}\n{}\nfail_hard() {{ echo \"provider failure: $*\" >&2; exit 1; }}\n{}\n{}\n{}\ninstall_headless_deps_provider\n",
                SCENARIO
                    .lines()
                    .find(|line| line.starts_with("REAL_CORE_DEPS="))
                    .expect("REAL_CORE_DEPS must be defined"),
                SCENARIO
                    .lines()
                    .find(|line| line.starts_with("FIXTURE_DEPS="))
                    .expect("FIXTURE_DEPS must be defined"),
                function_text(SCENARIO, "headless_dep_name"),
                function_text(SCENARIO, "headless_dep_constraint"),
                function_text(SCENARIO, "install_headless_deps_provider")
            ),
        )
        .unwrap();
        let mut command = std::process::Command::new("sh");
        command
            .arg(&harness)
            .arg(&stage)
            .arg(&repo)
            .arg(dir.join("payload"))
            .arg(&fixture)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            );
        (
            command,
            stage.join("build/qol-headless-deps/DEBIAN/control"),
        )
    }

    fn apt_health_harness(dir: &std::path::Path) -> std::process::Command {
        let bin = dir.join("bin");
        let fail_stage = dir.join("fail-stage");
        let ok_stage = dir.join("ok-stage");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&fail_stage).unwrap();
        std::fs::create_dir_all(&ok_stage).unwrap();
        std::fs::write(
            bin.join("apt-get"),
            "#!/bin/sh\ni=0\nwhile [ $i -lt 20 ]; do\n    echo \"apt stdout line $i\"\n    i=$((i + 1))\ndone\necho \"apt stderr unmet-package line\" >&2\nif [ \"${STUB_APT_FAIL:-0}\" = 1 ]; then\n    exit 1\nfi\nexit 0\n",
        )
        .unwrap();
        std::fs::write(bin.join("dpkg"), "#!/bin/sh\nexit 0\n").unwrap();
        for stub in ["apt-get", "dpkg"] {
            std::fs::set_permissions(
                bin.join(stub),
                std::os::unix::fs::PermissionsExt::from_mode(0o755),
            )
            .unwrap();
        }
        let harness = dir.join("harness.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\nFAIL_STAGE=$1\nOK_STAGE=$2\nFAIL_LOG=$3\nOK_LOG=$4\nFAIL_CALLS=$5\nfail_hard() {{ echo \"fail_hard:$*\" >>\"$FAIL_CALLS\"; exit 1; }}\n{}\nexport STUB_APT_FAIL=1\n(\n    STAGE=\"$FAIL_STAGE\"\n    LOG=\"$FAIL_LOG\"\n    require_apt_health\n)\nfail_rc=$?\nexport STUB_APT_FAIL=0\n(\n    STAGE=\"$OK_STAGE\"\n    LOG=\"$OK_LOG\"\n    require_apt_health\n)\nok_rc=$?\n[ $fail_rc -ne 0 ] || exit 2\n[ $ok_rc -eq 0 ] || exit 3\nexit 0\n",
                function_text(SCENARIO, "require_apt_health"),
            ),
        )
        .unwrap();
        let mut command = std::process::Command::new("sh");
        command
            .arg(&harness)
            .arg(&fail_stage)
            .arg(&ok_stage)
            .arg(dir.join("fail.log"))
            .arg(dir.join("ok.log"))
            .arg(dir.join("fail-hard.calls"))
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            );
        command
    }

    fn deb_lifecycle_harness(
        dir: &std::path::Path,
        remove_status: u32,
        remove_output: &str,
    ) -> std::process::Command {
        let bin = dir.join("bin");
        let stage = dir.join("stage");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        let payload = dir.join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(payload.join("qol-tray.deb"), b"deb").unwrap();
        let remove_output_path = dir.join("remove-output.txt");
        std::fs::write(&remove_output_path, remove_output).unwrap();
        std::fs::write(
            bin.join("qol-resident-policy"),
            "#!/bin/sh\nif [ \"$1\" = status ]; then echo \"state=active owners=stable-owner\"; exit 0; fi\nexit 0\n",
        )
        .unwrap();
        std::fs::write(
            bin.join("dpkg"),
            format!(
                "#!/bin/sh\nif [ \"$1\" = -r ]; then cat \"{}\"; exit {remove_status}; fi\nexit 0\n",
                remove_output_path.display()
            ),
        )
        .unwrap();
        std::fs::write(
            bin.join("dpkg-query"),
            "#!/bin/sh\nif [ \"$1\" = -W ]; then case \"$4\" in qol-tray) echo \"1.0\"; exit 0 ;; qol-headless-deps) exit 1 ;; esac; fi\nexit 0\n",
        )
        .unwrap();
        std::fs::write(bin.join("apt-get"), "#!/bin/sh\nexit 0\n").unwrap();
        for stub in ["qol-resident-policy", "dpkg", "dpkg-query", "apt-get"] {
            std::fs::set_permissions(
                bin.join(stub),
                std::os::unix::fs::PermissionsExt::from_mode(0o755),
            )
            .unwrap();
        }
        let harness = dir.join("harness.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\nSTAGE={}\nRESULTS={}\nPAYLOAD_ROOT={}\nTRAY={}\nPOLICY=nvidia-driver-version-pin\nJOURNAL=$STAGE/journal\nJOURNAL_STAGE=$STAGE/journal-stage\nFRAGMENT=$STAGE/fragment\nPROVIDER=qol-headless-deps\nLOG=$STAGE/log\n{}\n{}\n{}\n{}\n{}\ncase_deb_lifecycle\n",
                stage.display(),
                dir.join("results.jsonl").display(),
                payload.display(),
                bin.join("qol-resident-policy").display(),
                function_text(SCENARIO, "record"),
                function_text(SCENARIO, "rp_status_state"),
                function_text(SCENARIO, "rp_status_owners"),
                function_text(SCENARIO, "require_apt_health"),
                function_text(SCENARIO, "case_deb_lifecycle")
            ),
        )
        .unwrap();
        let mut command = std::process::Command::new("sh");
        command.arg(&harness).env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        );
        command
    }

    #[test]
    fn deb_lifecycle_remove_failure_falls_back_to_a_bounded_tail() {
        let dir = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (1..=30).map(|line| format!("l{line:02}")).collect();
        let remove_output = format!("{}\n", lines.join("\n"));
        std::fs::create_dir_all(dir.path().join("stage")).unwrap();
        std::fs::write(dir.path().join("stage/journal"), b"journal").unwrap();
        std::fs::write(dir.path().join("stage/fragment"), b"fragment").unwrap();
        let mut command = deb_lifecycle_harness(dir.path(), 7, &remove_output);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recorded = std::fs::read_to_string(dir.path().join("results.jsonl")).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(recorded.lines().next().unwrap()).unwrap();
        assert_eq!(first["id"], "deb-lifecycle");
        assert_eq!(first["pass"], false);
        let detail = first["detail"].as_str().unwrap();
        assert!(
            detail.contains("evidence: l19"),
            "the bounded tail must lead the record: {detail}"
        );
        assert!(
            detail.contains("l30"),
            "the last evidence line must be surfaced: {detail}"
        );
        assert!(
            detail.contains("| rc=7 journal=present fragment=present"),
            "{detail}"
        );
        assert!(
            !detail.contains("l01") && !detail.contains("l02"),
            "lines beyond the bounded tail must stay out of the record: {detail}"
        );
        assert!(
            detail.len() <= 200,
            "the record detail must stay bounded: {detail}"
        );
        let evidence = std::fs::read_to_string(dir.path().join("stage/deb-remove.log")).unwrap();
        assert_eq!(
            evidence, remove_output,
            "the exact combined remove output must be retained for post-mortem"
        );
    }

    #[test]
    fn deb_lifecycle_remove_failure_surfaces_the_resident_policy_cause() {
        let dir = tempfile::tempdir().unwrap();
        let remove_output = "(Reading database ... 22321 files and directories currently installed.)\nRemoving qol-tray (3.51.0-1) ...\nresident-policy: the qol-tray package is not currently installed (dpkg status-abbrev `rF `); resident activation requires an installed package\ndpkg: error processing package qol-tray (--remove):\n installed qol-tray package pre-removal script subprocess returned error exit status 1\n";
        std::fs::create_dir_all(dir.path().join("stage")).unwrap();
        std::fs::write(dir.path().join("stage/journal"), b"journal").unwrap();
        std::fs::write(dir.path().join("stage/fragment"), b"fragment").unwrap();
        let mut command = deb_lifecycle_harness(dir.path(), 7, remove_output);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recorded = std::fs::read_to_string(dir.path().join("results.jsonl")).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(recorded.lines().next().unwrap()).unwrap();
        assert_eq!(first["id"], "deb-lifecycle");
        assert_eq!(first["pass"], false);
        let detail = first["detail"].as_str().unwrap();
        assert!(
            detail.starts_with("evidence: resident-policy:"),
            "the resident-policy cause must lead the record: {detail}"
        );
        assert!(
            detail.contains(
                "resident-policy: the qol-tray package is not currently installed (dpkg status-abbrev `rF `)"
            ),
            "the exact CarrierError cause must survive the record: {detail}"
        );
        assert!(
            detail.contains("rF "),
            "the literal dpkg status-abbrev must survive the record: {detail}"
        );
        assert!(
            detail.contains("| rc=7 journal=present fragment=present"),
            "{detail}"
        );
        assert!(
            !detail.contains("Reading database"),
            "the dpkg preamble must not displace the cause: {detail}"
        );
        assert!(
            !detail.contains("pre-removal script subprocess"),
            "the dpkg failure epilogue must not displace the cause: {detail}"
        );
        assert!(
            detail.len() <= 200,
            "the record detail must stay bounded: {detail}"
        );
        let evidence = std::fs::read_to_string(dir.path().join("stage/deb-remove.log")).unwrap();
        assert_eq!(
            evidence, remove_output,
            "the exact combined remove output must be retained byte for byte"
        );
    }

    #[test]
    fn deb_lifecycle_success_removes_the_evidence_file() {
        let dir = tempfile::tempdir().unwrap();
        let remove_output = "remove evidence lines\nthat must be cleaned up\n";
        let mut command = deb_lifecycle_harness(dir.path(), 0, remove_output);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let recorded = std::fs::read_to_string(dir.path().join("results.jsonl")).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(recorded.lines().next().unwrap()).unwrap();
        assert_eq!(first["id"], "deb-lifecycle");
        assert_eq!(first["pass"], true);
        assert!(
            !dir.path().join("stage/deb-remove.log").exists(),
            "successful removal must delete the exact evidence file"
        );
    }

    #[test]
    fn headless_dep_provider_accepts_core_and_fixture_lists_with_t64_and_builds_versioned_provides()
    {
        let dir = tempfile::tempdir().unwrap();
        let depends = [
            "libc6 (>= 2.38)",
            "libglib2.0-0t64 (>= 2.80.0-6ubuntu2)",
            "libgtk-3-0t64 (>= 3.24.41-1ubuntu1) | libgtk-3-0 (>= 3.24.41-1ubuntu1)",
            "libayatana-appindicator3-1 (>= 0.5.3)",
        ]
        .join("\n");
        let (mut command, control) = headless_dep_harness(dir.path(), &depends);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let control = std::fs::read_to_string(control).unwrap();
        assert!(
            control.contains("Provides: libglib2.0-0t64 (= 2.80.0-6ubuntu2), libgtk-3-0t64 (= 3.24.41-1ubuntu1), libayatana-appindicator3-1 (= 0.5.3)"),
            "the versioned Provides must cover the t64 and alternate-named fixture deps: {control}"
        );
        assert!(
            !control.contains("libc6"),
            "core libraries must never be provided by the guest-only fixture: {control}"
        );
        let built = dir.path().join("stage/qol-headless-deps_1.0_all.deb");
        assert!(
            built.is_file(),
            "the provider deb must exist at its owned build path"
        );
        let outputs =
            std::fs::read_to_string(dir.path().join("stage/dpkg-deb-outputs.txt")).unwrap();
        let recorded = outputs.lines().next().unwrap();
        assert_eq!(
            recorded,
            built.to_string_lossy(),
            "dpkg-deb must build exactly the owned provider artifact"
        );
        assert!(
            !recorded.starts_with(&dir.path().join("repo").to_string_lossy().into_owned()),
            "the provider deb must be built outside the repository: {recorded}"
        );
        let pool: Vec<String> = std::fs::read_dir(dir.path().join("repo/pool"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            !pool.iter().any(|name| name.contains("qol-headless-deps")),
            "no provider deb may enter the repo pool: {pool:?}"
        );
    }

    #[test]
    fn headless_dep_provider_provides_the_xcb_keyboard_fixture_deps_with_constraints() {
        let dir = tempfile::tempdir().unwrap();
        let depends = [
            "libc6 (>= 2.38)",
            "libxcb1 (>= 1.15-1)",
            "libxkbcommon0 (>= 1.5.0-1)",
        ]
        .join("\n");
        let (mut command, control) = headless_dep_harness(dir.path(), &depends);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let control = std::fs::read_to_string(control).unwrap();
        assert!(
            control.contains("Provides: libxcb1 (= 1.15-1), libxkbcommon0 (= 1.5.0-1)"),
            "the generated provider must provide both headless-safe X deps with their exact version constraints: {control}"
        );
        assert!(
            !control.contains("libc6"),
            "true adapter/core dependencies must never be provided by the guest-only fixture: {control}"
        );
    }

    #[test]
    fn headless_dep_provider_drifts_closed_on_unapproved_names() {
        let dir = tempfile::tempdir().unwrap();
        let depends = "libc6 (>= 2.38)\nlibbogus-9 (>= 1.0)\n";
        let (mut command, _control) = headless_dep_harness(dir.path(), depends);
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("dependency drift"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn headless_dep_provider_rejects_malformed_names() {
        let dir = tempfile::tempdir().unwrap();
        let depends = "libevil;rm -rf (>= 1.0)\n";
        let (mut command, _control) = headless_dep_harness(dir.path(), depends);
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("malformed dependency name"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn record_escapes_hostile_detail_into_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let results = dir.path().join("results.jsonl");
        let harness = dir.path().join("record.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\nRESULTS={}\n{}\nrecord \"id-x\" 1 \"$(printf 'a\\\"b\\\\c\\td\\n')\"\nrecord \"id-y\" 0 \"$(printf 'line1\\nline2\\n')\"\n",
                results.display(),
                function_text(SCENARIO, "record")
            ),
        )
        .unwrap();
        let output = std::process::Command::new("sh")
            .arg(&harness)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines = std::fs::read_to_string(&results).unwrap();
        let mut lines = lines.lines();
        let first: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(first["id"], "id-x");
        assert_eq!(first["pass"], true);
        assert_eq!(
            first["detail"], "a\\\"b\\cd",
            "control characters must be stripped and backslashes and quotes escaped"
        );
        let second: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(second["detail"], "line1line2");
        assert!(lines.next().is_none());
    }

    fn fail_hard_tail_bound() -> usize {
        let fail_hard = function_text(SCENARIO, "fail_hard");
        fail_hard
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("tail -n ")
                    .and_then(|rest| rest.split_whitespace().next())
                    .and_then(|value| value.parse().ok())
            })
            .expect("fail_hard must configure a bounded tail")
    }

    fn fail_hard_harness(dir: &std::path::Path, results: &std::path::Path) -> std::path::PathBuf {
        let harness = dir.join("fail-hard.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\nSTAGE={}\nLOG=$STAGE/scenario.log\nRESULTS={}\nlog() {{ echo \"w2: $*\" >> \"$LOG\"; }}\n{}\n{}\nfail_hard \"boom\"\n",
                dir.join("stage").display(),
                results.display(),
                function_text(SCENARIO, "record"),
                function_text(SCENARIO, "fail_hard")
            ),
        )
        .unwrap();
        harness
    }

    #[test]
    fn fail_hard_forwards_only_the_configured_tail_and_keeps_the_generic_failure() {
        let bound = fail_hard_tail_bound();
        assert!(bound > 0, "the tail bound must be positive");
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let total = bound + 20;
        let mut seeded = String::new();
        for line in 1..=total {
            seeded.push_str(&format!("prelude line {line:04}\n"));
        }
        std::fs::write(stage.join("scenario.log"), &seeded).unwrap();
        let results = dir.path().join("results.jsonl");
        let harness = fail_hard_harness(dir.path(), &results);
        let output = std::process::Command::new("sh")
            .arg(&harness)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("w2 internal failure: boom"), "{stderr}");
        assert!(stderr.contains("w2 scenario log tail:"), "{stderr}");
        assert!(
            stderr.contains(&format!("prelude line {total:04}")),
            "{stderr}"
        );
        assert!(
            stderr.contains(&format!("prelude line {:04}", total - bound + 2)),
            "the first tailed line must be forwarded: {stderr}"
        );
        assert!(
            !stderr.contains(&format!("prelude line {:04}", total - bound + 1)),
            "lines outside the configured tail must never be forwarded: {stderr}"
        );
        assert!(!stderr.contains("prelude line 0001"), "{stderr}");
        let recorded = std::fs::read_to_string(&results).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(recorded.lines().next().unwrap()).unwrap();
        assert_eq!(first["id"], "internal");
        assert_eq!(first["pass"], false);
    }

    #[test]
    fn fail_hard_without_a_scenario_log_still_exits_one() {
        let dir = tempfile::tempdir().unwrap();
        let results = dir.path().join("results.jsonl");
        let harness = fail_hard_harness(dir.path(), &results);
        let output = std::process::Command::new("sh")
            .arg(&harness)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("w2 internal failure: boom"), "{stderr}");
        assert!(
            !stderr.contains("w2 scenario log tail:"),
            "an unavailable log must not forward a tail: {stderr}"
        );
        let recorded = std::fs::read_to_string(&results).unwrap();
        let first: serde_json::Value =
            serde_json::from_str(recorded.lines().next().unwrap()).unwrap();
        assert_eq!(first["id"], "internal");
    }

    #[test]
    fn scenario_bootstrap_check_fails_fast_before_any_phase_work() {
        let dir = tempfile::tempdir().unwrap();
        let harness = dir.path().join("harness.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\n{}\n{}\ncheck_workflow_id \"$1\"\n",
                function_text(SCENARIO, "bootstrap_fail"),
                function_text(SCENARIO, "check_workflow_id")
            ),
        )
        .unwrap();
        let run = |manifest: &std::path::Path, workflow_id: &str| {
            std::process::Command::new("sh")
                .arg(&harness)
                .arg(manifest)
                .env("WAVE2_WORKFLOW_ID", workflow_id)
                .output()
                .unwrap()
        };
        let missing = run(
            &dir.path().join("manifest.json"),
            "resident-wave2-apt-preferences",
        );
        assert!(!missing.status.success());
        assert!(
            String::from_utf8_lossy(&missing.stderr).contains("bootstrap failure"),
            "a missing manifest must take the early fatal path"
        );
        let wrong = dir.path().join("wrong.json");
        std::fs::write(
            &wrong,
            br#"{"workflow_id":"resident-wave2-package-contract","files":[]}"#,
        )
        .unwrap();
        let wrong = run(&wrong, "resident-wave2-apt-preferences");
        assert!(!wrong.status.success());
        assert!(
            String::from_utf8_lossy(&wrong.stderr).contains("bootstrap failure"),
            "a wrong workflow id must take the early fatal path"
        );
        let duplicate = dir.path().join("duplicate.json");
        std::fs::write(
            &duplicate,
            br#"{"workflow_id":"resident-wave2-apt-preferences","workflow_id":"resident-wave2-apt-preferences","files":[]}"#,
        )
        .unwrap();
        let duplicate = run(&duplicate, "resident-wave2-apt-preferences");
        assert!(!duplicate.status.success());
        assert!(
            String::from_utf8_lossy(&duplicate.stderr).contains("bootstrap failure"),
            "duplicate workflow ids must take the early fatal path"
        );
        let valid = dir.path().join("valid.json");
        std::fs::write(
            &valid,
            "{\n  \"schema\": 1,\n  \"workflow_id\": \"resident-wave2-apt-preferences\",\n  \"files\": []\n}\n",
        )
        .unwrap();
        assert!(
            run(&valid, "resident-wave2-apt-preferences")
                .status
                .success(),
            "the real pretty staged manifest shape must pass"
        );
    }

    #[test]
    fn stage_path_predicate_accepts_only_exact_template_names() {
        let (_dir, harness) = dir_harness(&[
            (
                "stage_path_owned",
                function_text(SCENARIO, "stage_path_owned"),
            ),
            ("", "stage_path_owned \"$1\"\n".to_string()),
        ]);
        for valid in [
            "/var/tmp/w2.abc123",
            "/var/tmp/w2c.ABC123",
            "/var/tmp/w2.0aB9zZ",
        ] {
            let output = std::process::Command::new("sh")
                .arg(&harness)
                .arg(valid)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "the template name {valid:?} must be owned"
            );
        }
        for invalid in [
            "/var/tmp/w2.abcdef/../../etc",
            "/var/tmp/w2.a/bcdef",
            "/var/tmp/w2.ab",
            "/var/tmp/w2.abcdefgh",
            "/var/tmp/w2.abcde",
            "/var/tmp/w2.abc\ndef",
            "/var/tmp/w2.abc123/",
            "/tmp/w2.abc123",
            "/var/tmp/other.abc123",
            "/var/tmp/w2c.ab",
            "",
        ] {
            let output = std::process::Command::new("sh")
                .arg(&harness)
                .arg(invalid)
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "the malicious value {invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn invalid_stage_values_never_reach_recursive_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rm-record");
        let harness = dir.path().join("guard-harness.sh");
        std::fs::write(
            &harness,
            format!(
                "#!/bin/sh\nset -u\nrm() {{ echo \"RM-CALLED:$*\" >> \"$RECORD\"; return 0; }}\n{}\n{}\nremove_owned_stage \"$1\"\n",
                function_text(SCENARIO, "stage_path_owned"),
                function_text(SCENARIO, "remove_owned_stage")
            ),
        )
        .unwrap();
        for invalid in [
            "/var/tmp/w2.abcdef/../../etc",
            "/var/tmp/w2.a/bcdef",
            "/var/tmp/w2.ab",
            "/var/tmp/w2.abcdefgh",
            "/var/tmp/w2.abc\ndef",
            "/var/tmp/w2.abc123/",
            "/tmp/w2.abc123",
            "/var/tmp/other.abc123",
            "",
        ] {
            let _ = std::fs::remove_file(&record);
            let output = std::process::Command::new("sh")
                .arg(&harness)
                .arg(invalid)
                .env("RECORD", &record)
                .output()
                .unwrap();
            assert!(
                !output.status.success(),
                "the guard must refuse {invalid:?}"
            );
            let calls = std::fs::read_to_string(&record).unwrap_or_default();
            assert!(
                !calls.contains("RM-CALLED"),
                "the deletion primitive must never run for {invalid:?}; calls: {calls}"
            );
        }
        let owned = format!("/var/tmp/w2.{:06}", std::process::id() % 1_000_000);
        std::fs::create_dir(&owned)
            .unwrap_or_else(|error| panic!("failed to create the template stage {owned}: {error}"));
        let real = dir.path().join("real-harness.sh");
        std::fs::write(
            &real,
            format!(
                "#!/bin/sh\nset -u\n{}\n{}\nremove_owned_stage \"$1\"\n",
                function_text(SCENARIO, "stage_path_owned"),
                function_text(SCENARIO, "remove_owned_stage")
            ),
        )
        .unwrap();
        let output = std::process::Command::new("sh")
            .arg(&real)
            .arg(&owned)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&owned);
        assert!(
            output.status.success(),
            "a real template-named directory must be removed by the guarded deletion"
        );
        assert!(
            !std::path::Path::new(&owned).exists(),
            "the guarded deletion must remove the exact validated path"
        );
    }

    fn dir_harness(sections: &[(&str, String)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("harness.sh");
        let mut text = String::from("#!/bin/sh\nset -u\n");
        for (_, body) in sections {
            text.push_str(body);
        }
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn extract_json_isolates_the_first_brace_object_from_console_noise() {
        let noisy = "cat /mnt/results.json\r\n{\"workflow\":\"x\",\"cases\":[]}\r\nQOL-RC-0-\r\n";
        assert_eq!(
            extract_json(noisy),
            Some("{\"workflow\":\"x\",\"cases\":[]}")
        );
        assert_eq!(extract_json("no braces here"), None);
    }

    #[test]
    fn wait_for_stick_checks_the_expected_device() {
        let command = wait_for_stick();
        assert!(command.contains("[ -b /dev/sda ]"), "command: {command}");
    }
}
