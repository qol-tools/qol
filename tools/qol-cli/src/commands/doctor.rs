use crate::cli::optional_single_arg;
use crate::workspace::repo_root;
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let step = optional_single_arg(args, "qol doctor [step]")?;
    let root = repo_root()?;

    let mut build = Command::new("cargo");
    build.current_dir(&root).args([
        "build",
        "-p",
        "qol-tray",
        "--features",
        "dev",
        "--bin",
        "qol-tray-doctor",
    ]);
    if !verbose {
        build.arg("--quiet");
    }
    let built = build.status().context("failed to build qol-tray-doctor")?;
    if !built.success() {
        std::process::exit(built.code().unwrap_or(1));
    }

    let binary = root
        .join("target")
        .join("debug")
        .join(crate::host_facade::exe_name("qol-tray-doctor"));
    let mut doctor = Command::new(&binary);
    doctor.current_dir(&root).arg("check");
    if let Some(step) = step {
        doctor.arg("--id").arg(step);
    }

    let status = doctor
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}
