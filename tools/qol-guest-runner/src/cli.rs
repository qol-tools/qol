use anyhow::{bail, Context, Result};
use qol_dev_guest::{DEFAULT_DEVICE_PATH, DEFAULT_IDENTITY_PATH, DEFAULT_RUN_ID_PATH};
use qol_headless::{Command, DoctorCheck, HeadlessApp, PlainTextOutput};
use std::path::PathBuf;
use std::process::ExitCode;

const BINARY_NAME: &str = "qol-guest-runner";

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RunOptions {
    pub(crate) device_path: PathBuf,
    pub(crate) identity_path: PathBuf,
    pub(crate) run_id_path: PathBuf,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            device_path: PathBuf::from(DEFAULT_DEVICE_PATH),
            identity_path: PathBuf::from(DEFAULT_IDENTITY_PATH),
            run_id_path: PathBuf::from(DEFAULT_RUN_ID_PATH),
        }
    }
}

impl RunOptions {
    fn parse(args: &[String]) -> Result<Self> {
        let mut options = Self::default();
        let mut device_path = None;
        let mut identity_path = None;
        let mut run_id_path = None;
        let mut args = args.iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--device" => set_once(
                    &mut device_path,
                    PathBuf::from(args.next().context("--device needs a path")?),
                    "--device",
                )?,
                "--identity" => set_once(
                    &mut identity_path,
                    PathBuf::from(args.next().context("--identity needs a path")?),
                    "--identity",
                )?,
                "--run-id-path" => set_once(
                    &mut run_id_path,
                    PathBuf::from(args.next().context("--run-id-path needs a path")?),
                    "--run-id-path",
                )?,
                other => bail!("unknown run argument `{other}`"),
            }
        }
        if let Some(path) = device_path {
            options.device_path = path;
        }
        if let Some(path) = identity_path {
            options.identity_path = path;
        }
        if let Some(path) = run_id_path {
            options.run_id_path = path;
        }
        Ok(options)
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("{option} was provided more than once");
    }
    Ok(())
}

pub(crate) fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(BINARY_NAME, BINARY_NAME)
        .about("Serve the guest-control channel inside a prepared disposable qol guest.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .about("Serve guest-control requests for the current graphical session.")
                .usage(
                    "qol-guest-runner run [--device PATH] [--identity PATH] [--run-id-path PATH]",
                )
                .detail("With no command, `run` is selected for service compatibility.")
                .output("Runtime diagnostics are written to stderr.")
                .exit_behavior("Runs until stopped; exits non-zero when guest setup is invalid.")
                .run_plain_text(|context| {
                    crate::platform::run(RunOptions::parse(context.args())?)?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .doctor_checks([
            DoctorCheck::new(
                "platform_supported",
                "Verify this host can run the guest-control service.",
                || Ok(crate::platform::platform_check()),
            ),
            DoctorCheck::new(
                "runtime_paths",
                "Verify the prepared guest-control paths are available.",
                || Ok(crate::platform::runtime_paths_check()),
            ),
        ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_headless::DoctorReport;

    #[test]
    fn parses_paths_once() {
        assert_eq!(
            RunOptions::parse(&[
                "--device".to_string(),
                "/tmp/device".to_string(),
                "--identity".to_string(),
                "/tmp/identity".to_string(),
            ])
            .unwrap(),
            RunOptions {
                device_path: PathBuf::from("/tmp/device"),
                identity_path: PathBuf::from("/tmp/identity"),
                run_id_path: PathBuf::from(DEFAULT_RUN_ID_PATH),
            }
        );
        assert!(RunOptions::parse(&[
            "--device".to_string(),
            "a".to_string(),
            "--device".to_string(),
            "b".to_string(),
        ])
        .is_err());
    }

    #[test]
    fn contextual_help_is_available_in_both_positions() {
        for args in [["help", "run"], ["run", "help"]] {
            let output = app().execute(args.map(str::to_string));
            assert_eq!(output.exit_code, 0, "{}", output.stderr);
            assert!(output.stdout.contains("--device PATH"));
        }
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_positions() {
        for args in [["--json", "doctor"], ["doctor", "--json"]] {
            let output = app().execute(args.map(str::to_string));
            assert!(matches!(output.exit_code, 0..=2), "{}", output.stderr);
            let report: DoctorReport = serde_json::from_str(&output.stdout).unwrap();
            assert_eq!(report.plugin_id, BINARY_NAME);
            assert_eq!(report.checks.len(), 2);
        }
    }

    #[test]
    fn json_is_rejected_before_run_dispatch() {
        let output = app().execute(["--json", "run"].map(str::to_string));
        assert_eq!(output.exit_code, qol_headless::EXIT_USAGE);
        assert!(output.stderr.contains("does not support --json"));
    }
}
