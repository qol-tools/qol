mod checks;
mod cli;
mod diagnosis;
mod install_id;
mod platform;
mod report;

use anyhow::Result;
use diagnosis::{apply_fix, Diagnosis};
pub use report::{FixReport, Outcome, OutcomeStatus, Report};

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
    let diagnoses = checks::collect_diagnoses();
    let before = report_from_diagnoses(&diagnoses);
    let summary = apply_fixes(diagnoses);
    let after = check();
    FixReport {
        before,
        after,
        attempted: summary.attempted,
        applied: summary.applied,
        failures: summary.failures,
    }
}

pub fn auto_fix_startup() -> FixReport {
    let report = fix_safe();
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

fn apply_fixes(diagnoses: Vec<Diagnosis>) -> FixSummary {
    let mut summary = FixSummary::default();
    for diagnosis in diagnoses {
        apply_diagnosis_fix(&mut summary, diagnosis);
    }
    summary
}

fn apply_diagnosis_fix(summary: &mut FixSummary, diagnosis: Diagnosis) {
    let Some(action) = diagnosis.fix else {
        return;
    };
    summary.attempted += 1;
    if let Err(error) = apply_fix(&action) {
        summary
            .failures
            .push(format!("{}: {}", diagnosis.outcome.id, error));
        return;
    }
    summary.applied += 1;
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
    failures: Vec<String>,
}
