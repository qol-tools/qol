use crate::host_facade;
use crate::progress::{print_hint, print_title, run_step, step_label, StepKind};
use crate::workspace::{repo_root, scan_buildable_plugins};
use anyhow::Result;
use std::process::Command;

pub(crate) fn run(verbose: bool) -> Result<()> {
    let root = repo_root()?;
    print_title("qol install");
    print_hint(verbose);

    let scan = scan_buildable_plugins(&root)?;
    step_label(
        "plugins",
        StepKind::Info,
        &format!(
            "{} buildable, {} unsupported here",
            scan.buildable.len(),
            scan.skipped_host
        ),
    );

    let mut build = Command::new("cargo");
    build
        .current_dir(&root)
        .args(["build", "--release", "-p", "qol-tray"]);
    for plugin in &scan.buildable {
        build.arg("-p").arg(&plugin.package_name);
    }
    run_step(
        "build",
        StepKind::Pending,
        "release binaries",
        &mut build,
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
