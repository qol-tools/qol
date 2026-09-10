use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::handoff_display::HandoffUpdates;

use super::{
    spawn_forwarders, terminate_child, try_wait, Dash, RebuildState, Reload, ReloadOutcome,
    ReloadProgress, TrayHandle, WorktreeSelection, CRASH_TAIL, HANDOFF_STOP_INTERVAL,
    PROMOTION_INTERVAL, PROMOTION_TIMEOUT, SHADOW_READY_INTERVAL, SHADOW_READY_TIMEOUT, STOP_GRACE,
};
use crate::dev_server::{
    health_ok, post_promote_generation, post_recompile_current, post_reload_plugins, post_shutdown,
};
use crate::dev_shutdown::{
    format_daemon_pids, handoff_daemon_groups, snapshot_runtime_daemon_pids,
    terminate_daemon_groups, wait_for_daemons_to_exit, TrackedDaemonPid,
};

const MONITOR_DAEMON_SOCKET_FILE: &str = "qol-plugin-monitor.sock";
const MONITOR_HANDOFF_TIMEOUT: Duration = Duration::from_millis(500);
const RELOAD_TAIL_QUIET: Duration = Duration::from_millis(50);
const RELOAD_TAIL_BUDGET: Duration = Duration::from_millis(500);

#[derive(Debug, Deserialize)]
struct ShadowGenerationReady {
    generation: String,
    id: Option<String>,
    port: u16,
    #[serde(rename = "stateSocket")]
    state_socket: String,
}

const PREDECESSOR_DAEMON_STOP_GRACE: Duration = Duration::from_secs(2);

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
    let branch = crate::commands::dev::current_active_worktree_marker();
    dash.plugin_reload = match post_reload_plugins(branch.as_deref()) {
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
    dash.clear_reload_failure();
    let selection = dash.worktree_selection.clone();
    match spawn_reload(dash) {
        Ok((child, rx)) => {
            dash.push_log("[qol dev] reloading: prebuild dev artifacts");
            dash.reload = Reload::Running {
                child,
                rx,
                activity: ReloadProgress::new(),
                selection,
            };
        }
        Err(error) => record_spawn_error(dash, &selection, &error),
    }
}

fn record_spawn_error(dash: &mut Dash, selection: &WorktreeSelection, error: &anyhow::Error) {
    dash.push_log(format!("[qol dev] reload failed to start: {error:#}"));
    dash.record_reload_failure(selection, &format!("{error:#}"));
}

fn spawn_reload(dash: &Dash) -> Result<(Child, Receiver<String>)> {
    let root = crate::workspace::repo_root().context("failed to resolve qol workspace root")?;
    let exe = reload_executable().context("failed to resolve reload executable")?;
    let raw_args = std::env::args_os().skip(1);
    let mut command = reload_prebuild_command(&root, &exe, raw_args, reload_target_arg(dash));
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn reload prebuild command {}", exe.display()))?;
    let rx = spawn_forwarders(&mut child);
    Ok((child, rx))
}

fn reload_executable() -> Result<PathBuf> {
    let current = std::env::current_exe().context("failed to resolve current executable")?;
    Ok(reload_executable_path(
        &current,
        std::env::var_os("HOME").as_deref().map(Path::new),
    ))
}

fn reload_executable_path(current: &Path, home: Option<&Path>) -> PathBuf {
    if current.symlink_metadata().is_ok() {
        return current.to_path_buf();
    }
    if let Some(home) = home {
        return home
            .join(".cargo")
            .join("bin")
            .join(crate::workspace::exe_name("qol"));
    }
    PathBuf::from(crate::workspace::exe_name("qol"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReloadTargetArg {
    Passthrough,
    Base,
    Branch(String),
}

fn reload_target_arg(dash: &Dash) -> ReloadTargetArg {
    match &dash.worktree_selection {
        WorktreeSelection::Follow => ReloadTargetArg::Passthrough,
        WorktreeSelection::Pin(None) => ReloadTargetArg::Base,
        WorktreeSelection::Pin(Some(branch)) => ReloadTargetArg::Branch(branch.clone()),
    }
}

fn reload_prebuild_command(
    root: &Path,
    exe: &Path,
    raw_args: impl IntoIterator<Item = std::ffi::OsString>,
    target: ReloadTargetArg,
) -> Command {
    let mut command = Command::new(exe);
    command
        .arg(crate::commands::dev::DEV_PREBUILD_COMMAND)
        .args(reload_prebuild_args(raw_args, target))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn reload_prebuild_args(
    raw_args: impl IntoIterator<Item = std::ffi::OsString>,
    target: ReloadTargetArg,
) -> Vec<std::ffi::OsString> {
    let parsed = crate::cli::parse_cli(raw_args.into_iter().collect());
    let mut args = Vec::new();
    if parsed.verbose {
        args.push("-v".into());
    }
    if parsed.skip_plugins {
        args.push("-n".into());
    }
    match target {
        ReloadTargetArg::Base => args.push(crate::commands::dev::DEV_PREBUILD_BASE_ARG.into()),
        ReloadTargetArg::Branch(branch) => args.push(branch.into()),
        ReloadTargetArg::Passthrough => {}
    }
    args
}

pub(super) fn poll_reload(dash: &mut Dash) -> ReloadOutcome {
    let mut drained = Vec::new();
    let status = match &mut dash.reload {
        Reload::Idle | Reload::Handoff { .. } => return ReloadOutcome::Pending,
        Reload::Running {
            child,
            rx,
            activity,
            ..
        } => {
            while let Ok(line) = rx.try_recv() {
                if !activity.observe(&line) {
                    activity.observe_failure(&line);
                    drained.push(line);
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    drain_forwarder_tail(rx, activity, &mut drained);
                    status
                }
                _ => {
                    for line in drained {
                        dash.push_log(line);
                    }
                    return ReloadOutcome::Pending;
                }
            }
        }
    };
    for line in &drained {
        dash.push_log(line.clone());
    }
    let completed = std::mem::replace(&mut dash.reload, Reload::Idle);
    match completed {
        Reload::Running {
            mut activity,
            selection,
            ..
        } => {
            if status.success() {
                dash.clear_reload_failure();
                activity.phase = "handoff".to_string();
                activity.detail = "successor generation".to_string();
                dash.reload = Reload::Handoff {
                    activity,
                    selection,
                };
                return ReloadOutcome::Ready;
            }
            dash.push_log(format!("[qol dev] reload aborted: prebuild {status}"));
            dash.record_reload_failure(&selection, &reload_failure_reason(status, &activity));
            ReloadOutcome::Pending
        }
        Reload::Idle | Reload::Handoff { .. } => ReloadOutcome::Pending,
    }
}

fn drain_forwarder_tail(
    rx: &Receiver<String>,
    activity: &mut ReloadProgress,
    drained: &mut Vec<String>,
) {
    let deadline = Instant::now() + RELOAD_TAIL_BUDGET;
    while Instant::now() < deadline {
        match rx.recv_timeout(RELOAD_TAIL_QUIET) {
            Ok(line) => {
                if !activity.observe(&line) {
                    activity.observe_failure(&line);
                    drained.push(line);
                }
            }
            Err(_) => return,
        }
    }
}

fn reload_failure_reason(status: impl std::fmt::Display, activity: &ReloadProgress) -> String {
    activity
        .coded_error
        .clone()
        .or_else(|| activity.summary_error.clone())
        .unwrap_or_else(|| status.to_string())
}

pub(super) fn restart_child_from_prebuilt(
    child: &mut TrayHandle,
    lines: &mut Receiver<String>,
    selection: &WorktreeSelection,
    updates: &HandoffUpdates,
) -> Result<()> {
    let prior = crate::commands::dev::current_active_worktree_marker();
    let desired = desired_marker(selection, prior.clone());
    if desired != prior {
        crate::commands::dev::persist_active_worktree(desired.as_deref())?;
    }
    let result = hand_off_to_prebuilt(child, lines, updates, desired.clone());
    if result.is_err() && desired != prior {
        match crate::commands::dev::persist_active_worktree(prior.as_deref()) {
            Ok(()) => updates.push_log("[qol dev] handoff failed: worktree selection rolled back"),
            Err(error) => {
                updates.push_log(format!("[qol dev] selection rollback failed: {error:#}"))
            }
        }
    }
    result
}

fn desired_marker(selection: &WorktreeSelection, prior: Option<String>) -> Option<String> {
    match selection {
        WorktreeSelection::Follow => prior,
        WorktreeSelection::Pin(target) => target.clone(),
    }
}

fn address_monitor_handoff_to_successor(updates: &HandoffUpdates, successor_id: &str) {
    let Some(runtime) = qol_config::runtime_dir() else {
        updates.push_log("[qol dev] monitor handoff not addressed: runtime dir unavailable");
        return;
    };
    let socket = runtime.join("sockets").join(MONITOR_DAEMON_SOCKET_FILE);
    if !socket.exists() {
        updates.push_log(
            "[qol dev] monitor handoff not addressed: predecessor monitor socket absent; relying on SIGHUP",
        );
        return;
    }
    let config = qol_plugin_daemon::daemon::DaemonConfig {
        socket: qol_plugin_daemon::daemon::SocketSource::Path(socket),
        support_replace_existing: true,
    };
    let delivered = qol_plugin_daemon::daemon::send_request(
        &config,
        "handoff",
        serde_json::json!({ "generation": successor_id }),
        MONITOR_HANDOFF_TIMEOUT,
    )
    .is_ok();
    if delivered {
        updates.push_log(format!(
            "[qol dev] monitor handoff addressed to successor generation {successor_id}"
        ));
    } else {
        updates.push_log(
            "[qol dev] monitor handoff socket delivery failed; relying on SIGHUP fallback",
        );
    }
}

fn hand_off_to_prebuilt(
    child: &mut TrayHandle,
    lines: &mut Receiver<String>,
    updates: &HandoffUpdates,
    marker: Option<String>,
) -> Result<()> {
    updates.push_log("[qol dev] starting successor generation");
    let predecessor_daemons = snapshot_runtime_daemon_pids();
    if !predecessor_daemons.is_empty() {
        updates.push_log(format!(
            "[qol dev] predecessor daemons tracked for handoff: {}",
            format_daemon_pids(&predecessor_daemons)
        ));
    }
    let root = crate::workspace::repo_root()?;
    let (target, note) = crate::commands::dev::marker_tray_target(&root, marker);
    if let Some(note) = note {
        updates.push_log(note);
    }
    let built_binary = crate::commands::dev::dev_binary_path(&target.root);
    updates.phase("stage", "tray executable");
    let runtime = qol_dev_build::tray::stage_runtime_generation(&root, &built_binary)
        .map_err(|error| anyhow::anyhow!("tray runtime staging failed: {error}"))?;
    let run_root = crate::commands::dev::dev_run_root(&target.root);
    updates.phase("start", "successor generation");
    let (next, next_lines, ready) = start_shadow_generation(&run_root, &runtime, updates)?;
    let mut next = TrayHandle::Owned(next);
    updates.phase("handoff", "plugin daemons");
    address_monitor_handoff_to_successor(updates, runtime.id());
    let remaining = handoff_daemon_groups(predecessor_daemons.clone());
    if remaining.is_empty() {
        updates.push_log("[qol dev] predecessor daemons handed off for reload");
    } else {
        updates.push_log(format!(
            "[qol dev] predecessor daemons did not hand off: {}",
            format_daemon_pids(&remaining)
        ));
    }
    updates.phase("stop", "previous generation");
    if let Err(error) = retire_child_for_handoff(child) {
        terminate_child(&mut next);
        let _ = next.wait();
        return Err(error);
    }
    updates.phase("wait", "previous daemons");
    if let Err(error) = wait_for_predecessor_daemons(predecessor_daemons, updates) {
        abandon_failed_successor(&mut next);
        return Err(error);
    }
    updates.phase("promote", "successor generation");
    if let Err(error) = promote_shadow_generation(ready.port, &mut next, &next_lines, updates) {
        updates.push_log(format!("[qol dev] successor promotion failed: {error:#}"));
        abandon_failed_successor(&mut next);
        return Err(error);
    }
    updates.adopt_running_worktree(run_root);
    *child = next;
    *lines = next_lines;
    updates.phase("cleanup", "runtime generations");
    if let Err(error) =
        qol_dev_build::tray::prune_runtime_generations(&root, &[runtime.executable()])
    {
        updates.push_log(format!("[qol dev] runtime prune failed: {error}"));
    }
    updates.phase("repair", "autostart target");
    repair_autostart_after_promotion(updates, &root);
    updates.push_log("[qol dev] successor generation active");
    Ok(())
}

fn repair_autostart_after_promotion(updates: &HandoffUpdates, root: &Path) {
    let binary = crate::workspace::doctor_binary_path(root);
    if !binary.is_file() {
        updates.push_log("[qol dev] autostart repair skipped: base doctor not built");
        return;
    }
    let output = autostart_repair_command(root, &binary).output();
    match output {
        Ok(out) if out.status.success() => {
            updates.push_log("[qol dev] autostart re-aligned to the promoted selection");
        }
        Ok(out) => updates.push_log(format!(
            "[qol dev] autostart repair failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(error) => updates.push_log(format!("[qol dev] autostart repair failed: {error}")),
    }
}

fn autostart_repair_command(root: &Path, binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .args([
            qol_conventions::doctor_cli::ARG_FIX,
            qol_conventions::doctor_cli::ARG_ID,
            "autostart_target",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn start_shadow_generation(
    root: &Path,
    runtime: &qol_dev_build::tray::StagedRuntimeGeneration,
    updates: &HandoffUpdates,
) -> Result<(Child, Receiver<String>, ShadowGenerationReady)> {
    let ready_file = shadow_ready_file(root, runtime.id());
    let _ = fs::remove_file(&ready_file);
    updates.push_log(format!(
        "[qol dev] booting successor generation {}",
        runtime.id()
    ));
    let mut command =
        shadow_generation_command(root, runtime.executable(), runtime.id(), &ready_file);
    crate::dev_shutdown::configure_tray_child(&mut command)?;
    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to start successor {}",
            runtime.executable().display()
        )
    })?;
    let rx = spawn_forwarders(&mut child);
    let ready = match wait_for_shadow_ready(&ready_file, &mut child, &rx, updates) {
        Ok(ready) => ready,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    updates.push_log(format!(
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
    updates: &HandoffUpdates,
) -> Result<()> {
    let deadline = Instant::now() + PROMOTION_TIMEOUT;
    let mut requested = false;
    let mut last_error = None;
    while Instant::now() < deadline {
        let _ = drain_shadow_logs(rx, updates);
        if let Some(status) = child.try_wait()? {
            bail!("successor generation exited during promotion: {status}");
        }
        if !requested {
            match post_promote_generation(port) {
                Ok(()) => {
                    requested = true;
                    updates.push_log("[qol dev] successor promotion requested");
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
    updates: &HandoffUpdates,
) -> Result<ShadowGenerationReady> {
    let deadline = Instant::now() + SHADOW_READY_TIMEOUT;
    let mut recent_logs = VecDeque::new();
    while Instant::now() < deadline {
        append_shadow_logs(&mut recent_logs, drain_shadow_logs(rx, updates));
        if ready_file.is_file() {
            return read_shadow_ready(ready_file);
        }
        if let Some(status) = child.try_wait()? {
            append_shadow_logs(&mut recent_logs, drain_shadow_logs(rx, updates));
            bail!(
                "shadow generation exited before ready: {status}{}",
                shadow_crash_detail(&recent_logs)
            );
        }
        std::thread::sleep(SHADOW_READY_INTERVAL);
    }
    append_shadow_logs(&mut recent_logs, drain_shadow_logs(rx, updates));
    bail!(
        "shadow generation did not become ready within {SHADOW_READY_TIMEOUT:?}{}",
        shadow_crash_detail(&recent_logs)
    )
}

fn drain_shadow_logs(rx: &Receiver<String>, updates: &HandoffUpdates) -> Vec<String> {
    let mut drained = Vec::new();
    while let Ok(line) = rx.try_recv() {
        let line = format!("[qol dev:shadow] {line}");
        updates.push_log(line.clone());
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

fn wait_for_predecessor_daemons(
    predecessor_daemons: Vec<TrackedDaemonPid>,
    updates: &HandoffUpdates,
) -> Result<()> {
    if predecessor_daemons.is_empty() {
        return Ok(());
    }
    updates.push_log(format!(
        "[qol dev] waiting for predecessor daemons to exit: {}",
        format_daemon_pids(&predecessor_daemons)
    ));
    let remaining = wait_for_daemons_to_exit(predecessor_daemons, PREDECESSOR_DAEMON_STOP_GRACE);
    if remaining.is_empty() {
        updates.push_log("[qol dev] predecessor daemons exited cleanly");
        return Ok(());
    }
    updates.push_log(format!(
        "[qol dev] predecessor daemons still alive; terminating groups: {}",
        format_daemon_pids(&remaining)
    ));
    let remaining = terminate_daemon_groups(remaining);
    let remaining = wait_for_daemons_to_exit(remaining, PREDECESSOR_DAEMON_STOP_GRACE);
    if !remaining.is_empty() {
        bail!(
            "predecessor daemon groups did not exit: {}",
            format_daemon_pids(&remaining)
        );
    }
    updates.push_log("[qol dev] predecessor daemon groups terminated");
    Ok(())
}

fn abandon_failed_successor(next: &mut TrayHandle) {
    terminate_child(next);
    let _ = next.wait();
}

fn retire_child_for_handoff(child: &mut TrayHandle) -> Result<()> {
    if post_shutdown().is_err() {
        terminate_child(child);
    }
    let deadline = Instant::now() + STOP_GRACE;
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
    fn successful_prebuild_keeps_elapsed_activity_for_the_handoff() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut activity = ReloadProgress::new();
        activity.started = Instant::now() - Duration::from_secs(8);
        let started = activity.started;
        let mut dash = Dash::new(Vec::new());
        dash.reload = Reload::Running {
            child,
            rx,
            activity,
            selection: WorktreeSelection::Follow,
        };

        assert!(matches!(poll_reload(&mut dash), ReloadOutcome::Ready));
        assert!(dash.is_reloading());
        let activity = dash.activity().expect("handoff remains an active reload");
        assert_eq!(activity.phase, "handoff");
        assert!(activity.elapsed >= Duration::from_secs(8));
        assert!(activity.elapsed <= started.elapsed());
    }

    #[test]
    fn reload_failure_reason_prefers_the_first_coded_diagnostic() {
        let mut activity = ReloadProgress::new();
        for line in [
            "qol dev: prebuild output",
            "error[E0425]: cannot find value `CURRENT_TIME` in module `xproto`",
            "error[E0601]: `main` function not found",
            "error: could not compile `qol-windowing` (lib) due to 1 previous error",
            "command failed with exit status: 101",
        ] {
            activity.observe_failure(line);
        }
        assert_eq!(
            reload_failure_reason("exit status: 1", &activity),
            "error[E0425]: cannot find value `CURRENT_TIME` in module `xproto`"
        );
    }

    #[test]
    fn reload_failure_reason_falls_back_to_the_compile_summary() {
        let mut activity = ReloadProgress::new();
        activity.observe_failure("qol dev: prebuild output");
        activity.observe_failure(
            "error: could not compile `qol-windowing` (lib) due to 1 previous error",
        );
        assert_eq!(
            reload_failure_reason("exit status: 1", &activity),
            "error: could not compile `qol-windowing` (lib) due to 1 previous error"
        );
    }

    #[test]
    fn reload_failure_reason_falls_back_to_the_exit_status() {
        assert_eq!(
            reload_failure_reason("exit status: 1", &ReloadProgress::new()),
            "exit status: 1"
        );
        let mut activity = ReloadProgress::new();
        activity.observe_failure("ordinary line");
        assert_eq!(
            reload_failure_reason("exit status: 1", &activity),
            "exit status: 1"
        );
    }

    #[test]
    fn observe_failure_keeps_the_first_coded_error_across_polls() {
        let mut activity = ReloadProgress::new();
        activity.observe_failure("\x1b[31merror[E0425]: cannot find value `CURRENT_TIME`\x1b[0m");
        activity.observe_failure("warning: unused import");
        activity.observe_failure("error[E0601]: `main` function not found");
        activity.observe_failure("error: could not compile `qol-windowing` (lib)");
        activity.observe_failure("error: later summary");

        assert_eq!(
            activity.coded_error.as_deref(),
            Some("error[E0425]: cannot find value `CURRENT_TIME`"),
            "the first coded diagnostic drives the reason even when a later poll sees another"
        );
        assert_eq!(
            activity.summary_error.as_deref(),
            Some("error: later summary"),
            "the summary tracks the last compile-level error"
        );
    }

    #[test]
    fn spawn_error_records_the_pinned_target_in_the_failure() {
        let mut dash = Dash::new(Vec::new());
        let selection = WorktreeSelection::Pin(Some("feat/x".to_string()));
        let error = anyhow::anyhow!("failed to resolve qol workspace root");

        record_spawn_error(&mut dash, &selection, &error);

        let failure = dash.reload_failure.as_deref().expect("failure recorded");
        assert!(failure.starts_with("switch to feat/x failed"), "{failure}");
        assert!(
            failure.contains("failed to resolve qol workspace root"),
            "{failure}"
        );
        assert!(dash
            .logs
            .ring
            .lines
            .iter()
            .any(|line| line.contains("reload failed to start")));
    }

    #[test]
    #[cfg(unix)]
    fn failure_records_the_spawn_time_target_after_the_selection_changes() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo 'error[E0425]: cannot find value `CURRENT_TIME` in module `xproto`' >&2; exit 1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let rx = spawn_forwarders(&mut child);
        assert!(!child.wait().unwrap().success());
        let mut dash = Dash::new(Vec::new());
        dash.reload = Reload::Running {
            child,
            rx,
            activity: ReloadProgress::new(),
            selection: WorktreeSelection::Pin(Some("monitor-display".to_string())),
        };
        dash.worktree_selection = WorktreeSelection::Pin(Some("somewhere-else".to_string()));

        assert!(matches!(poll_reload(&mut dash), ReloadOutcome::Pending));

        let failure = dash.reload_failure.as_deref().expect("failure recorded");
        assert!(
            failure.starts_with("switch to monitor-display failed"),
            "{failure}"
        );
        assert!(!failure.contains("somewhere-else"), "{failure}");
    }

    #[test]
    fn successful_reload_clears_a_seeded_failure() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut dash = Dash::new(Vec::new());
        dash.record_reload_failure(
            &WorktreeSelection::Pin(Some("feat/x".to_string())),
            "exit status: 1",
        );
        assert!(dash.reload_failure.is_some());
        dash.reload = Reload::Running {
            child,
            rx,
            activity: ReloadProgress::new(),
            selection: WorktreeSelection::Follow,
        };

        assert!(matches!(poll_reload(&mut dash), ReloadOutcome::Ready));
        assert!(dash.reload_failure.is_none(), "success clears the failure");
        assert!(
            dash.notice.is_none(),
            "the failure notice is removed with the failure"
        );
    }

    #[test]
    #[cfg(unix)]
    fn failed_prebuild_records_a_visible_reason_and_keeps_the_pending_target() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(concat!(
                "echo 'error[E0425]: cannot find value `CURRENT_TIME` in module `xproto`' >&2; ",
                "echo 'error: could not compile `qol-windowing` (lib) due to 1 previous error' >&2; ",
                "exit 1"
            ))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let rx = spawn_forwarders(&mut child);
        assert!(!child.wait().unwrap().success());
        let mut dash = Dash::new(Vec::new());
        dash.worktree_selection = WorktreeSelection::Pin(Some("monitor-display".to_string()));
        dash.reload = Reload::Running {
            child,
            rx,
            activity: ReloadProgress::new(),
            selection: WorktreeSelection::Pin(Some("monitor-display".to_string())),
        };

        assert!(matches!(poll_reload(&mut dash), ReloadOutcome::Pending));

        let failure = dash.reload_failure.as_deref().expect("failure recorded");
        assert!(
            failure.starts_with("switch to monitor-display failed"),
            "{failure}"
        );
        assert!(failure.contains("error[E0425]"), "{failure}");
        assert_eq!(
            dash.notice.as_ref().expect("notice recorded").1,
            failure,
            "the visible notice must carry the recorded reason"
        );
        assert!(
            dash.worktree_diverged(),
            "a failed switch must keep the target for a retry"
        );
        assert!(
            dash.logs
                .ring
                .lines
                .iter()
                .any(|line| line.contains("reload aborted")),
            "the abort must stay in the log"
        );

        dash.clear_reload_failure();
        assert!(dash.reload_failure.is_none());
    }

    #[test]
    fn reload_activity_accepts_only_structured_worker_progress() {
        let mut activity = ReloadProgress::new();
        let cases = [
            (
                format!(
                    "{}plugins\tdev-linked plugins",
                    crate::commands::dev::DEV_RELOAD_PROGRESS_PREFIX
                ),
                true,
                "plugins",
                "dev-linked plugins",
            ),
            (
                "[qol dev] ordinary worker output".to_string(),
                false,
                "plugins",
                "dev-linked plugins",
            ),
            (
                crate::commands::dev::DEV_RELOAD_PROGRESS_PREFIX.to_string(),
                false,
                "plugins",
                "dev-linked plugins",
            ),
        ];

        for (line, observed, phase, detail) in cases {
            assert_eq!(activity.observe(&line), observed, "line={line:?}");
            assert_eq!(activity.phase, phase);
            assert_eq!(activity.detail, detail);
        }
    }

    #[test]
    fn armed_reload_follow_defers_to_the_persisted_marker() {
        let root = Path::new("/repo/qol");
        let exe = Path::new("/bin/qol");
        let command = reload_prebuild_command(
            root,
            exe,
            ["-n", "dev", "feat/x", "-v"].map(std::ffi::OsString::from),
            ReloadTargetArg::Passthrough,
        );
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(
            args,
            [crate::commands::dev::DEV_PREBUILD_COMMAND, "-v", "-n"].map(OsStr::new),
            "follow must not forward the argv branch; the prebuild reads the marker"
        );
        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(command.get_program(), exe.as_os_str());
    }

    #[test]
    fn reload_executable_prefers_the_current_binary_when_it_still_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let current = tmp.path().join("target/debug/qol");
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(&current, "").unwrap();

        let got = reload_executable_path(&current, Some(tmp.path()));

        assert_eq!(got, current);
    }

    #[test]
    fn reload_executable_falls_back_to_installed_qol_when_current_binary_is_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let current = tmp.path().join("target/debug/qol");
        let expected = tmp
            .path()
            .join(".cargo")
            .join("bin")
            .join(crate::workspace::exe_name("qol"));

        let got = reload_executable_path(&current, Some(tmp.path()));

        assert_eq!(got, expected);
    }

    #[test]
    fn reload_executable_falls_back_to_path_lookup_without_home() {
        let tmp = tempfile::TempDir::new().unwrap();
        let current = tmp.path().join("target/debug/qol");

        let got = reload_executable_path(&current, None);

        assert_eq!(got, PathBuf::from(crate::workspace::exe_name("qol")));
    }

    #[test]
    fn armed_reload_uses_selected_worktree_target_over_argv() {
        let args = reload_prebuild_args(
            ["-n", "dev", "argv-branch", "-v"].map(std::ffi::OsString::from),
            ReloadTargetArg::Branch("panel-branch".to_string()),
        );
        let got: Vec<&OsStr> = args.iter().map(|arg| arg.as_os_str()).collect();
        assert_eq!(
            got,
            ["-v", "-n", "panel-branch"].map(OsStr::new),
            "panel selection must override startup argv branch"
        );
    }

    #[test]
    fn armed_reload_can_explicitly_select_base_over_argv() {
        let args = reload_prebuild_args(
            ["dev", "argv-branch"].map(std::ffi::OsString::from),
            ReloadTargetArg::Base,
        );
        let got: Vec<&OsStr> = args.iter().map(|arg| arg.as_os_str()).collect();
        assert_eq!(
            got,
            [crate::commands::dev::DEV_PREBUILD_BASE_ARG].map(OsStr::new),
            "explicit base target must clear a startup argv branch"
        );
    }

    #[test]
    fn desired_marker_commits_pins_and_follows_the_prior_selection() {
        let cases = [
            (WorktreeSelection::Follow, Some("feat/x"), Some("feat/x")),
            (WorktreeSelection::Follow, None, None),
            (
                WorktreeSelection::Pin(Some("feat/y".to_string())),
                Some("feat/x"),
                Some("feat/y"),
            ),
            (WorktreeSelection::Pin(None), Some("feat/x"), None),
        ];
        for (selection, prior, expected) in cases {
            assert_eq!(
                desired_marker(&selection, prior.map(str::to_string)),
                expected.map(str::to_string),
                "selection: {selection:?} prior: {prior:?}"
            );
        }
    }

    #[test]
    fn autostart_repair_runs_the_scoped_doctor_fix_at_the_base_root() {
        let root = Path::new("/repo/qol");
        let binary = Path::new("/repo/qol/target/debug/qol-tray-doctor");
        let command = autostart_repair_command(root, binary);
        assert_eq!(command.get_program(), binary.as_os_str());
        assert_eq!(command.get_current_dir(), Some(root));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["fix", "--id", "autostart_target"].map(OsStr::new),
            "repair must be the scoped autostart fix, never a full fix run"
        );
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

    #[test]
    #[cfg(unix)]
    fn monitor_handoff_delivers_over_a_path_addressed_socket_without_env_use() {
        use std::io::Write as _;

        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("monitor.sock");
        let listener = qol_runtime::local_ipc::bind_listener(&socket_path).unwrap();

        let successor_id = "handoff-gen-42";
        let observed = std::sync::Arc::new(std::sync::Mutex::new(None));
        let observed_for_thread = observed.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let request: qol_runtime::protocol::DaemonRequest = serde_json::from_str(
                &qol_runtime::local_ipc::read_line(&mut reader)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            *observed_for_thread.lock().unwrap() = Some(request);
            let reply = serde_json::to_string(&qol_runtime::protocol::DaemonResponse::Handled {
                data: None,
            })
            .unwrap();
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
        });

        let config = qol_plugin_daemon::daemon::DaemonConfig {
            socket: qol_plugin_daemon::daemon::SocketSource::Path(socket_path.clone()),
            support_replace_existing: true,
        };
        assert_eq!(
            qol_plugin_daemon::daemon::socket_path(&config),
            Some(socket_path.clone()),
            "Path config must resolve directly, ignoring the daemon-socket env var"
        );
        let prior_daemon_socket = std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET);
        let delivered = qol_plugin_daemon::daemon::send_request(
            &config,
            "handoff",
            serde_json::json!({ "generation": successor_id }),
            MONITOR_HANDOFF_TIMEOUT,
        )
        .is_ok();
        server.join().unwrap();

        assert!(
            delivered,
            "handoff send over the path-addressed socket must succeed"
        );
        let request = observed.lock().unwrap().clone();
        assert_eq!(request.as_ref().unwrap().action, "handoff");
        assert_eq!(
            request.unwrap().input["generation"],
            serde_json::json!(successor_id)
        );
        assert_eq!(
            std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET),
            prior_daemon_socket,
            "the daemon-socket env must be untouched by a path-addressed handoff"
        );
    }

    #[test]
    fn abandon_failed_successor_terminates_and_reaps() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut next = TrayHandle::Owned(child);

        abandon_failed_successor(&mut next);

        assert!(
            next.try_wait().unwrap().is_some(),
            "failed successor must be reaped, not left running"
        );
    }
}
