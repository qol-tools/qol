use crate::cli::optional_single_arg;
use crate::workspace::{doctor_binary_path, repo_root};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(args: &[OsString], output_format: qol_headless::OutputFormat) -> Result<()> {
    let step = doctor_step(args)?;
    let root = repo_root()?;
    let binary = doctor_binary_path(&root);
    require_doctor_binary(&binary)?;
    let mut doctor = doctor_command(&root, &binary, step, output_format);

    let status = doctor
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub(crate) fn help_text() -> &'static str {
    "qol doctor\n\nRun read-only host and plugin health checks.\n\nUsage:\n  qol doctor\n  qol doctor <step>\n  qol --json doctor\n  qol doctor --json\n  qol --json doctor <step>\n  qol doctor <step> --json\n  qol doctor help\n  qol help doctor\n\nDetails:\n  Without a step, doctor aggregates host and installed-plugin checks.\n  A step selects one legacy host check and never runs repairs.\n  --json is a global output flag and may precede or follow the command path.\n\nOutput:\n  Plain text on stdout by default; diagnostics on stderr.\n  Aggregate JSON contains status, host, and plugins.\n  Legacy JSON contains the selected host-check report.\n\nExit:\n  Exits 0 when healthy, 1 for warnings, and 2 for failures.\n  Usage and execution failures exit non-zero before running checks.\n"
}

fn doctor_step(args: &[OsString]) -> Result<Option<&str>> {
    let step = optional_single_arg(args, "qol doctor [step]")?;
    let Some(step) = step else {
        return Ok(None);
    };
    if matches!(step, "help" | "-h" | "--help" | "--json") {
        bail!("`{step}` is a global qol CLI token, not a doctor check id");
    }
    if step.starts_with('-') {
        bail!("unknown qol doctor option `{step}`");
    }
    Ok(Some(step))
}

fn require_doctor_binary(binary: &Path) -> Result<()> {
    if binary.is_file() {
        return Ok(());
    }
    bail!(
        "doctor binary is unavailable at {}; run `qol build qol-tray` first",
        binary.display()
    )
}

fn doctor_command(
    root: &Path,
    binary: &Path,
    step: Option<&str>,
    output_format: qol_headless::OutputFormat,
) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(root);
    match step {
        Some(step) => {
            command
                .arg(qol_conventions::doctor_cli::ARG_CHECK)
                .arg(qol_conventions::doctor_cli::ARG_ID)
                .arg(step);
        }
        None => {
            command.arg("doctor");
        }
    }
    if output_format == qol_headless::OutputFormat::Json {
        command.arg(qol_conventions::doctor_cli::ARG_JSON);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::{doctor_command, doctor_step, require_doctor_binary};
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;

    #[test]
    fn doctor_command_runs_the_prebuilt_binary() {
        let root = Path::new("/repo/qol");
        let binary = Path::new("/repo/qol/target/debug/qol-tray-doctor");
        let cases = [
            (
                None,
                qol_headless::OutputFormat::PlainText,
                vec![OsStr::new("doctor")],
            ),
            (
                None,
                qol_headless::OutputFormat::Json,
                vec![
                    OsStr::new("doctor"),
                    OsStr::new(qol_conventions::doctor_cli::ARG_JSON),
                ],
            ),
            (
                Some("autostart_target"),
                qol_headless::OutputFormat::PlainText,
                vec![
                    OsStr::new(qol_conventions::doctor_cli::ARG_CHECK),
                    OsStr::new(qol_conventions::doctor_cli::ARG_ID),
                    OsStr::new("autostart_target"),
                ],
            ),
            (
                Some("autostart_target"),
                qol_headless::OutputFormat::Json,
                vec![
                    OsStr::new(qol_conventions::doctor_cli::ARG_CHECK),
                    OsStr::new(qol_conventions::doctor_cli::ARG_ID),
                    OsStr::new("autostart_target"),
                    OsStr::new(qol_conventions::doctor_cli::ARG_JSON),
                ],
            ),
        ];

        for (step, output_format, expected_args) in cases {
            let command = doctor_command(root, binary, step, output_format);
            assert_eq!(command.get_program(), binary.as_os_str(), "step={step:?}");
            assert_eq!(command.get_current_dir(), Some(root), "step={step:?}");
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                expected_args,
                "step={step:?}"
            );
        }
    }

    #[test]
    fn missing_doctor_binary_is_rejected_before_execution() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("qol-tray-doctor");

        let error = require_doctor_binary(&binary).unwrap_err();

        assert!(error.to_string().contains("qol build qol-tray"));
        fs::write(&binary, "").unwrap();
        require_doctor_binary(&binary).unwrap();
    }

    #[test]
    fn global_tokens_are_never_accepted_as_doctor_check_ids() {
        for token in ["help", "-h", "--help", "--json", "--fix"] {
            let error = doctor_step(&[token.into()]).unwrap_err();
            assert!(error.to_string().contains(token));
        }
    }
}
