use crate::cli::optional_single_arg;
use crate::workspace::{doctor_binary_path, repo_root};
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    let step = optional_single_arg(args, "qol doctor [step]")?;
    let root = repo_root()?;
    let binary = doctor_binary_path(&root);
    require_doctor_binary(&binary)?;
    let mut doctor = doctor_command(&root, &binary, step);

    let status = doctor
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
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

fn doctor_command(root: &Path, binary: &Path, step: Option<&str>) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .arg(qol_conventions::doctor_cli::ARG_CHECK);
    if let Some(step) = step {
        command.arg(qol_conventions::doctor_cli::ARG_ID).arg(step);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::{doctor_command, require_doctor_binary};
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
                vec![OsStr::new(qol_conventions::doctor_cli::ARG_CHECK)],
            ),
            (
                Some("autostart_target"),
                vec![
                    OsStr::new(qol_conventions::doctor_cli::ARG_CHECK),
                    OsStr::new(qol_conventions::doctor_cli::ARG_ID),
                    OsStr::new("autostart_target"),
                ],
            ),
        ];

        for (step, expected_args) in cases {
            let command = doctor_command(root, binary, step);
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
}
