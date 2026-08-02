use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const JOURNAL_WINDOW: &str = "-15 min";
const RESTART_SCRIPT: &str = "systemctl restart bluetooth";

pub(crate) fn service_journal() -> Option<String> {
    let output = Command::new("journalctl")
        .args(["-u", "bluetooth", "--since", JOURNAL_WINDOW, "--no-pager"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn audio_server() -> Option<String> {
    let output = Command::new("pactl").arg("info").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("Server Name:"))
        .map(|name| name.trim().to_string())
}

pub(crate) fn process_running(process: &str) -> bool {
    !process_ids(process).is_empty()
}

pub(crate) fn stop_process(process: &str) -> Result<()> {
    let pids = process_ids(process);
    if pids.is_empty() {
        bail!("{process} is no longer running");
    }
    for pid in &pids {
        qol_process::terminate_pid(*pid, Duration::from_secs(2));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if pids.iter().all(|pid| !process_alive(*pid)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    bail!("failed to stop {process} without leaving a live matching process")
}

pub(crate) fn start_process(process: &str) -> Result<()> {
    Command::new(process)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to restart {process}"))?;
    Ok(())
}

pub(crate) fn restart_service() -> Result<()> {
    qol_host_fixes::elevation::run_privileged("qol-bluetooth", RESTART_SCRIPT, &[])
}

fn process_ids(process: &str) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse().ok())
        })
        .filter(|pid| process_matches(*pid, process))
        .collect()
}

fn process_matches(pid: u32, process: &str) -> bool {
    let proc_dir = Path::new("/proc").join(pid.to_string());
    if fs::read_to_string(proc_dir.join("comm")).is_ok_and(|comm| comm.trim() == process) {
        return true;
    }
    let Ok(command_line) = fs::read(proc_dir.join("cmdline")) else {
        return false;
    };
    command_line.split(|byte| *byte == 0).any(|argument| {
        Path::new(std::str::from_utf8(argument).unwrap_or_default())
            .file_name()
            .is_some_and(|name| name == process)
    })
}

fn process_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid) && !qol_process::is_pid_zombie(pid)
}

const BLUEMAN_AUTOSTART: &str = "blueman.desktop";

fn autostart_path() -> Result<std::path::PathBuf> {
    use std::path::PathBuf;
    let config_root = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".config"))
            .ok_or_else(|| {
                anyhow::anyhow!("HOME is unavailable for the Blueman autostart claim")
            })?,
    };
    if !config_root.is_absolute() {
        bail!("XDG_CONFIG_HOME must be an absolute path");
    }
    Ok(config_root.join("autostart").join(BLUEMAN_AUTOSTART))
}

pub(crate) fn read_autostart() -> Option<String> {
    autostart_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
}

pub(crate) fn write_autostart(content: &str) -> Result<()> {
    let path = autostart_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)
        .map_err(|error| anyhow::anyhow!("failed to write {}: {error}", path.display()))
}

pub(crate) fn remove_autostart() -> Result<()> {
    let path = autostart_path()?;
    std::fs::remove_file(&path)
        .map_err(|error| anyhow::anyhow!("failed to remove {}: {error}", path.display()))
}

pub(crate) fn supports_autostart() -> bool {
    true
}
