use anyhow::{bail, Context, Result};
use std::process::{Command, Stdio};

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
    Command::new("pgrep")
        .args(["-f", process])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn stop_process(process: &str) -> Result<()> {
    let status = Command::new("pkill")
        .args(["-f", process])
        .status()
        .with_context(|| format!("failed to launch pkill for {process}"))?;
    if !status.success() {
        bail!("pkill did not stop {process}");
    }
    Ok(())
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
