use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use qol_headless::{Command, CommandResult, DoctorCheck, DoctorCheckResult, HeadlessApp};
use qol_platform::Os;

use crate::nav::path;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const OPEN_COMMAND: &str = "open";
const SUPPORTED: [Os; 1] = [Os::Linux];

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with(crate::ui::open, dirs::home_dir)
}

fn app_with<Open>(open: Open, home: fn() -> Option<PathBuf>) -> HeadlessApp
where
    Open: Fn(PathBuf) -> Result<()> + Send + Sync + 'static,
{
    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Browse folders and pick files from the keyboard.")
        .default_command([OPEN_COMMAND])
        .command(
            Command::new(OPEN_COMMAND)
                .about("Open the Explorer window.")
                .usage(format!("{PLUGIN_ID} {OPEN_COMMAND} [path]"))
                .detail("Without a path the window opens on the home folder.")
                .detail("A path may start with ~ for the home folder.")
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the window cannot be opened.")
                .run_result(move |context| {
                    let start = match context.args() {
                        [] => home_folder(home)?,
                        [input] => path::resolve(
                            input,
                            &home_folder(home)?,
                            &std::env::current_dir().context("no current folder")?,
                        ),
                        [_, extra, ..] => {
                            return Ok(CommandResult::usage(format!(
                                "{PLUGIN_ID}: unexpected argument {extra:?}"
                            )))
                        }
                    };
                    open(start)?;
                    Ok(CommandResult::success(""))
                }),
        )
        .doctor_checks(doctor_checks())
}

fn home_folder(home: fn() -> Option<PathBuf>) -> Result<PathBuf> {
    home().ok_or_else(|| anyhow!("the home folder is not known"))
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by the plugin.",
            || Ok(platform_supported_result(Os::current())),
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify the plugin can run without persistent config.",
            || {
                Ok(DoctorCheckResult::ok(
                    "config_readable",
                    "No persistent config is required.",
                ))
            },
        ),
    ]
}

fn platform_supported_result(os: Option<Os>) -> DoctorCheckResult {
    let label = os.map_or_else(qol_platform::os::current_label, Os::label);
    if os.is_some_and(|os| SUPPORTED.contains(&os)) {
        return DoctorCheckResult::ok("platform_supported", format!("{label} is supported."));
    }
    DoctorCheckResult::fail(
        "platform_supported",
        format!("{label} is not declared by this plugin."),
    )
    .with_fix("Run Explorer on Linux.")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use qol_headless::{DoctorReport, DoctorStatus, Execution, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;

    fn test_home() -> Option<PathBuf> {
        Some(PathBuf::from("/explorer-test-home"))
    }

    fn run(args: &[&str]) -> (Execution, Vec<PathBuf>) {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&opened);
        let app = app_with(
            move |start| {
                sink.lock().unwrap().push(start);
                Ok(())
            },
            test_home,
        );
        let execution = app.execute(args.iter().map(|arg| (*arg).to_string()));
        let opened = opened.lock().unwrap().clone();
        (execution, opened)
    }

    #[test]
    fn open_starts_on_home_or_the_given_path() {
        let cases: [(&[&str], PathBuf); 4] = [
            (&[], PathBuf::from("/explorer-test-home")),
            (&["open"], PathBuf::from("/explorer-test-home")),
            (
                &["open", "~/Git"],
                Path::new("/explorer-test-home").join("Git"),
            ),
            (&["open", "/srv/data"], PathBuf::from("/srv/data")),
        ];
        for (args, expected) in cases {
            let (execution, opened) = run(args);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert_eq!(opened, vec![expected], "args: {args:?}");
        }
    }

    #[test]
    fn open_rejects_a_second_path() {
        let (execution, opened) = run(&["open", "/a", "/b"]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("unexpected argument \"/b\""));
        assert!(opened.is_empty());
    }

    #[test]
    fn help_first_and_final_are_equivalent() {
        let (first, _) = run(&["help", "open"]);
        let (final_token, _) = run(&["open", "help"]);
        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Does not support --json."));
    }

    #[test]
    fn doctor_and_help_never_open_the_window() {
        let cases: [&[&str]; 6] = [
            &["doctor"],
            &["--json", "doctor"],
            &["doctor", "--json"],
            &["help"],
            &["help", "open"],
            &["open", "help"],
        ];
        for args in cases {
            let (execution, opened) = run(args);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args: {args:?}");
            assert!(opened.is_empty(), "args: {args:?}");
        }
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_flag_positions() {
        let (before, _) = run(&["--json", "doctor"]);
        let (after, _) = run(&["doctor", "--json"]);
        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 2);
    }

    #[test]
    fn only_declared_platforms_are_supported() {
        let cases = [
            (Some(Os::Linux), DoctorStatus::Ok),
            (Some(Os::Macos), DoctorStatus::Fail),
            (Some(Os::Windows), DoctorStatus::Fail),
            (None, DoctorStatus::Fail),
        ];
        for (os, status) in cases {
            let result = platform_supported_result(os);
            assert_eq!(result.status, status, "os: {os:?}");
            assert_eq!(
                result.fix.is_some(),
                status == DoctorStatus::Fail,
                "os: {os:?}"
            );
        }
    }
}
