use crate::progress::{print_hint, print_title, run_cargo_step, run_step, step_label, StepKind};
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
    let build_identity =
        qol_dev_build::build_identity::BuildIdentityEnvironment::production(&root)?;
    build_identity.apply_to(&mut build);
    let artifacts = run_cargo_step(
        "build",
        StepKind::Pending,
        "release binaries",
        &mut build,
        verbose,
    )?;
    build_identity.verify_unchanged(&root)?;

    let installer = qol_dev_build::cargo_build::select_binary_executable(
        &artifacts,
        &root.join("apps/qol-tray/Cargo.toml"),
        qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
    )?;
    let tray = qol_dev_build::cargo_build::select_binary_executable(
        &artifacts,
        &root.join("apps/qol-tray/Cargo.toml"),
        qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
    )?;
    qol_artifact::verify_path(
        &installer,
        &qol_artifact::ArtifactExpectation::production(
            qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            qol_conventions::artifact::BuildRole::Installer,
        )
        .with_exact_source(build_identity.source()),
    )?;
    qol_artifact::verify_path(
        &tray,
        &qol_artifact::ArtifactExpectation::production(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            qol_conventions::artifact::BuildRole::Host,
        )
        .with_exact_source(build_identity.source()),
    )?;
    let installer_display = installer.display().to_string();
    let mut command = Command::new(installer);
    command
        .arg("--source")
        .arg(tray)
        .arg("--workspace")
        .arg(&root);
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
