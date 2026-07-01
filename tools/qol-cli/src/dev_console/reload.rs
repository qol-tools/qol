use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::dev_server::{
    health_ok, post_promote_generation, post_recompile_current, post_reload_plugins,
    wait_for_shutdown_best_effort,
};
use crate::host_facade;

use super::{
    spawn_forwarders, terminate_child, try_wait, Dash, RebuildState, Reload, ReloadOutcome,
    TrayHandle, CRASH_TAIL, HANDOFF_STOP_GRACE, HANDOFF_STOP_INTERVAL, PROMOTION_INTERVAL,
    PROMOTION_TIMEOUT, SHADOW_READY_INTERVAL, SHADOW_READY_TIMEOUT,
};

#[derive(Debug, Deserialize)]
struct ShadowGenerationReady {
    generation: String,
    id: Option<String>,
    port: u16,
    #[serde(rename = "stateSocket")]
    state_socket: String,
}

pub(super) fn trigger_rebuild(dash: &mut Dash) {
    dash.rebuild = match post_recompile_current() {
        Ok(()) => {
            dash.pokes.doctor = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

pub(super) fn trigger_reload(dash: &mut Dash) {
    dash.plugin_reload = match post_reload_plugins() {
        Ok(()) => {
            dash.pokes.links = true;
            dash.pokes.doctor = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}

pub(super) fn start_reload(dash: &mut Dash) {
    if dash.is_reloading() {
        return;
    }
    match spawn_reload() {
        Some((child, rx)) => {
            dash.push_log("[qol dev] reloading: prebuild dev artifacts");
            dash.reload = Reload::Running { child, rx };
        }
        None => dash.push_log("[qol dev] reload failed to start"),
    }
}

fn spawn_reload() -> Option<(Child, Receiver<String>)> {
    let root = crate::workspace::repo_root().ok()?;
    let exe = std::env::current_exe().ok()?;
    let raw_args = std::env::args_os().skip(1);
    let mut command = reload_prebuild_command(&root, &exe, raw_args);
    let mut child = command.spawn().ok()?;
    let rx = spawn_forwarders(&mut child);
    Some((child, rx))
}

fn reload_prebuild_command(
    root: &Path,
    exe: &Path,
    raw_args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Command {
    let mut command = Command::new(exe);
    command
        .arg(crate::commands::dev::DEV_PREBUILD_COMMAND)
        .args(reload_prebuild_args(raw_args))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn reload_prebuild_args(
    raw_args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Vec<std::ffi::OsString> {
    let parsed = crate::cli::parse_cli(raw_args.into_iter().collect());
    let mut args = Vec::new();
    if parsed.verbose {
        args.push("-v".into());
    }
    if parsed.skip_plugins {
        args.push("-n".into());
    }
    if parsed.values.first().and_then(|arg| arg.to_str()) == Some("dev") {
        args.extend(parsed.values.into_iter().skip(1));
    }
    args
}

pub(super) fn poll_reload(dash: &mut Dash) -> ReloadOutcome {
    let mut drained = Vec::new();
    let status = match &mut dash.reload {
        Reload::Idle => return ReloadOutcome::Pending,
        Reload::Running { child, rx, .. } => {
            while let Ok(line) = rx.try_recv() {
                drained.push(line);
            }
            match child.try_wait() {
                Ok(Some(status)) => status,
                _ => {
                    for line in drained {
                        dash.push_log(line);
                    }
                    return ReloadOutcome::Pending;
                }
            }
        }
    };
    for line in drained {
        dash.push_log(line);
    }
    dash.reload = Reload::Idle;
    if status.success() {
        return ReloadOutcome::Ready;
    }
    dash.push_log(format!("[qol dev] reload aborted: prebuild {status}"));
    ReloadOutcome::Pending
}

pub(super) fn restart_child_from_prebuilt(
    child: &mut TrayHandle,
    lines: &mut Receiver<String>,
    dash: &mut Dash,
) -> Result<()> {
    dash.push_log("[qol dev] starting successor generation");
    let root = crate::workspace::repo_root()?;
    let binary = root
        .join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray"));
    let (next, next_lines, ready) = start_shadow_generation(&root, &binary, dash)?;
    let mut next = TrayHandle::Owned(next);
    if let Err(error) = retire_child_for_handoff(child) {
        terminate_child(&mut next);
        let _ = next.wait();
        return Err(error);
    }
    wait_for_shutdown_best_effort();
    if let Err(error) = promote_shadow_generation(ready.port, &mut next, &next_lines, dash) {
        dash.push_log(format!(
            "[qol dev] successor promotion incomplete: {error:#}"
        ));
    }
    *child = next;
    *lines = next_lines;
    dash.pokes.doctor = true;
    dash.pokes.links = true;
    dash.push_log("[qol dev] successor generation active");
    Ok(())
}

fn start_shadow_generation(
    root: &Path,
    binary: &Path,
    dash: &mut Dash,
) -> Result<(Child, Receiver<String>, ShadowGenerationReady)> {
    let generation_id = shadow_generation_id();
    let ready_file = shadow_ready_file(root, &generation_id);
    let _ = fs::remove_file(&ready_file);
    dash.push_log(format!(
        "[qol dev] booting successor generation {generation_id}"
    ));
    let mut child = shadow_generation_command(root, binary, &generation_id, &ready_file)
        .spawn()
        .with_context(|| format!("failed to start successor {}", binary.display()))?;
    let rx = spawn_forwarders(&mut child);
    let ready = match wait_for_shadow_ready(&ready_file, &mut child, &rx, dash) {
        Ok(ready) => ready,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    dash.push_log(format!(
        "[qol dev] successor ready: {} localhost:{} state={}",
        ready.id.as_deref().unwrap_or("unknown"),
        ready.port,
        ready.state_socket
    ));
    Ok((child, rx, ready))
}

fn promote_shadow_generation(
    port: u16,
    child: &mut TrayHandle,
    rx: &Receiver<String>,
    dash: &mut Dash,
) -> Result<()> {
    let deadline = Instant::now() + PROMOTION_TIMEOUT;
    let mut requested = false;
    let mut last_error = None;
    while Instant::now() < deadline {
        let _ = drain_shadow_logs(rx, dash);
        if let Some(status) = child.try_wait()? {
            bail!("successor generation exited during promotion: {status}");
        }
        if !requested {
            match post_promote_generation(port) {
                Ok(()) => {
                    requested = true;
                    dash.push_log("[qol dev] successor promotion requested");
                }
                Err(error) => last_error = Some(error.to_string()),
            }
        }
        if requested && health_ok() {
            return Ok(());
        }
        std::thread::sleep(PROMOTION_INTERVAL);
    }
    match last_error {
        Some(error) => bail!("stable dev API did not promote: {error}"),
        None => bail!("stable dev API did not become healthy after promotion"),
    }
}

fn shadow_generation_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("{}-{millis}", std::process::id())
}

fn shadow_ready_file(root: &Path, generation_id: &str) -> PathBuf {
    root.join("target")
        .join("qol-dev")
        .join("generations")
        .join(format!("{generation_id}.json"))
}

fn shadow_generation_command(
    root: &Path,
    binary: &Path,
    generation_id: &str,
    ready_file: &Path,
) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .arg("--write-mode=dev")
        .env(
            qol_conventions::ENV_DEV_GENERATION_MODE,
            qol_conventions::DEV_GENERATION_MODE_SHADOW,
        )
        .env(qol_conventions::ENV_DEV_GENERATION_ID, generation_id)
        .env(qol_conventions::ENV_DEV_READY_FILE, ready_file)
        .env(qol_conventions::ENV_DEV_UI_PORT, "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn wait_for_shadow_ready(
    ready_file: &Path,
    child: &mut Child,
    rx: &Receiver<String>,
    dash: &mut Dash,
) -> Result<ShadowGenerationReady> {
    let deadline = Instant::now() + SHADOW_READY_TIMEOUT;
    let mut recent_logs = VecDeque::new();
    while Instant::now() < deadline {
        append_shadow_logs(&mut recent_logs, drain_shadow_logs(rx, dash));
        if ready_file.is_file() {
            return read_shadow_ready(ready_file);
        }
        if let Some(status) = child.try_wait()? {
            append_shadow_logs(&mut recent_logs, drain_shadow_logs(rx, dash));
            bail!(
                "shadow generation exited before ready: {status}{}",
                shadow_crash_detail(&recent_logs)
            );
        }
        std::thread::sleep(SHADOW_READY_INTERVAL);
    }
    append_shadow_logs(&mut recent_logs, drain_shadow_logs(rx, dash));
    bail!(
        "shadow generation did not become ready within {SHADOW_READY_TIMEOUT:?}{}",
        shadow_crash_detail(&recent_logs)
    )
}

fn drain_shadow_logs(rx: &Receiver<String>, dash: &mut Dash) -> Vec<String> {
    let mut drained = Vec::new();
    while let Ok(line) = rx.try_recv() {
        let line = format!("[qol dev:shadow] {line}");
        dash.push_log(line.clone());
        drained.push(line);
    }
    drained
}

fn append_shadow_logs(recent: &mut VecDeque<String>, lines: Vec<String>) {
    for line in lines {
        recent.push_back(line);
        while recent.len() > CRASH_TAIL {
            recent.pop_front();
        }
    }
}

fn shadow_crash_detail(recent: &VecDeque<String>) -> String {
    if recent.is_empty() {
        return String::new();
    }
    let mut detail = String::from("\nlast shadow logs:");
    for line in recent {
        detail.push('\n');
        detail.push_str(line);
    }
    detail
}

fn read_shadow_ready(path: &Path) -> Result<ShadowGenerationReady> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read shadow ready file {}", path.display()))?;
    let ready: ShadowGenerationReady =
        serde_json::from_str(&content).context("invalid shadow ready payload")?;
    if ready.generation != qol_conventions::DEV_GENERATION_MODE_SHADOW {
        bail!("unexpected shadow generation marker: {}", ready.generation);
    }
    Ok(ready)
}

fn retire_child_for_handoff(child: &mut TrayHandle) -> Result<()> {
    terminate_child(child);
    let deadline = Instant::now() + HANDOFF_STOP_GRACE;
    while Instant::now() < deadline {
        if try_wait(child)?.is_some() {
            return Ok(());
        }
        std::thread::sleep(HANDOFF_STOP_INTERVAL);
    }
    let _ = child.kill();
    child
        .wait()
        .context("failed to reap previous qol-tray generation")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn armed_reload_builds_the_workspace_cli_incrementally() {
        let root = Path::new("/repo/qol");
        let exe = Path::new("/bin/qol");
        let command = reload_prebuild_command(
            root,
            exe,
            ["-n", "dev", "feat/x", "-v"].map(std::ffi::OsString::from),
        );
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(
            args,
            [
                crate::commands::dev::DEV_PREBUILD_COMMAND,
                "-v",
                "-n",
                "feat/x",
            ]
            .map(OsStr::new)
        );
        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(command.get_program(), exe.as_os_str());
    }

    #[test]
    fn shadow_generation_command_uses_shared_generation_contract() {
        let root = Path::new("/repo/qol");
        let binary = Path::new("/repo/qol/target/debug/qol-tray");
        let ready_file = Path::new("/repo/qol/target/qol-dev/generations/abc.json");
        let command = shadow_generation_command(root, binary, "abc", ready_file);

        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(command.get_program(), binary.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("--write-mode=dev")]
        );
        let envs: std::collections::HashMap<_, _> = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect();
        assert_eq!(
            envs.get(OsStr::new(qol_conventions::ENV_DEV_GENERATION_MODE))
                .copied(),
            Some(OsStr::new(qol_conventions::DEV_GENERATION_MODE_SHADOW))
        );
        assert_eq!(
            envs.get(OsStr::new(qol_conventions::ENV_DEV_GENERATION_ID))
                .copied(),
            Some(OsStr::new("abc"))
        );
        assert_eq!(
            envs.get(OsStr::new(qol_conventions::ENV_DEV_READY_FILE))
                .copied(),
            Some(ready_file.as_os_str())
        );
        assert_eq!(
            envs.get(OsStr::new(qol_conventions::ENV_DEV_UI_PORT))
                .copied(),
            Some(OsStr::new("0"))
        );
    }

    #[test]
    fn shadow_crash_detail_includes_recent_logs() {
        let mut recent = VecDeque::new();
        append_shadow_logs(
            &mut recent,
            vec![
                "[qol dev:shadow] first".to_string(),
                "[qol dev:shadow] Error: daemon missing".to_string(),
            ],
        );

        assert_eq!(
            shadow_crash_detail(&recent),
            "\nlast shadow logs:\n[qol dev:shadow] first\n[qol dev:shadow] Error: daemon missing"
        );
    }
}
