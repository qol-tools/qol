use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

#[cfg(target_os = "macos")]
pub(super) fn codesign_debug_binaries(plugin_id: &str, plugin_path: &Path) {
    let Some(identity) = codesign_identity() else {
        return;
    };

    for binary in plugin_debug_binaries(plugin_path) {
        codesign_path(plugin_id, &identity, binary);
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn codesign_debug_binaries(_plugin_id: &str, _plugin_path: &Path) {}

#[cfg(target_os = "macos")]
fn codesign_identity() -> Option<String> {
    std::env::var("QOL_CODESIGN_IDENTITY")
        .ok()
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn plugin_debug_binaries(plugin_path: &Path) -> Vec<PathBuf> {
    let Ok(plugin_root) = plugin_path.canonicalize() else {
        return Vec::new();
    };
    let Some(metadata) = cargo_metadata(&plugin_root) else {
        return Vec::new();
    };
    let Some(target_directory) = metadata.get("target_directory").and_then(|v| v.as_str()) else {
        return Vec::new();
    };
    let debug_dir = Path::new(target_directory).join("debug");
    let Some(packages) = metadata.get("packages").and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    packages
        .iter()
        .filter(|&package| package_in_plugin(package, &plugin_root))
        .flat_map(bin_target_names)
        .map(|name| debug_dir.join(name))
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(target_os = "macos")]
fn cargo_metadata(plugin_root: &Path) -> Option<serde_json::Value> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(plugin_root)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

#[cfg(target_os = "macos")]
fn package_in_plugin(package: &serde_json::Value, plugin_root: &Path) -> bool {
    package
        .get("manifest_path")
        .and_then(|v| v.as_str())
        .and_then(|manifest| Path::new(manifest).canonicalize().ok())
        .map(|manifest| manifest.starts_with(plugin_root))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn bin_target_names(package: &serde_json::Value) -> Vec<String> {
    let Some(targets) = package.get("targets").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    targets
        .iter()
        .filter(|target| target_is_bin(target))
        .filter_map(|target| target.get("name").and_then(|v| v.as_str()))
        .map(String::from)
        .collect()
}

#[cfg(target_os = "macos")]
fn target_is_bin(target: &serde_json::Value) -> bool {
    target
        .get("kind")
        .and_then(|v| v.as_array())
        .map(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bin")))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn codesign_path(plugin_id: &str, identity: &str, path: PathBuf) {
    if !codesign_candidate(&path) {
        return;
    }

    match codesign_status(identity, &path) {
        Ok(status) if status.success() => {
            log::debug!("[{}] codesigned {}", plugin_id, path.display());
        }
        Ok(_) => {
            log::warn!("[{}] codesign failed for {}", plugin_id, path.display());
        }
        Err(error) => {
            log::warn!("[{}] codesign exec failed: {}", plugin_id, error);
        }
    }
}

#[cfg(target_os = "macos")]
fn codesign_candidate(path: &Path) -> bool {
    path.is_file() && executable(path) && path.extension().is_none()
}

#[cfg(target_os = "macos")]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn codesign_status(identity: &str, path: &Path) -> std::io::Result<std::process::ExitStatus> {
    Command::new("codesign")
        .args(["-fs", identity])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
}
