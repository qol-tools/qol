use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::mode::{ModeConfig, ModeFlag};

pub mod autostart;
pub mod boot_environment;
pub(crate) mod desktop_entry;
mod files;
mod platform;
mod shell_hook;
mod source;

pub use boot_environment::BootEnvironment;
pub(crate) use platform::binary_filename;

const INSTALL_ID_FILE: &str = "qol-tray.install-id";

pub fn autostart_path() -> Result<PathBuf> {
    platform::autostart_path()
}

pub fn install_shell_hook() -> Result<()> {
    shell_hook::install()
}

pub(crate) use shell_hook::{is_installed as shell_hook_status, ShellHookStatus};

pub(crate) fn shell_hook_any_rc_exists() -> Result<bool> {
    shell_hook::any_rc_file_exists()
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

pub fn run() -> Result<()> {
    let args = source::parse_args()?;
    if args.help {
        print_help();
        return Ok(());
    }
    match args.mode {
        source::Mode::Install => {
            run_install(args.source.as_deref(), args.skip_shell_hook, args.dev_mode)
        }
        source::Mode::Uninstall => run_uninstall(args.skip_shell_hook),
    }
}

fn run_install(
    source_override: Option<&Path>,
    skip_shell_hook: bool,
    dev_mode: bool,
) -> Result<()> {
    if dev_mode && !cfg!(feature = "dev") {
        return Err(anyhow!(
            "--dev requires a binary built with --features dev. Use `make install-dev` or pass --features dev to cargo build."
        ));
    }
    println!("Installing QoL Tray...");
    let repo_root = env::current_dir().context("Failed to determine current directory")?;
    let source_binary = source::resolve_source_binary(&repo_root, source_override, dev_mode)?;
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
    install_shell_hook_warn_only(skip_shell_hook);
    write_mode_config(dev_mode)?;
    platform::start_now(&installed_binary)?;
    open_ui_after_start();
    print_summary(&installed_binary, &install_id, &plugins_dir, &install_dir)
}

fn run_uninstall(skip_shell_hook: bool) -> Result<()> {
    println!("Uninstalling QoL Tray shell hook...");
    if skip_shell_hook {
        println!("Skipping shell hook removal (--skip-shell-hook).");
        return Ok(());
    }
    if let Err(error) = shell_hook::uninstall() {
        eprintln!("Warning: failed to remove shell hook: {error}");
        return Err(error);
    }
    println!("Shell hook removed. Open a new terminal for changes to take effect.");
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

fn install_shell_hook_warn_only(skip_shell_hook: bool) {
    if skip_shell_hook {
        println!("Skipping shell hook installation (--skip-shell-hook).");
        return;
    }
    if let Err(error) = shell_hook::install() {
        eprintln!("Warning: failed to install shell hook: {error}");
        return;
    }
    println!("Shell hook installed. Open a new terminal for changes to take effect.");
}

fn print_help() {
    println!(
        "qol-tray-install\n\
         \n\
         Install or uninstall the QoL Tray binary, autostart entry, and shell hook.\n\
         \n\
         Usage:\n  \
           qol-tray-install [--source <path>] [--skip-shell-hook] [--dev]\n  \
           qol-tray-install --uninstall [--skip-shell-hook]\n  \
           qol-tray-install --help\n\
         \n\
         Flags:\n  \
           --source <path>      Use the binary at <path> as the install source.\n  \
           --uninstall          Remove the qol-tools shell hook from rc files.\n  \
           --skip-shell-hook    Do not touch ~/.zshrc or ~/.bashrc.\n  \
           --dev                Write runtime mode = dev. Requires --features dev.\n  \
           --help, -h           Print this help message.\n"
    );
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
