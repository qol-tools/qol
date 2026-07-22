use std::process::ExitCode;

use anyhow::Result;
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};

use crate::{app, PLUGIN_ID};

const BINARY_NAME: &str = "plugin-controllers";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Inspect connected game controllers and apply relevant driver-specific fixes.")
        .default_command(["status"])
        .command(apply_command())
        .command(status_command())
        .command(settings_command())
        .doctor_checks(doctor_checks())
}

fn apply_command() -> Command {
    Command::new("apply_fixes")
        .about("Apply fixes that match each controller's currently bound driver.")
        .usage(format!("{BINARY_NAME} apply_fixes"))
        .detail("The current compatibility fix applies only when xpadneo is active.")
        .detail("xpadneo is optional and is never installed by this command.")
        .detail("Writes /etc/modprobe.d/qol-controllers.conf and the live sysfs quirk.")
        .detail("Runs one pkexec authorization prompt.")
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if no driver-specific fix applies or pkexec fails.")
        .run_plain_text(|_| {
            app::execute_action_once("apply_fixes")?;
            Ok(PlainTextOutput::empty())
        })
}

fn status_command() -> Command {
    Command::new("status")
        .about("Print connected controllers and their verdicts.")
        .usage(format!("{BINARY_NAME} status"))
        .output("One line per connected controller.")
        .exit_behavior("Exits zero even when no controller is connected.")
        .run_plain_text(|_| {
            let snapshot = app::snapshot();
            if snapshot.rows.is_empty() {
                return Ok(PlainTextOutput::text("no controllers detected"));
            }
            Ok(PlainTextOutput::text(summary_lines(&snapshot.rows)))
        })
        .run_json(|_| {
            let snapshot = app::snapshot();
            Ok(app::snapshot_payload(&snapshot))
        })
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings page.")
        .usage(format!("{BINARY_NAME} settings"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if the settings URL cannot be opened.")
        .run_plain_text(|_| {
            crate::platform::open_settings()?;
            Ok(PlainTextOutput::empty())
        })
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "pkexec_available",
            "Verify pkexec exists for the privileged apply step.",
            pkexec_check,
        ),
        DoctorCheck::new(
            "controller_fixes",
            "Verify connected known controllers have their fixes applied.",
            fixes_check,
        ),
    ]
}

fn pkexec_check() -> Result<DoctorCheckResult> {
    let snapshot = app::snapshot();
    if !snapshot.rows.iter().any(|row| row.fixable) {
        return Ok(DoctorCheckResult::ok(
            "pkexec_available",
            "no privileged controller fixes currently apply",
        ));
    }
    let found = std::process::Command::new("pkexec")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    Ok(if found {
        DoctorCheckResult::ok("pkexec_available", "pkexec found")
    } else {
        DoctorCheckResult::warn("pkexec_available", "pkexec not found")
            .with_fix("install polkit (provides pkexec)")
    })
}

fn fixes_check() -> Result<DoctorCheckResult> {
    let snapshot = app::snapshot();
    if snapshot.rows.is_empty() {
        return Ok(DoctorCheckResult::ok(
            "controller_fixes",
            format!(
                "no controllers detected ({} fixes in database)",
                crate::fixes::FIXES.len()
            ),
        ));
    }
    let summary = summary_lines(&snapshot.rows);
    if snapshot.rows.iter().any(|row| row.fixable) {
        return Ok(DoctorCheckResult::warn("controller_fixes", summary)
            .with_fix(format!("run: {BINARY_NAME} apply_fixes")));
    }
    Ok(DoctorCheckResult::ok("controller_fixes", summary))
}

fn summary_lines(rows: &[app::ControllerRow]) -> String {
    rows.iter()
        .map(|row| {
            format!(
                "{} [{}; {}]: {}",
                row.name, row.transport, row.driver, row.verdict
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
