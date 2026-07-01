use crate::cli::optional_single_arg;
use crate::workspace::{cargo_build_command, doctor_binary_path, repo_root, DOCTOR_BUILD_ARGS};
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let step = optional_single_arg(args, "qol doctor [step]")?;
    let root = repo_root()?;

    let mut build = cargo_build_command(&root, &DOCTOR_BUILD_ARGS);
    if !verbose {
        build.arg("--quiet");
    }
    let built = build.status().context("failed to build qol-tray-doctor")?;
    if !built.success() {
        std::process::exit(built.code().unwrap_or(1));
    }

    let binary = doctor_binary_path(&root);
    let mut doctor = Command::new(&binary);
    doctor
        .current_dir(&root)
        .arg(qol_conventions::doctor_cli::ARG_CHECK);
    if let Some(step) = step {
        doctor.arg(qol_conventions::doctor_cli::ARG_ID).arg(step);
    }

    let status = doctor
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
