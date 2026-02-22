use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt};

pub fn install_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".local").join("bin"))
}

pub fn is_running(process_name: &str) -> bool {
    Command::new("pgrep")
        .arg("-x")
        .arg(process_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn start_now(binary_path: &Path) -> Result<()> {
    Command::new(binary_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start {}", binary_path.display()))?;
    Ok(())
}

pub fn stop_running(binary_path: &Path, process_name: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if binary_path.exists() {
            stop_running_linux(binary_path);
            return Ok(());
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = binary_path;

    let _ = Command::new("pkill")
        .arg("-TERM")
        .arg("-x")
        .arg(process_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    for _ in 0..30 {
        if !is_running(process_name) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let _ = Command::new("pkill")
        .arg("-KILL")
        .arg("-x")
        .arg(process_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    Ok(())
}

pub fn set_executable_permissions(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for {}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to set executable permissions on {}", path.display()))?;
    Ok(())
}

pub fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    let link_target = fs::read_link(source)
        .with_context(|| format!("Failed to read symlink {}", source.display()))?;
    std::os::unix::fs::symlink(&link_target, target)
        .with_context(|| format!("Failed to create symlink {}", target.display()))?;
    Ok(())
}

pub fn on_file_copied(source: &Path, target: &Path) -> Result<()> {
    if source
        .file_name()
        .map(|name| name.to_string_lossy() == "run.sh")
        .unwrap_or(false)
    {
        set_executable_permissions(target)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn stop_running_linux(binary_path: &Path) {
    let pids = linux_pids_for_binary(binary_path);
    if pids.is_empty() {
        return;
    }

    for pid in &pids {
        crate::process_utils::terminate_pid(*pid, std::time::Duration::from_millis(100));
    }

    for _ in 0..30 {
        if pids.iter().all(|pid| !crate::process_utils::is_pid_alive(*pid)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    for pid in pids {
        crate::process_utils::terminate_pid(pid, std::time::Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn linux_pids_for_binary(binary_path: &Path) -> Vec<i32> {
    let target = std::fs::canonicalize(binary_path).unwrap_or_else(|_| binary_path.to_path_buf());
    let current_pid = std::process::id() as i32;
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
        .filter(|pid| *pid > 0 && *pid != current_pid)
        .filter(|pid| {
            let exe = std::path::Path::new("/proc")
                .join(pid.to_string())
                .join("exe");
            let Ok(exe_path) = std::fs::read_link(exe) else {
                return false;
            };
            let resolved = std::fs::canonicalize(&exe_path).unwrap_or(exe_path);
            resolved == target
        })
        .collect()
}
