use anyhow::{anyhow, Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::platform;

const APP_NAME: &str = "qol-tray";

pub(super) fn resolve_source_binary(repo_root: &Path) -> Result<PathBuf> {
    if let Some(path) = source_binary_from_args()? {
        return ensure_existing_source(path);
    }
    if let Some(path) = source_binary_from_env() {
        return ensure_existing_source(path);
    }
    if let Some(path) = source_binary_from_platform_candidates()? {
        return Ok(path);
    }
    release_binary(repo_root)
}

fn release_binary(repo_root: &Path) -> Result<PathBuf> {
    let path = repo_root
        .join("target")
        .join("release")
        .join(platform::binary_filename());
    if path.is_file() {
        return Ok(path);
    }
    if repo_root.join("Cargo.toml").is_file() {
        build_release_binary(repo_root)?;
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(anyhow!("Built binary not found at {}", path.display()))
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
    env::var("QOL_TRAY_INSTALL_SOURCE").ok().map(PathBuf::from)
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

fn build_release_binary(repo_root: &Path) -> Result<()> {
    let manifest_path = repo_root.join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(anyhow!(
            "Cargo.toml not found at {}",
            manifest_path.display()
        ));
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
