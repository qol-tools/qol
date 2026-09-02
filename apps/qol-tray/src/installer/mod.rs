use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::mode::{ModeConfig, ModeFlag};

pub mod autostart;
pub mod boot_environment;
mod files;
pub mod housekeeping;
pub mod mode;
pub(crate) mod platform;
mod source;

pub use boot_environment::BootEnvironment;
pub(crate) use platform::binary_filename;

const INSTALL_ID_FILE: &str = "qol-tray.install-id";

pub fn autostart_path() -> Result<PathBuf> {
    autostart::autostart_path()
}

pub fn bootstrap_current_install() -> Result<()> {
    let current_exe = env::current_exe().context("Failed to determine current executable")?;
    if !platform::should_bootstrap_current_install(&current_exe)? {
        return Ok(());
    }
    if has_install_marker(&current_exe) {
        return Ok(());
    }
    if crate::paths::has_active_install_id() {
        return Ok(());
    }

    let install_id = create_install_id();
    crate::paths::set_active_install_id(&install_id)?;
    files::ensure_plugin_dir()?;
    #[cfg(feature = "dev")]
    {
        let env = boot_environment::InstallBootEnvironment {
            installed_binary: current_exe.clone(),
            honors_dev_selection: true,
        };
        let lister = crate::dev::boot_contract::GitWorktreeLister;
        let probe = crate::dev::boot_contract::FsBinaryProbe;
        let config_dir = crate::paths::shared_config_dir()?;
        crate::dev::boot_contract::set_selected_worktree(&env, &config_dir, None, &lister, &probe)?;
    }
    #[cfg(not(feature = "dev"))]
    autostart::write_target(&current_exe)?;
    Ok(())
}

pub fn ensure_installed_desktop_registration() {
    #[cfg(target_os = "linux")]
    {
        let Ok(current_exe) = env::current_exe() else {
            return;
        };
        let installed = has_install_marker(&current_exe) && is_production_mode();
        if let Err(error) = platform::ensure_desktop_entries(&current_exe, installed) {
            log::warn!("desktop entry self-heal failed: {error:#}");
        }
    }
}

#[cfg(target_os = "linux")]
fn is_production_mode() -> bool {
    mode::ModeConfig::load()
        .map(|config| !config.is_dev())
        .unwrap_or(!cfg!(feature = "dev"))
}

pub fn run(args: impl IntoIterator<Item = String>) -> Result<()> {
    let args = source::parse_args(args)?;
    run_install(
        args.source.as_deref(),
        args.workspace.as_deref(),
        args.dev_mode,
    )
}

pub fn check_platform_paths() -> Result<()> {
    platform::install_dir()?;
    autostart::autostart_path()?;
    Ok(())
}

fn run_install(
    source_override: Option<&Path>,
    workspace_root: Option<&Path>,
    dev_mode: bool,
) -> Result<()> {
    if dev_mode && !cfg!(feature = "dev") {
        return Err(anyhow!(
            "--dev requires a binary built with --features dev. Use `qol install --dev` or pass --features dev to cargo build."
        ));
    }
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
    let install_dir = platform::install_dir()?;
    fs::create_dir_all(&install_dir).with_context(|| {
        format!(
            "Failed to create install directory {}",
            install_dir.display()
        )
    })?;
    platform::remove_legacy_install();
    let installed_binary = install_dir.join(platform::binary_filename());
    platform::stop_running(&installed_binary)?;
    install_binary_atomically(&source.path, &installed_binary)?;
    platform::set_executable_permissions(&installed_binary)?;
    let install_id = register_install_id(&installed_binary)?;
    let plugins_dir = files::ensure_plugin_dir()?;
    install_workspace_plugins(workspace_root, &plugins_dir)?;
    #[cfg(feature = "dev")]
    {
        let env = boot_environment::InstallBootEnvironment {
            installed_binary: installed_binary.clone(),
            honors_dev_selection: true,
        };
        let lister = crate::dev::boot_contract::GitWorktreeLister;
        let probe = crate::dev::boot_contract::FsBinaryProbe;
        let config_dir = crate::paths::shared_config_dir()?;
        crate::dev::boot_contract::set_selected_worktree(&env, &config_dir, None, &lister, &probe)?;
    }
    #[cfg(not(feature = "dev"))]
    autostart::write_target(&installed_binary)?;
    platform::warn_system_install_conflict();
    platform::register_application(&installed_binary)?;
    write_mode_config(dev_mode)?;
    platform::start_now(&installed_binary)?;
    print_summary(&installed_binary, &install_id, &plugins_dir, &install_dir)
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

fn register_install_id(installed_binary: &Path) -> Result<String> {
    let install_id = create_install_id();
    write_install_id_marker(installed_binary, &install_id)?;
    crate::paths::set_active_install_id(&install_id)?;
    Ok(install_id)
}

fn print_summary(
    installed_binary: &Path,
    install_id: &str,
    plugins_dir: &Path,
    install_dir: &Path,
) -> Result<()> {
    println!("Installation complete.");
    println!("Installed binary: {}", installed_binary.display());
    println!("Install ID: {}", install_id);
    println!(
        "Autostart entry: {}",
        autostart::autostart_path()?.display()
    );
    println!("Plugins directory: {}", plugins_dir.display());
    if !is_in_path(install_dir) {
        println!(
            "{} is not in PATH. Add it to run qol-tray directly.",
            install_dir.display()
        );
    }
    Ok(())
}

fn install_binary_atomically(source_binary: &Path, installed_binary: &Path) -> Result<()> {
    let staged_binary = installed_binary.with_extension("new");
    if staged_binary.exists() {
        let _ = fs::remove_file(&staged_binary);
    }
    fs::copy(source_binary, &staged_binary).with_context(|| {
        format!(
            "Failed to copy {} to {}",
            source_binary.display(),
            staged_binary.display()
        )
    })?;
    platform::set_executable_permissions(&staged_binary)?;
    platform::prepare_atomic_replace(installed_binary)?;
    fs::rename(&staged_binary, installed_binary).with_context(|| {
        format!(
            "Failed to finalize install by moving {} to {}",
            staged_binary.display(),
            installed_binary.display()
        )
    })?;
    Ok(())
}

fn write_install_id_marker(installed_binary: &Path, install_id: &str) -> Result<()> {
    let parent = installed_binary
        .parent()
        .context("Installed binary has no parent directory")?;
    let marker_path = parent.join(INSTALL_ID_FILE);
    fs::write(&marker_path, format!("{}\n", install_id))
        .with_context(|| format!("Failed to write install marker {}", marker_path.display()))
}

fn has_install_marker(installed_binary: &Path) -> bool {
    let Some(parent) = installed_binary.parent() else {
        return false;
    };
    parent.join(INSTALL_ID_FILE).exists()
}

fn create_install_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("install-{}-{}", ts, std::process::id())
}

fn is_in_path(dir: &Path) -> bool {
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path_var).any(|path| path == dir)
}
