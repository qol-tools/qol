use super::{check, fix_with_policy, FixPolicy, FixReport, Outcome, OutcomeStatus, Report};
use anyhow::{anyhow, Result};

pub(super) fn run_cli_from_env() -> Result<i32> {
    match command()? {
        DoctorCommand::Check => run_check(),
        DoctorCommand::Fix(policy) => run_fix(policy),
    }
}

enum DoctorCommand {
    Check,
    Fix(FixPolicy),
}

fn command() -> Result<DoctorCommand> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "check".to_string());
    let rest: Vec<String> = args.collect();

    match command.as_str() {
        "check" => {
            if !rest.is_empty() {
                return Err(unknown_args_error(&rest));
            }
            Ok(DoctorCommand::Check)
        }
        "fix" => Ok(DoctorCommand::Fix(parse_fix_flags(&rest)?)),
        _ => Err(anyhow!("Unknown command: {}", command)),
    }
}

fn parse_fix_flags(rest: &[String]) -> Result<FixPolicy> {
    let mut policy = FixPolicy::safe();
    for arg in rest {
        match arg.as_str() {
            "--apply-de-fixes" => policy.apply_de_fixes = true,
            other => return Err(anyhow!("Unknown flag: {}", other)),
        }
    }
    Ok(policy)
}

fn unknown_args_error(rest: &[String]) -> anyhow::Error {
    anyhow!(
        "Usage: qol-tray-doctor [check|fix [--apply-de-fixes]] (got extras: {})",
        rest.join(" ")
    )
}

fn run_check() -> Result<i32> {
    let report = check();
    print_report("Doctor Check", &report);
    Ok(exit_code_for_report(&report))
}

fn run_fix(policy: FixPolicy) -> Result<i32> {
    let report = fix_with_policy(policy);
    print_fix_report(&report);
    Ok(exit_code_for_report(&report.after))
}

fn print_fix_report(report: &FixReport) {
    print_report("Doctor Check (Before)", &report.before);
    println!();
    println!(
        "Fixes attempted={}, applied={}, skipped={}, failures={}",
        report.attempted,
        report.applied,
        report.skipped,
        report.failures.len()
    );
    print_failures(&report.failures);
    println!();
    print_report("Doctor Check (After)", &report.after);
}

fn print_failures(failures: &[String]) {
    for failure in failures {
        println!("[ERR] {}", failure);
    }
}

fn print_report(title: &str, report: &Report) {
    println!("{}", title);
    for outcome in &report.outcomes {
        print_outcome(outcome);
    }
    println!(
        "Summary: ok={}, warn={}, error={}",
        report.count_ok(),
        report.count_warn(),
        report.count_error()
    );
}

fn print_outcome(outcome: &Outcome) {
    println!(
        "[{}] {}: {}{}",
        status_label(outcome.status),
        outcome.id,
        outcome.message,
        fix_suffix(outcome.fix_available)
    );
}

fn status_label(status: OutcomeStatus) -> &'static str {
    match status {
        OutcomeStatus::Ok => "OK",
        OutcomeStatus::Warn => "WARN",
        OutcomeStatus::Error => "ERR",
    }
}

fn fix_suffix(fix_available: bool) -> &'static str {
    if fix_available {
        return " (fix available)";
    }
    ""
}

fn exit_code_for_report(report: &Report) -> i32 {
    if report.has_errors() {
        return 2;
    }
    if report.has_warnings() {
        return 1;
    }
    0
}
