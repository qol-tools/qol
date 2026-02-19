use anyhow::{anyhow, Context, Result};
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

mod platform;

const APP_NAME: &str = "qol-tray";
const INSTALL_ID_FILE: &str = "qol-tray.install-id";

pub fn run() -> Result<()> {
    println!("Installing QoL Tray...");

    let repo_root = env::current_dir().context("Failed to determine current directory")?;
    let source_binary = resolve_source_binary(&repo_root)?;

    let install_dir = platform::install_dir()?;
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("Failed to create install directory {}", install_dir.display()))?;

    let installed_binary = install_dir.join(platform::binary_filename());
    platform::stop_running()?;
    install_binary_atomically(&source_binary, &installed_binary)?;

    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&installed_binary)
            .with_context(|| format!("Failed to read metadata for {}", installed_binary.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&installed_binary, permissions).with_context(|| {
            format!("Failed to set executable permissions on {}", installed_binary.display())
        })?;
    }

    let install_id = create_install_id();
    write_install_id_marker(&installed_binary, &install_id)?;
    crate::paths::set_active_install_id(&install_id)?;
    let plugins_dir = ensure_plugin_dir(&install_id)?;
    seed_example_plugin(&repo_root, &plugins_dir)?;
    platform::write_autostart_entry(&installed_binary)?;
    platform::start_now(&installed_binary)?;

    println!("Installation complete.");
    println!("Installed binary: {}", installed_binary.display());
    println!("Install ID: {}", install_id);
    println!("Autostart entry: {}", platform::autostart_path()?.display());
    println!("Plugins directory: {}", plugins_dir.display());

    if !is_in_path(&install_dir) {
        println!("{} is not in PATH. Add it to run qol-tray directly.", install_dir.display());
    }

    Ok(())
}

fn resolve_source_binary(repo_root: &Path) -> Result<PathBuf> {
    if let Some(path) = source_binary_from_args()? {
        return ensure_existing_source(path);
    }

    if let Some(path) = source_binary_from_env() {
        return ensure_existing_source(path);
    }

    if let Some(path) = source_binary_from_platform_candidates()? {
        return Ok(path);
    }

    let source_binary = repo_root
        .join("target")
        .join("release")
        .join(platform::binary_filename());

    if source_binary.is_file() {
        return Ok(source_binary);
    }

    if repo_root.join("Cargo.toml").is_file() {
        build_release_binary(repo_root)?;
        if source_binary.is_file() {
            return Ok(source_binary);
        }
    }

    Err(anyhow!("Built binary not found at {}", source_binary.display()))
}

fn source_binary_from_args() -> Result<Option<PathBuf>> {
    let mut args = env::args().skip(1);
    let mut source = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--source requires a path"))?;
                source = Some(PathBuf::from(value));
            }
            _ => {
                return Err(anyhow!("Unknown argument: {}", arg));
            }
        }
    }

    Ok(source)
}

fn source_binary_from_env() -> Option<PathBuf> {
    env::var("QOL_TRAY_INSTALL_SOURCE")
        .ok()
        .map(PathBuf::from)
}

fn source_binary_from_platform_candidates() -> Result<Option<PathBuf>> {
    let current_exe = env::current_exe().context("Failed to determine installer path")?;
    for path in platform::bundled_binary_candidates(&current_exe) {
        if path != current_exe && path.is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn ensure_existing_source(path: PathBuf) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path);
    }
    Err(anyhow!("Source binary not found at {}", path.display()))
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

    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&staged_binary)
            .with_context(|| format!("Failed to read metadata for {}", staged_binary.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&staged_binary, permissions).with_context(|| {
            format!("Failed to set executable permissions on {}", staged_binary.display())
        })?;
    }

    #[cfg(windows)]
    if installed_binary.exists() {
        fs::remove_file(installed_binary).with_context(|| {
            format!(
                "Failed to remove existing installed binary {}",
                installed_binary.display()
            )
        })?;
    }

    fs::rename(&staged_binary, installed_binary).with_context(|| {
        format!(
            "Failed to finalize install by moving {} to {}",
            staged_binary.display(),
            installed_binary.display()
        )
    })?;

    Ok(())
}

fn build_release_binary(repo_root: &Path) -> Result<()> {
    let manifest_path = repo_root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(anyhow!("Cargo.toml not found at {}", manifest_path.display()));
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--bin")
        .arg(APP_NAME)
        .arg("--manifest-path")
        .arg(&manifest_path)
        .status()
        .context("Failed to run cargo build")?;

    if !status.success() {
        return Err(anyhow!("cargo build failed with status {}", status));
    }

    Ok(())
}

fn ensure_plugin_dir(install_id: &str) -> Result<PathBuf> {
    let config_dir = crate::paths::config_dir_for_install_id(install_id)?;
    let plugins_dir = config_dir.join("plugins");
    fs::create_dir_all(&plugins_dir)
        .with_context(|| format!("Failed to create plugins directory {}", plugins_dir.display()))?;
    Ok(plugins_dir)
}

fn write_install_id_marker(installed_binary: &Path, install_id: &str) -> Result<()> {
    let parent = installed_binary
        .parent()
        .context("Installed binary has no parent directory")?;
    let marker_path = parent.join(INSTALL_ID_FILE);
    fs::write(&marker_path, format!("{}\n", install_id))
        .with_context(|| format!("Failed to write install marker {}", marker_path.display()))
}

fn create_install_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("install-{}-{}", ts, std::process::id())
}

fn seed_example_plugin(repo_root: &Path, plugins_dir: &Path) -> Result<()> {
    let source = repo_root
        .join("examples")
        .join("plugins")
        .join("screen-recorder");
    if !source.is_dir() {
        return Ok(());
    }

    let target = plugins_dir.join("screen-recorder");
    if target.exists() {
        return Ok(());
    }

    copy_dir_recursive(&source, &target)?;
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("Failed to create directory {}", target.display()))?;

    for entry in fs::read_dir(source)
        .with_context(|| format!("Failed to read directory {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("Failed to read entry in {}", source.display()))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("Failed to read file type for {}", source_path.display()))?;

        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
            continue;
        }

        if file_type.is_file() {
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "Failed to copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;

            #[cfg(unix)]
            {
                if source_path
                    .file_name()
                    .map(|name| name.to_string_lossy() == "run.sh")
                    .unwrap_or(false)
                {
                    let mut permissions = fs::metadata(&target_path)
                        .with_context(|| {
                            format!("Failed to read metadata for {}", target_path.display())
                        })?
                        .permissions();
                    permissions.set_mode(0o755);
                    fs::set_permissions(&target_path, permissions).with_context(|| {
                        format!("Failed to set executable permissions on {}", target_path.display())
                    })?;
                }
            }
        }
    }

    Ok(())
}

fn is_in_path(dir: &Path) -> bool {
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path_var).any(|path| path == dir)
}
