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
    let Some(entries) = debug_entries(plugin_path) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        codesign_path(plugin_id, &identity, entry.path());
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
fn debug_entries(plugin_path: &Path) -> Option<std::fs::ReadDir> {
    std::fs::read_dir(plugin_path.join("target/debug")).ok()
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
