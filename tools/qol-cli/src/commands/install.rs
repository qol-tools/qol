use crate::progress::{print_hint, print_title, run_cargo_step, run_step, step_label, StepKind};
use crate::workspace::{
    qualified_plugin_build_features, repo_root, scan_buildable_plugins, BuildablePlugin,
};
use anyhow::{bail, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let dev = parse_dev_flag(args)?;
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

    let mut build = build_command(&root, &scan.buildable, dev)?;
    let build_identity = install_build_identity(&root, dev)?;
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
    let installer_expectation =
        installer_expectation(dev).with_exact_source(build_identity.source());
    qol_artifact::verify_path(&installer, &installer_expectation)?;

    let tray = if dev {
        None
    } else {
        let tray = qol_dev_build::cargo_build::select_binary_executable(
            &artifacts,
            &root.join("apps/qol-tray/Cargo.toml"),
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
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
        Some(tray)
    };

    let installer_display = installer.display().to_string();
    let mut command = install_command(&installer, &root, tray.as_deref(), dev);
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

fn parse_dev_flag(args: &[OsString]) -> Result<bool> {
    let mut dev = false;
    for arg in args {
        match arg.to_str() {
            Some("--dev") => dev = true,
            Some(other) => {
                bail!("unknown argument `{other}` for qol install (only --dev is supported)")
            }
            None => bail!("non-UTF-8 argument for qol install"),
        }
    }
    Ok(dev)
}

fn build_command(root: &Path, buildable: &[BuildablePlugin], dev: bool) -> Result<Command> {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .args(["build", "--release", "--locked", "-p", "qol-tray"]);
    if dev {
        command.args(["--features", "dev,linux_evdev"]);
    } else {
        command.args(["--features", "linux_evdev"]);
    }
    for plugin in buildable {
        command.arg("-p").arg(&plugin.package_name);
        for feature in qualified_plugin_build_features(&plugin.dir)? {
            command.arg("--features").arg(feature);
        }
    }
    Ok(command)
}

fn install_build_identity(
    root: &Path,
    dev: bool,
) -> Result<qol_build_identity::BuildIdentityEnvironment> {
    if dev {
        Ok(qol_build_identity::BuildIdentityEnvironment::development(
            root,
        )?)
    } else {
        Ok(qol_build_identity::BuildIdentityEnvironment::production(
            root,
        )?)
    }
}

fn installer_expectation(dev: bool) -> qol_artifact::ArtifactExpectation {
    if dev {
        qol_artifact::ArtifactExpectation::development_release(
            qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            qol_conventions::artifact::BuildRole::Installer,
            true,
        )
    } else {
        qol_artifact::ArtifactExpectation::production(
            qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            qol_conventions::artifact::BuildRole::Installer,
        )
    }
}

fn install_command(installer: &Path, root: &Path, tray: Option<&Path>, dev: bool) -> Command {
    let mut command = Command::new(installer);
    if dev {
        command.arg("--dev");
    }
    if let Some(tray) = tray {
        command.arg("--source").arg(tray);
    }
    command.arg("--workspace").arg(root);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(command: &Command) -> String {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn dev_flag_accepts_only_dash_dash_dev() {
        assert!(parse_dev_flag(&[OsString::from("--dev")]).unwrap());
        assert!(!parse_dev_flag(&[]).unwrap());
        assert!(parse_dev_flag(&[OsString::from("--verbose")]).is_err());
        #[cfg(unix)]
        assert!(parse_dev_flag(&[non_utf8_arg()]).is_err());
    }

    #[cfg(unix)]
    fn non_utf8_arg() -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0xFF])
    }

    #[test]
    fn build_command_adds_evdev_to_every_install() {
        let root = Path::new("/a/b/ws");
        let plain = build_command(root, &[], false).unwrap();
        assert!(
            args_of(&plain).contains("--features linux_evdev"),
            "production install must grab the keyboard at the evdev level so the desktop cannot shadow it"
        );
        let dev = build_command(root, &[], true).unwrap();
        assert!(
            args_of(&dev).contains("--features dev,linux_evdev"),
            "dev install carries dev and evdev features"
        );
    }

    #[test]
    fn install_command_swaps_source_for_dev_flag() {
        let root = Path::new("/a/b/ws");
        let installer = Path::new("/a/b/ws/target/release/qol-tray-install");
        let tray = Path::new("/a/b/ws/target/release/qol-tray");
        let production = install_command(installer, root, Some(tray), false);
        assert!(args_of(&production).contains("--source"));
        assert!(!args_of(&production).contains("--dev"));
        let dev = install_command(installer, root, None, true);
        let dev_args = args_of(&dev);
        assert!(dev_args.contains("--dev"));
        assert!(!dev_args.contains("--source"));
        assert!(dev_args.contains("--workspace"));
    }
}
