use super::{
    check, check_quick, check_single, fix_single_with_policy, fix_with_policy, FixPolicy,
    FixReport, Outcome, OutcomeStatus, Report,
};
use anyhow::{anyhow, Result};
use qol_conventions::doctor_cli::{ARG_CHECK, ARG_FIX, ARG_ID, ARG_QUICK};

pub(super) fn run_cli_from_env() -> Result<i32> {
    match command()? {
        DoctorCommand::Check { selection } => run_check(selection),
        DoctorCommand::Fix { id, policy } => run_fix(id, policy),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CheckSelection {
    All,
    Quick,
    Id(String),
}

enum DoctorCommand {
    Check {
        selection: CheckSelection,
    },
    Fix {
        id: Option<String>,
        policy: FixPolicy,
    },
}

fn command() -> Result<DoctorCommand> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| ARG_CHECK.to_string());
    let rest: Vec<String> = args.collect();

    match command.as_str() {
        ARG_CHECK => Ok(DoctorCommand::Check {
            selection: parse_check_flags(&rest)?,
        }),
        ARG_FIX => {
            let (id, policy) = parse_fix_flags(&rest)?;
            Ok(DoctorCommand::Fix { id, policy })
        }
        _ => Err(anyhow!("Unknown command: {}", command)),
    }
}

fn parse_check_flags(rest: &[String]) -> Result<CheckSelection> {
    let mut id = None;
    let mut quick = false;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            ARG_ID => id = Some(take_id_value(&mut args)?),
            ARG_QUICK => quick = true,
            _ => return Err(usage_error("check [--id <CHECK_ID>] [--quick]", rest)),
        }
    }
    match (id, quick) {
        (Some(_), true) => Err(anyhow!(
            "qol-tray-doctor check accepts either --id or --quick, not both"
        )),
        (Some(id), false) => Ok(CheckSelection::Id(id)),
        (None, true) => Ok(CheckSelection::Quick),
        (None, false) => Ok(CheckSelection::All),
    }
}

fn parse_fix_flags(rest: &[String]) -> Result<(Option<String>, FixPolicy)> {
    let mut id = None;
    let mut host_fixes = false;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            ARG_ID => id = Some(take_id_value(&mut args)?),
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
        FixPolicy::with_host_fixes()
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

fn run_check(selection: CheckSelection) -> Result<i32> {
    let report = match selection {
        CheckSelection::All => check(),
        CheckSelection::Quick => check_quick(),
        CheckSelection::Id(id) => check_single(&id),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parse_check_flags_selects_quick_scope() {
        assert_eq!(
            parse_check_flags(&args(&["--quick"])).unwrap(),
            CheckSelection::Quick
        );
    }

    #[test]
    fn parse_check_flags_rejects_ambiguous_scope() {
        let error = parse_check_flags(&args(&["--quick", "--id", "install_identity"]))
            .expect_err("--quick and --id select different check sets");
        assert!(
            error.to_string().contains("either --id or --quick"),
            "unexpected error: {error:#}"
        );
    }
}
