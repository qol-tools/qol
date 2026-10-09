use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::mode::{ModeConfig, ModeFlag};
use crate::paths::install_marker;

pub mod autostart;
pub mod boot_environment;
mod cli;
mod files;
pub mod housekeeping;
pub mod mode;
pub(crate) mod platform;
mod source;

pub use boot_environment::BootEnvironment;
pub use cli::{run, uninstall};
pub(crate) use platform::binary_filename;

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
    if let Err(error) = platform::ensure_desktop_registration() {
        log::warn!("desktop entry self-heal failed: {error:#}");
    }
}

pub fn check_platform_paths() -> Result<()> {
    platform::install_dir()?;
    autostart::autostart_path()?;
    Ok(())
}

fn register_install_id(installed_binary: &Path) -> Result<String> {
    let install_id = create_install_id();
    write_install_id_marker(installed_binary, &install_id)?;
    crate::paths::set_active_install_id(&install_id)?;
    Ok(install_id)
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
    let _ = parent;
    let marker_path = install_marker::marker_path(installed_binary)
        .context("Installed binary has no parent directory")?;
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create marker directory {}", parent.display()))?;
    }
    fs::write(&marker_path, format!("{}\n", install_id))
        .with_context(|| format!("Failed to write install marker {}", marker_path.display()))?;
    if let Some(legacy) = install_marker::legacy_marker_path(installed_binary) {
        if legacy != marker_path {
            let _ = fs::remove_file(legacy);
        }
    }
    Ok(())
}

fn has_install_marker(installed_binary: &Path) -> bool {
    install_marker::existing_marker_path(installed_binary).is_some()
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
