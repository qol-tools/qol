#![allow(clippy::print_stdout, clippy::print_stderr)]

use super::*;

use qol_apps::tray_install::InstallProfile;

pub fn run(args: impl IntoIterator<Item = String>) -> Result<()> {
    let args = source::parse_args(args)?;
    run_install(
        args.source.as_deref(),
        args.workspace.as_deref(),
        profile_from(args.dev_mode),
    )
}

pub fn uninstall(dev_mode: bool) -> Result<()> {
    let profile = profile_from(dev_mode);
    let installed_binary = platform::install_dir(profile)?.join(platform::binary_filename());
    platform::uninstall(&installed_binary, profile)?;
    if profile == InstallProfile::Development {
        println!("The development QoL Tray install was removed.");
    } else {
        println!("QoL Tray was uninstalled.");
    }
    Ok(())
}

fn profile_from(dev_mode: bool) -> InstallProfile {
    if dev_mode {
        InstallProfile::Development
    } else {
        InstallProfile::Production
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InstallSteps {
    stops_running_tray: bool,
    writes_autostart: bool,
    registers_application: bool,
    claims_active_install_id: bool,
    removes_legacy_install: bool,
    warns_about_system_install: bool,
    installs_workspace_plugins: bool,
}

impl InstallSteps {
    fn for_profile(profile: InstallProfile) -> Self {
        match profile {
            InstallProfile::Production => Self {
                stops_running_tray: true,
                writes_autostart: true,
                registers_application: true,
                claims_active_install_id: true,
                removes_legacy_install: true,
                warns_about_system_install: true,
                installs_workspace_plugins: true,
            },
            InstallProfile::Development => Self {
                stops_running_tray: false,
                writes_autostart: false,
                registers_application: false,
                claims_active_install_id: false,
                removes_legacy_install: false,
                warns_about_system_install: false,
                installs_workspace_plugins: false,
            },
        }
    }
}

fn run_install(
    source_override: Option<&Path>,
    workspace_root: Option<&Path>,
    profile: InstallProfile,
) -> Result<()> {
    let dev_mode = profile == InstallProfile::Development;
    if dev_mode && !cfg!(feature = "dev") {
        return Err(anyhow!(
            "--dev requires a binary built with --features dev. Use `qol install --dev` or pass --features dev to cargo build."
        ));
    }
    let steps = InstallSteps::for_profile(profile);
    println!("Installing QoL Tray...");
    let repo_root = env::current_dir().context("Failed to determine current directory")?;
    let source = source::resolve_source_binary(&repo_root, source_override, dev_mode)?;
    let mut expectation = if dev_mode {
        qol_artifact::ArtifactExpectation::development_release(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            qol_conventions::artifact::BuildRole::Host,
            true,
        )
    } else {
        qol_artifact::ArtifactExpectation::production(
            qol_conventions::artifact::TRAY_HOST_BINARY_NAME,
            qol_conventions::artifact::TRAY_PACKAGE_NAME,
            qol_conventions::artifact::BuildRole::Host,
        )
    };
    if let Some(exact_source) = &source.exact_source {
        expectation = expectation.with_exact_source(exact_source);
    }
    qol_artifact::verify_path(&source.path, &expectation).with_context(|| {
        format!(
            "Refusing to install unverified binary {}",
            source.path.display()
        )
    })?;
    let install_dir = platform::install_dir(profile)?;
    fs::create_dir_all(&install_dir).with_context(|| {
        format!(
            "Failed to create install directory {}",
            install_dir.display()
        )
    })?;
    if steps.removes_legacy_install {
        platform::remove_legacy_install();
    }
    let installed_binary = install_dir.join(platform::binary_filename());
    if steps.stops_running_tray {
        platform::stop_running(&installed_binary)?;
    }
    install_binary_atomically(&source.path, &installed_binary)?;
    platform::set_executable_permissions(&installed_binary)?;
    let install_id = register_install_id(&installed_binary, steps.claims_active_install_id)?;
    let plugins_dir = files::ensure_plugin_dir()?;
    if steps.installs_workspace_plugins {
        install_workspace_plugins(workspace_root, &plugins_dir)?;
    }
    #[cfg(feature = "dev")]
    if dev_mode {
        let env = boot_environment::InstallBootEnvironment {
            installed_binary: installed_binary.clone(),
            honors_dev_selection: true,
            owns_autostart: false,
        };
        let lister = crate::dev::boot_contract::GitWorktreeLister;
        let probe = crate::dev::boot_contract::FsBinaryProbe;
        let config_dir = crate::paths::shared_config_dir()?;
        crate::dev::boot_contract::set_selected_worktree(&env, &config_dir, None, &lister, &probe)?;
    }
    if steps.writes_autostart {
        autostart::write_target(&installed_binary)?;
    }
    if steps.warns_about_system_install {
        platform::warn_system_install_conflict();
    }
    if steps.registers_application {
        platform::register_application(&installed_binary)?;
    }
    write_mode_config(dev_mode)?;
    if dev_mode && platform::tray_running() {
        println!(
            "A QoL Tray is already running, so the development install was staged but not started. Quit the running tray and start {} to use it.",
            installed_binary.display()
        );
    } else {
        platform::start_now(&installed_binary)?;
    }
    print_summary(
        &installed_binary,
        &install_id,
        &plugins_dir,
        &install_dir,
        profile,
    )
}

fn install_workspace_plugins(workspace_root: Option<&Path>, plugins_dir: &Path) -> Result<()> {
    let Some(workspace_root) = workspace_root else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to start the local plugin installer")?;
    let installer =
        crate::features::plugin_store::installer::PluginInstaller::new(plugins_dir.to_path_buf());
    let installed = runtime.block_on(installer.install_local_workspace(workspace_root))?;
    println!("Installed local plugins: {installed}");
    Ok(())
}

fn write_mode_config(dev_mode: bool) -> Result<()> {
    let target = if dev_mode {
        ModeFlag::Dev
    } else {
        ModeFlag::Prod
    };
    ModeConfig::set(target).with_context(|| format!("Failed to write mode config ({target:?})"))?;
    println!("Runtime mode: {}", if dev_mode { "dev" } else { "prod" });
    Ok(())
}

fn print_summary(
    installed_binary: &Path,
    install_id: &str,
    plugins_dir: &Path,
    install_dir: &Path,
    profile: InstallProfile,
) -> Result<()> {
    println!("Installation complete.");
    println!("Installed binary: {}", installed_binary.display());
    println!("Install ID: {}", install_id);
    println!(
        "Autostart entry: {}",
        autostart::autostart_path()?.display()
    );
    println!("Plugins directory: {}", plugins_dir.display());
    if profile == InstallProfile::Development {
        println!(
            "This is the development install. It keeps its own binary and never replaces the production install."
        );
    } else if !is_in_path(install_dir) {
        println!(
            "{} is not in PATH. Add it to run qol-tray directly.",
            install_dir.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_production_profile_owns_host_wide_install_artifacts() {
        assert_eq!(
            InstallSteps::for_profile(InstallProfile::Production),
            InstallSteps {
                stops_running_tray: true,
                writes_autostart: true,
                registers_application: true,
                claims_active_install_id: true,
                removes_legacy_install: true,
                warns_about_system_install: true,
                installs_workspace_plugins: true,
            }
        );
        assert_eq!(
            InstallSteps::for_profile(InstallProfile::Development),
            InstallSteps {
                stops_running_tray: false,
                writes_autostart: false,
                registers_application: false,
                claims_active_install_id: false,
                removes_legacy_install: false,
                warns_about_system_install: false,
                installs_workspace_plugins: false,
            }
        );
    }

    #[test]
    fn the_dev_flag_selects_the_development_profile() {
        assert_eq!(profile_from(true), InstallProfile::Development);
        assert_eq!(profile_from(false), InstallProfile::Production);
    }
}
