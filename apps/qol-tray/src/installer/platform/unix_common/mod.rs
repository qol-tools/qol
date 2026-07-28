use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{fs, os::unix::fs::PermissionsExt};

pub(super) fn is_running(process_name: &str) -> bool {
    Command::new("pgrep")
        .arg("-x")
        .arg(process_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    super::spawn_detached(binary_path)
}

fn pkill_signal(signal: &str, process_name: &str) {
    let _ = Command::new("pkill")
        .arg(format!("-{signal}"))
        .arg("-x")
        .arg(process_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn wait_for_exit(process_name: &str) -> bool {
    for _ in 0..30 {
        if !is_running(process_name) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

pub(super) fn stop_running_by_name(process_name: &str) -> Result<()> {
    pkill_signal("TERM", process_name);
    if !wait_for_exit(process_name) {
        pkill_signal("KILL", process_name);
    }
    Ok(())
}

pub(super) fn set_executable_permissions(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for {}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to set executable permissions on {}", path.display()))?;
    Ok(())
}
