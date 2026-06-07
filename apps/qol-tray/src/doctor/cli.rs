use super::{
    check, check_single, fix_single_with_policy, fix_with_policy, FixPolicy, FixReport, Outcome,
    OutcomeStatus, Report,
};
use anyhow::{anyhow, Result};

pub(super) fn run_cli_from_env() -> Result<i32> {
    match command()? {
        DoctorCommand::Check { id } => run_check(id),
        DoctorCommand::Fix { id, policy } => run_fix(id, policy),
    }
}

enum DoctorCommand {
    Check {
        id: Option<String>,
    },
    Fix {
        id: Option<String>,
        policy: FixPolicy,
    },
}

fn command() -> Result<DoctorCommand> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "check".to_string());
    let rest: Vec<String> = args.collect();

    match command.as_str() {
        "check" => Ok(DoctorCommand::Check {
            id: parse_check_flags(&rest)?,
        }),
        "fix" => {
            let (id, policy) = parse_fix_flags(&rest)?;
            Ok(DoctorCommand::Fix { id, policy })
        }
        _ => Err(anyhow!("Unknown command: {}", command)),
    }
}

fn parse_check_flags(rest: &[String]) -> Result<Option<String>> {
    let mut id = None;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--id" => id = Some(take_id_value(&mut args)?),
            _ => return Err(usage_error("check [--id <CHECK_ID>]", rest)),
        }
    }
    Ok(id)
}

fn parse_fix_flags(rest: &[String]) -> Result<(Option<String>, FixPolicy)> {
    let mut id = None;
    let mut host_fixes = false;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--id" => id = Some(take_id_value(&mut args)?),
            "--apply-host-fixes" | "--apply-de-fixes" => host_fixes = true,
            _ => {
                return Err(usage_error(
                    "fix [--id <CHECK_ID>] [--apply-host-fixes]",
                    rest,
                ))
            }
        }
    }
    let policy = if host_fixes {
        FixPolicy::startup()
    } else {
        FixPolicy::safe()
    };
    Ok((id, policy))
}

fn take_id_value<'a>(args: &mut impl Iterator<Item = &'a String>) -> Result<String> {
    args.next()
        .cloned()
        .ok_or_else(|| anyhow!("--id requires a check id"))
}

fn usage_error(usage: &str, rest: &[String]) -> anyhow::Error {
    anyhow!("Usage: qol-tray-doctor {usage} (got: {})", rest.join(" "))
}

fn run_check(id: Option<String>) -> Result<i32> {
    let report = match id {
        Some(id) => check_single(&id),
        None => check(),
    };
    print_report("Doctor Check", &report);
    Ok(exit_code_for_report(&report))
}

fn run_fix(id: Option<String>, policy: FixPolicy) -> Result<i32> {
    let report = match id {
        Some(id) => fix_single_with_policy(&id, policy),
        None => fix_with_policy(policy),
    };
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
    for outcome in report.outcomes() {
        print_outcome(outcome);
    }
    println!(
        "Summary: ok={}, warn={}, error={}, crash={}",
        report.count_ok(),
        report.count_warn(),
        report.count_error(),
        report.count_crash()
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
        OutcomeStatus::Crash => "CRASH",
    }
}

fn fix_suffix(fix_available: bool) -> &'static str {
    if fix_available {
        return " (fix available)";
    }
    ""
}

fn exit_code_for_report(report: &Report) -> i32 {
    if report.has_crashes() {
        return 2;
    }
    if report.has_errors() {
        return 2;
    }
    if report.has_warnings() {
        return 1;
    }
    0
}
