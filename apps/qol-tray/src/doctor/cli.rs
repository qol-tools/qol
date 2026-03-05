use super::{check, fix_safe, FixReport, Outcome, OutcomeStatus, Report};
use anyhow::{anyhow, Result};

pub(super) fn run_cli_from_env() -> Result<i32> {
    match command()? {
        DoctorCommand::Check => run_check(),
        DoctorCommand::Fix => run_fix(),
    }
}

enum DoctorCommand {
    Check,
    Fix,
}

fn command() -> Result<DoctorCommand> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "check".to_string());
    if args.next().is_some() {
        return Err(anyhow!("Usage: qol-tray-doctor [check|fix]"));
    }

    match command.as_str() {
        "check" => Ok(DoctorCommand::Check),
        "fix" => Ok(DoctorCommand::Fix),
        _ => Err(anyhow!("Unknown command: {}", command)),
    }
}

fn run_check() -> Result<i32> {
    let report = check();
    print_report("Doctor Check", &report);
    Ok(exit_code_for_report(&report))
}

fn run_fix() -> Result<i32> {
    let report = fix_safe();
    print_fix_report(&report);
    Ok(exit_code_for_report(&report.after))
}

fn print_fix_report(report: &FixReport) {
    print_report("Doctor Check (Before)", &report.before);
    println!();
    println!(
        "Fixes attempted={}, applied={}, failures={}",
        report.attempted,
        report.applied,
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
