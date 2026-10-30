use std::process::ExitCode;

use anyhow::{Context, Result};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, Execution, HeadlessApp,
};

use crate::updates;
use crate::PLUGIN_ID;

const PLATFORM_SUPPORTED: &str = "platform_supported";
const REQUIRED_BINARIES: &str = "required_binaries";
const REQUIRED: [&str; 4] = ["apt-get", "apt-cache", "pkexec", "systemctl"];
const OPTIONAL: [&str; 2] = ["flatpak", "fwupdmgr"];

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("See what updates wait, install them when you choose, and restart when it suits you.")
        .command(
            Command::new("status")
                .about("List the updates that wait, what finishes at a restart, and what is skipped.")
                .usage(format!("{PLUGIN_ID} status"))
                .output("One line per waiting update, then what a restart finishes, then the skipped entries.")
                .exit_behavior("Exits 64 on a usage error; 0 otherwise, with read problems on stderr.")
                .run_result(status),
        )
        .command(
            Command::new("press")
                .about("Do what a row on an Update Manager page does.")
                .usage(format!("{PLUGIN_ID} press"))
                .detail("The running daemon answers presses from the settings panel; this command only says so.")
                .exit_behavior("Always exits non-zero: a press needs the daemon.")
                .run_result(press),
        )
        .command(
            Command::new("settings")
                .about("Open the Update Manager settings.")
                .usage(format!("{PLUGIN_ID} settings"))
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the settings cannot be opened.")
                .run_result(settings),
        )
        .doctor_checks(checks())
}

fn no_args(context: &CommandContext, usage: &str) -> Option<Execution> {
    context.args().first().map(|token| {
        let problem = if token.starts_with('-') {
            format!("unknown flag `{token}`")
        } else {
            format!("unexpected argument `{token}`")
        };
        Execution::usage(format!("{problem}\n{usage}"))
    })
}

fn status(context: &CommandContext) -> Result<Execution> {
    if let Some(usage) = no_args(context, &format!("{PLUGIN_ID} status")) {
        return Ok(usage);
    }
    let found = updates::read_updates();
    let host = updates::read_host();
    let list = updates::visible(&found.list, &host.skips);
    let mut out = String::new();
    let total: usize = list.iter().map(|update| update.count()).sum();
    out.push_str(&format!("{total} waiting\n"));
    for update in &list {
        out.push_str(&format!(
            "  {} [{}]: {}\n",
            update.label,
            update.tag,
            update.wait()
        ));
    }
    for row in &host.finish {
        out.push_str(&format!("Finishes at a restart: {}\n", row.label));
    }
    if !host.skips.is_empty() {
        out.push_str(&format!("Skipped: {}\n", host.skips.join(", ")));
    }
    let problems: Vec<String> = found.problems.into_iter().chain(host.problems).collect();
    Ok(Execution::new(out, problems.join("\n"), 0))
}

fn press(_context: &CommandContext) -> Result<Execution> {
    Ok(Execution::runtime_error(
        "press is answered by the running Update Manager daemon; open its settings from qol-tray",
    ))
}

fn settings(context: &CommandContext) -> Result<Execution> {
    if let Some(usage) = no_args(context, &format!("{PLUGIN_ID} settings")) {
        return Ok(usage);
    }
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open the Update Manager settings")?;
    Ok(Execution::success(String::new()))
}

fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            PLATFORM_SUPPORTED,
            "Answer whether Update Manager can read this computer's updaters.",
            || Ok(platform_supported()),
        ),
        DoctorCheck::new(
            REQUIRED_BINARIES,
            "Answer whether the programs Update Manager runs are installed.",
            || Ok(required_binaries()),
        ),
    ]
}

fn platform_supported() -> DoctorCheckResult {
    match updates::supported() {
        Err(problem) => DoctorCheckResult::fail(PLATFORM_SUPPORTED, problem),
        Ok(()) if updates::is_mint() => DoctorCheckResult::ok(
            PLATFORM_SUPPORTED,
            "Reads APT, Flatpak, firmware and Mint's automatic upgrades",
        ),
        Ok(()) => DoctorCheckResult::warn(
            PLATFORM_SUPPORTED,
            "Mint's Update Manager is not installed: updates list, and nothing installs on its own",
        ),
    }
}

fn required_binaries() -> DoctorCheckResult {
    let required = updates::missing(&REQUIRED);
    if !required.is_empty() {
        return DoctorCheckResult::fail(
            REQUIRED_BINARIES,
            format!("{} missing, so nothing installs", required.join(", ")),
        )
        .with_fix(format!("Install {}", required.join(" and ")));
    }
    let optional = updates::missing(&OPTIONAL);
    if !optional.is_empty() {
        return DoctorCheckResult::warn(
            REQUIRED_BINARIES,
            format!(
                "{} missing, so those updates are not listed",
                optional.join(", ")
            ),
        );
    }
    DoctorCheckResult::ok(REQUIRED_BINARIES, "Every program it runs is installed")
}

#[cfg(test)]
mod tests {
    use qol_headless::{DoctorStatus, EXIT_USAGE};

    use super::*;

    fn run(args: &[&str]) -> u8 {
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        let code = app().run(args);
        if code == ExitCode::SUCCESS {
            0
        } else if code == ExitCode::from(EXIT_USAGE) {
            EXIT_USAGE
        } else {
            1
        }
    }

    #[test]
    fn help_and_usage_errors_follow_the_headless_contract() {
        assert_eq!(run(&["help"]), 0);
        assert_eq!(run(&["help", "status"]), 0);
        assert_eq!(run(&["settings", "help"]), 0);
        assert_eq!(run(&["status", "--all"]), EXIT_USAGE);
        assert_eq!(run(&["press"]), 1);
    }

    #[test]
    fn checks_offer_a_fix_only_when_they_fail() {
        for result in [platform_supported(), required_binaries()] {
            if result.status == DoctorStatus::Ok {
                assert!(result.fix.is_none(), "{} must not offer a fix", result.id);
            }
        }
    }
}
