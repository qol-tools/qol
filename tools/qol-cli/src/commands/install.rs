use crate::host_facade;
use crate::progress::{print_hint, print_title, run_step, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::Result;
use std::process::Command;

pub(crate) fn run(verbose: bool) -> Result<()> {
    let root = repo_root()?;
    print_title("qol install");
    print_hint(verbose);
    run_step(
        "build",
        StepKind::Pending,
        "release binaries",
        Command::new("cargo")
            .current_dir(&root)
            .args(["build", "--release", "--bins"]),
        verbose,
    )?;
    let installer = root
        .join("target")
        .join("release")
        .join(host_facade::exe_name("qol-tray-install"));
    let installer_display = installer.display().to_string();
    let mut command = Command::new(installer);
    run_step(
        "install",
        StepKind::Pending,
        &installer_display,
        &mut command,
        verbose,
    )?;
    step_label("ready", StepKind::Success, "qol-tray");
    Ok(())
}
