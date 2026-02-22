use anyhow::{anyhow, Result};
use qol_tray::doctor::{self, FixReport, OutcomeStatus, Report};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "check".to_string());
    if args.next().is_some() {
        return Err(anyhow!("Usage: qol-tray-doctor [check|fix]"));
    }

    match command.as_str() {
        "check" => {
            let report = doctor::check();
            print_report("Doctor Check", &report);
            std::process::exit(exit_code_for_report(&report));
        }
        "fix" => {
            let report = doctor::fix_safe();
            print_fix_report(&report);
            std::process::exit(exit_code_for_report(&report.after));
        }
        _ => Err(anyhow!("Unknown command: {}", command)),
    }
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
    for failure in &report.failures {
        println!("[ERR] {}", failure);
    }
    println!();
    print_report("Doctor Check (After)", &report.after);
}

fn print_report(title: &str, report: &Report) {
    println!("{}", title);
    for outcome in &report.outcomes {
        let status = match outcome.status {
            OutcomeStatus::Ok => "OK",
            OutcomeStatus::Warn => "WARN",
            OutcomeStatus::Error => "ERR",
        };
        let fix_suffix = if outcome.fix_available {
            " (fix available)"
        } else {
            ""
        };
        println!(
            "[{}] {}: {}{}",
            status, outcome.id, outcome.message, fix_suffix
        );
    }
    println!(
        "Summary: ok={}, warn={}, error={}",
        report.count_ok(),
        report.count_warn(),
        report.count_error()
    );
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
