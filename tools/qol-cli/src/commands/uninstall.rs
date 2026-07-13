use crate::host_facade;
use crate::progress::{print_hint, print_title, run_step, StepKind};
use crate::workspace::repo_root;
use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::process::{Command, Stdio};

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let root = repo_root()?;
    let json = args.iter().any(|arg| arg == "--json");
    if !json {
        print_title("qol uninstall");
        print_hint(verbose);
    }
    build_installer(&root, verbose, json)?;
    let installer = root
        .join("target")
        .join("release")
        .join(host_facade::exe_name("qol-tray-install"));
    let mut command = Command::new(&installer);
    command.arg("--uninstall").args(args);
    let status = command
        .status()
        .with_context(|| format!("failed to start {}", installer.display()))?;
    if !status.success() {
        bail!("uninstaller exited with {status}");
    }
    Ok(())
}

fn build_installer(root: &std::path::Path, verbose: bool, json: bool) -> Result<()> {
    let mut build = Command::new("cargo");
    build.current_dir(root).args([
        "build",
        "--release",
        "-p",
        "qol-tray",
        "--bin",
        "qol-tray-install",
    ]);
    if json {
        build.stdout(Stdio::null());
        let status = build.status().context("failed to build qol-tray-install")?;
        if !status.success() {
            bail!("failed to build qol-tray-install: {status}");
        }
        return Ok(());
    }
    run_step(
        "build",
        StepKind::Pending,
        "qol-tray-install",
        &mut build,
        verbose,
    )
}
