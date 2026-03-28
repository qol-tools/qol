use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

mod files;
mod platform;
mod source;

const INSTALL_ID_FILE: &str = "qol-tray.install-id";

pub fn autostart_path() -> Result<PathBuf> {
    platform::autostart_path()
}

pub fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    platform::write_autostart_entry(binary_path)
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
    platform::write_autostart_entry(&current_exe)
}

pub fn run() -> Result<()> {
    println!("Installing QoL Tray...");
    let repo_root = env::current_dir().context("Failed to determine current directory")?;
    let source_binary = source::resolve_source_binary(&repo_root)?;
    let install_dir = platform::install_dir()?;
    fs::create_dir_all(&install_dir).with_context(|| {
        format!(
            "Failed to create install directory {}",
            install_dir.display()
        )
    })?;
    #[cfg(target_os = "macos")]
    platform::remove_legacy_install();
    let installed_binary = install_dir.join(platform::binary_filename());
    platform::stop_running(&installed_binary)?;
    install_binary_atomically(&source_binary, &installed_binary)?;
    platform::set_executable_permissions(&installed_binary)?;
    let install_id = register_install_id(&installed_binary)?;
    let plugins_dir = files::ensure_plugin_dir()?;
    platform::write_autostart_entry(&installed_binary)?;
    platform::warn_system_install_conflict();
    platform::register_application(&installed_binary)?;
    platform::start_now(&installed_binary)?;
    open_ui_after_start();
    print_summary(&installed_binary, &install_id, &plugins_dir, &install_dir)
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
    println!("Autostart entry: {}", platform::autostart_path()?.display());
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

fn open_ui_after_start() {
    std::thread::sleep(std::time::Duration::from_secs(2));
    let url = "http://localhost:42700";
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", url])
        .spawn();
}

fn is_in_path(dir: &Path) -> bool {
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path_var).any(|path| path == dir)
}
