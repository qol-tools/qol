//! Linux Mint implementation of the Alt Tab adversarial workflow.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;
use serde_json::{json, Value};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, dispatch_plugin_action, fd_count,
    install_payload, plugin_daemon_pid, require_exec, require_plugin_action_guards, spawn,
    start_tray_and_wait_plugin_with_setup, trace_tail_command, wait_for_command,
    wait_for_probe_fields, wait_for_probe_line, wait_for_window_id, wait_for_window_title,
    within_fd_budget, xdotool_key, TraceCursor,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const PLUGIN_ID: &str = "plugin-alt-tab";
const PICKER_PREFIX: &str = "qol-alt-tab-picker@";
const CINNAMON_EXTENSION_UUID: &str = "qol-alt-tab-preview-plane@qol-tools";
const CINNAMON_EXTENSION_PARENT: &str = "/home/qol/.local/share/cinnamon/extensions";
const CINNAMON_EXTENSION_ROOT: &str =
    "/home/qol/.local/share/cinnamon/extensions/qol-alt-tab-preview-plane@qol-tools";
const LEGACY_EXTENSION_TARGET: &str = concat!(
    "/home/qol/qol-monorepo/plugins/plugin-alt-tab/shell/cinnamon/",
    "qol-alt-tab-preview-plane@qol-tools"
);
const CINNAMON_EXTENSION_FILES: [&str; 3] =
    ["extension.js", "generated-theme-tokens.js", "metadata.json"];
const FIXTURE_COUNT: usize = 8;
const FRESH_CLOSE_CYCLES: usize = 3;
const RETAINED_CYCLES: usize = 50;
const KEY_CYCLES: usize = 240;
const PERFORMANCE_KEY_CYCLES: usize = 120;
const PERFORMANCE_RETAINED_CYCLES: usize = 50;

pub(super) fn run_performance(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let (auth, integration_cursor) = start_tray_and_wait_plugin_with_setup(
        &mut guest,
        PLUGIN_ID,
        seed_legacy_extension_install,
    )?;
    launch_fixtures(&mut guest)?;
    set_sticky_config(&mut guest, &auth)?;

    let first_cursor = dispatch(&mut guest, &auth, "open")?;
    wait_for_probe_fields(
        &mut guest,
        integration_cursor,
        "PREVIEW_PLANE_INTEGRATION",
        &["outcome=ready", "root=migrated_symlink", "reloaded=true"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        &mut guest,
        first_cursor,
        "RENDERING_FLOW",
        &[
            "preview_renderer=external_preview_plane",
            "backend=cinnamon_shell",
            "gpui_preview_images=false",
        ],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        &mut guest,
        first_cursor,
        "PREVIEW_PLANE_SHOW",
        &["backend=cinnamon_shell", "outcome=ok"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_line(
        &mut guest,
        first_cursor,
        "SHOW_PAINTED",
        "show_id=",
        ACTION_TIMEOUT,
    )?;
    wait_for_focus_title(
        &mut guest,
        |title| title.starts_with(PICKER_PREFIX),
        "Alt Tab picker",
    )?;
    dismiss_sticky_performance(&mut guest)?;

    let metrics_cursor = current_trace_cursor(&mut guest)?;
    let pid_before = daemon_pid(&mut guest)?;
    let fds_before = fd_count(&mut guest, &pid_before)?;
    for _ in 0..PERFORMANCE_RETAINED_CYCLES {
        open_sticky_performance(&mut guest, &auth)?;
        dismiss_sticky_performance(&mut guest)?;
    }

    open_sticky_performance(&mut guest, &auth)?;
    let key_cursor = current_trace_cursor(&mut guest)?;
    for index in 0..PERFORMANCE_KEY_CYCLES {
        let key_name = if index % 5 == 0 { "shift+Tab" } else { "Tab" };
        key(&mut guest, key_name)?;
    }
    wait_for_probe_fields(
        &mut guest,
        key_cursor,
        "CYCLE",
        &["method=tab", &format!("count={FIXTURE_COUNT}")],
        ACTION_TIMEOUT,
    )?;
    dismiss_sticky_performance(&mut guest)?;

    let pid_after = daemon_pid(&mut guest)?;
    if pid_before != pid_after {
        bail!("Alt Tab restarted during Cinnamon performance storm");
    }
    let fds_after = fd_count(&mut guest, &pid_after)?;
    if !within_fd_budget(fds_before, fds_after) {
        bail!("Alt Tab file descriptors grew from {fds_before} to {fds_after}");
    }

    let probes = require_exec(
        &mut guest,
        trace_tail_command(metrics_cursor)?,
        COMMAND_TIMEOUT,
    )?;
    let trace_lines: Vec<String> = probes
        .stdout
        .lines()
        .filter(|line| is_performance_probe(line))
        .map(str::to_string)
        .collect();
    let metrics = performance_metrics(&trace_lines, fds_before, fds_after)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let metrics_path = artifacts_dir.join("alt-tab-performance.json");
    std::fs::write(&metrics_path, serde_json::to_vec_pretty(&metrics)?)
        .with_context(|| format!("failed to write {}", metrics_path.display()))?;
    let mut traces = trace_lines;
    traces.push(performance_trace(&metrics));
    step_label(
        "performance",
        StepKind::Success,
        &format!(
            "Cinnamon show/preview storm passed: {PERFORMANCE_RETAINED_CYCLES} retained cycles, {PERFORMANCE_KEY_CYCLES} navigation keys, fd={fds_before}->{fds_after}"
        ),
    );
    Ok(Verdict {
        pass: true,
        traces,
        artifacts: vec![metrics_path],
    })
}

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let (auth, integration_cursor) = start_tray_and_wait_plugin_with_setup(
        &mut guest,
        PLUGIN_ID,
        seed_legacy_extension_install,
    )?;
    launch_fixtures(&mut guest)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;

    test_hold_mode(&mut guest, &auth, integration_cursor)?;
    set_sticky_config(&mut guest, &auth)?;
    let picker = artifacts_dir.join("picker.ppm");
    test_sticky_input_storm(&mut guest, &auth, &mut qmp, &picker)?;
    test_fresh_window_actions(&mut guest, &auth)?;
    test_retained_cycles(&mut guest, &auth)?;
    let settings = artifacts_dir.join("settings.ppm");
    test_settings(&mut guest, &auth, &mut qmp, &settings)?;
    test_http_guards(&mut guest, &auth)?;
    test_crash_recovery(&mut guest, &auth)?;

    let final_state = artifacts_dir.join("final.ppm");
    qmp.screendump(&final_state)?;
    let probes = require_exec(
        &mut guest,
        command(
            "/usr/bin/grep",
            &[
                "-E",
                "ACTIVATE_WIN|CLOSE_WIN|CMD_RECV|DISMISS|FOCUS_REASSERT|KEY_RECV|NAV_GRID|PREVIEW_PLANE_(INTEGRATION|SHOW)|QUIT_APP|RENDERING_FLOW|SHOW_(CYCLE_FAST|LIST|PAINTED|RECV)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        "Cinnamon legacy-link migration, compositor previews, hold mode, fresh close/quit actions, 8-window churn, 240 keys, 50 retained cycles, settings, guards, and crash recovery passed",
    );
    Ok(Verdict {
        pass: true,
        traces: probes.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![picker, settings, final_state],
    })
}

fn seed_legacy_extension_install(guest: &mut GuestControlClient) -> Result<TraceCursor> {
    step_label(
        "legacy-link",
        StepKind::Pending,
        "seeding the pre-rename Cinnamon source symlink",
    );
    require_exec(
        guest,
        command("/usr/bin/rm", &["-rf", "--", CINNAMON_EXTENSION_ROOT]),
        COMMAND_TIMEOUT,
    )?;
    require_exec(
        guest,
        command("/usr/bin/mkdir", &["-p", CINNAMON_EXTENSION_PARENT]),
        COMMAND_TIMEOUT,
    )?;
    require_exec(
        guest,
        command(
            "/usr/bin/ln",
            &[
                "--symbolic",
                LEGACY_EXTENSION_TARGET,
                CINNAMON_EXTENSION_ROOT,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    require_exec(
        guest,
        command(
            "/usr/bin/gsettings",
            &[
                "set",
                "org.cinnamon",
                "enabled-extensions",
                &format!("['{CINNAMON_EXTENSION_UUID}']"),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    require_exec(
        guest,
        command("/usr/bin/touch", &[TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    let cursor = current_trace_cursor(guest)?;
    step_label(
        "legacy-link",
        StepKind::Success,
        "broken source symlink enabled before the production daemon starts",
    );
    Ok(cursor)
}

fn launch_fixtures(guest: &mut GuestControlClient) -> Result<()> {
    for index in 0..FIXTURE_COUNT {
        let title = format!("qol-alt-tab-storm-{index:02}");
        let geometry = format!(
            "42x10+{}+{}",
            40 + (index % 4) * 285,
            70 + (index / 4) * 290
        );
        spawn(
            guest,
            command("/usr/bin/xterm", &["-T", &title, "-geometry", &geometry]),
        )?;
        wait_for_window_id(guest, &title, ACTION_TIMEOUT)?;
    }
    Ok(())
}

fn dispatch(guest: &mut GuestControlClient, auth: &str, action: &str) -> Result<TraceCursor> {
    dispatch_plugin_action(guest, auth, PLUGIN_ID, action, "{}", ACTION_TIMEOUT)
}

fn set_sticky_config(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let url = format!("{}/api/plugins/{PLUGIN_ID}/config", local_base_url());
    let body =
        r#"{"action_mode":"sticky","open_behavior":"show_only","reset_selection_on_open":true}"#;
    let cursor = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command(
            "/usr/bin/curl",
            &[
                "--fail",
                "--silent",
                "--show-error",
                "--header",
                auth,
                "--header",
                "Content-Type: application/json",
                "--request",
                "PUT",
                "--data",
                body,
                &url,
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_line(guest, cursor, "CMD_RECV", "cmd=reload", ACTION_TIMEOUT)?;
    Ok(())
}

fn key(guest: &mut GuestControlClient, value: &str) -> Result<()> {
    xdotool_key(guest, value, true)
}

fn open_sticky(guest: &mut GuestControlClient, auth: &str) -> Result<TraceCursor> {
    let cursor = dispatch(guest, auth, "open")?;
    wait_for_probe_line(guest, cursor, "SHOW_PAINTED", "show_id=", ACTION_TIMEOUT)?;
    wait_for_focus_title(
        guest,
        |title| title.starts_with(PICKER_PREFIX),
        "Alt Tab picker",
    )?;
    Ok(cursor)
}

fn open_sticky_performance(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = open_sticky(guest, auth)?;
    wait_for_probe_fields(
        guest,
        cursor,
        "PREVIEW_PLANE_SHOW",
        &["backend=cinnamon_shell", "outcome=ok"],
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn dismiss_sticky(guest: &mut GuestControlClient) -> Result<TraceCursor> {
    let cursor = current_trace_cursor(guest)?;
    key(guest, "Escape")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "DISMISS",
        &["from=key/escape", "active=true"],
        ACTION_TIMEOUT,
    )?;
    Ok(cursor)
}

fn dismiss_sticky_performance(guest: &mut GuestControlClient) -> Result<()> {
    let cursor = dismiss_sticky(guest)?;
    wait_for_probe_fields(
        guest,
        cursor,
        "PREVIEW_PLANE_HIDE",
        &["backend=cinnamon_shell", "outcome=ok"],
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn wait_for_focus_title(
    guest: &mut GuestControlClient,
    predicate: impl Fn(&str) -> bool,
    description: &str,
) -> Result<()> {
    let result = wait_for_window_title(
        guest,
        &["getwindowfocus", "getwindowname"],
        predicate,
        description,
        ACTION_TIMEOUT,
    );
    let Err(error) = result else {
        return Ok(());
    };
    let trace = require_exec(
        guest,
        command("/usr/bin/tail", &["-n", "120", TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    bail!("{error:#}; final Alt Tab trace:\n{}", trace.stdout.trim());
}

fn test_hold_mode(
    guest: &mut GuestControlClient,
    auth: &str,
    integration_cursor: TraceCursor,
) -> Result<()> {
    let cursor = dispatch(guest, auth, "open")?;
    wait_for_probe_fields(
        guest,
        integration_cursor,
        "PREVIEW_PLANE_INTEGRATION",
        &["outcome=ready", "root=migrated_symlink", "reloaded=true"],
        ACTION_TIMEOUT,
    )?;
    verify_owned_extension_install(guest)?;
    wait_for_probe_fields(
        guest,
        cursor,
        "RENDERING_FLOW",
        &[
            "preview_renderer=external_preview_plane",
            "backend=cinnamon_shell",
            "gpui_preview_images=false",
        ],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "PREVIEW_PLANE_SHOW",
        &["backend=cinnamon_shell", "outcome=ok"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "DISMISS",
        &["from=alt-release/poll", "active=true"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_line(
        guest,
        cursor,
        "ACTIVATE_WIN",
        "title=\"qol-alt-tab-storm-",
        ACTION_TIMEOUT,
    )?;
    step_label(
        "hold-mode",
        StepKind::Success,
        "Cinnamon compositor rendered live previews before modifier release activated a fixture",
    );
    Ok(())
}

fn verify_owned_extension_install(guest: &mut GuestControlClient) -> Result<()> {
    let root = require_exec(
        guest,
        command(
            "/usr/bin/find",
            &[
                CINNAMON_EXTENSION_ROOT,
                "-maxdepth",
                "0",
                "-type",
                "d",
                "-print",
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    if root.stdout.trim() != CINNAMON_EXTENSION_ROOT {
        bail!("Cinnamon extension root remained a symlink after migration");
    }
    for name in CINNAMON_EXTENSION_FILES {
        let path = format!("{CINNAMON_EXTENSION_ROOT}/{name}");
        require_exec(
            guest,
            command("/usr/bin/test", &["-f", &path]),
            COMMAND_TIMEOUT,
        )?;
    }
    Ok(())
}

fn test_sticky_input_storm(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    artifact: &std::path::Path,
) -> Result<()> {
    open_sticky(guest, auth)?;
    qmp.screendump(artifact)?;
    let pid_before = daemon_pid(guest)?;
    let fds_before = fd_count(guest, &pid_before)?;
    let cursor = current_trace_cursor(guest)?;
    for index in 0..KEY_CYCLES {
        let key_name = if index % 5 == 0 { "shift+Tab" } else { "Tab" };
        key(guest, key_name)?;
    }
    for key_name in ["Right", "Left", "Down", "Up"] {
        key(guest, key_name)?;
    }
    wait_for_probe_fields(
        guest,
        cursor,
        "NAV_GRID",
        &["method=arrow-up", &format!("count={FIXTURE_COUNT}")],
        ACTION_TIMEOUT,
    )?;
    let action_cursor = dispatch(guest, auth, "open-reverse")?;
    wait_for_probe_fields(
        guest,
        action_cursor,
        "SHOW_CYCLE_FAST",
        &["cycled=false"],
        ACTION_TIMEOUT,
    )?;
    key(guest, "w")?;
    wait_for_probe_fields(
        guest,
        action_cursor,
        "KEY_RECV",
        &["key=\"w\"", "visible=true"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        action_cursor,
        "CLOSE_WIN",
        &["outcome=sent"],
        ACTION_TIMEOUT,
    )?;
    wait_for_fixture_count(guest, FIXTURE_COUNT - 1)?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOW_LIST",
        &["path=ghost", &format!("n={}", FIXTURE_COUNT - 1)],
        ACTION_TIMEOUT,
    )?;
    let pid_after = daemon_pid(guest)?;
    if pid_before != pid_after {
        bail!("Alt Tab restarted during sticky input storm");
    }
    let fds_after = fd_count(guest, &pid_after)?;
    if !within_fd_budget(fds_before, fds_after) {
        bail!("Alt Tab file descriptors grew from {fds_before} to {fds_after}");
    }
    key(guest, "Return")?;
    wait_for_focus_title(
        guest,
        |title| title.starts_with("qol-alt-tab-storm-"),
        "selected fixture",
    )?;
    step_label(
        "input",
        StepKind::Success,
        &format!("{KEY_CYCLES} keys retained pid={pid_after} fds={fds_after}"),
    );
    Ok(())
}

fn test_fresh_window_actions(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let mut expected = FIXTURE_COUNT - 1;
    for _ in 0..FRESH_CLOSE_CYCLES {
        let cursor = dispatch(guest, auth, "open")?;
        wait_for_probe_line(guest, cursor, "SHOW_PAINTED", "show_id=", ACTION_TIMEOUT)?;
        key(guest, "w")?;
        wait_for_probe_fields(
            guest,
            cursor,
            "KEY_RECV",
            &["key=\"w\"", "visible=true"],
            ACTION_TIMEOUT,
        )?;
        wait_for_probe_fields(
            guest,
            cursor,
            "CLOSE_WIN",
            &["outcome=sent"],
            ACTION_TIMEOUT,
        )?;
        expected -= 1;
        wait_for_fixture_count(guest, expected)?;
        dismiss_sticky(guest)?;
    }

    let cursor = dispatch(guest, auth, "open")?;
    wait_for_probe_line(guest, cursor, "SHOW_PAINTED", "show_id=", ACTION_TIMEOUT)?;
    key(guest, "q")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "KEY_RECV",
        &["key=\"q\"", "visible=true"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "QUIT_APP",
        &["outcome=sigterm_sent", "pid="],
        ACTION_TIMEOUT,
    )?;
    expected -= 1;
    wait_for_fixture_count(guest, expected)?;
    dismiss_sticky(guest)?;
    step_label(
        "window-actions",
        StepKind::Success,
        &format!("{FRESH_CLOSE_CYCLES} fresh-show closes and one fresh-show quit reached XTerm"),
    );
    Ok(())
}

fn test_retained_cycles(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let pid_before = daemon_pid(guest)?;
    let fds_before = fd_count(guest, &pid_before)?;
    for _ in 0..RETAINED_CYCLES {
        open_sticky(guest, auth)?;
        dismiss_sticky(guest)?;
    }
    let pid_after = daemon_pid(guest)?;
    if pid_before != pid_after {
        bail!("Alt Tab restarted during retained show/dismiss cycles");
    }
    let fds_after = fd_count(guest, &pid_after)?;
    if !within_fd_budget(fds_before, fds_after) {
        bail!("Alt Tab file descriptors grew from {fds_before} to {fds_after}");
    }
    Ok(())
}

fn test_settings(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    artifact: &std::path::Path,
) -> Result<()> {
    dispatch(guest, auth, "settings")?;
    wait_for_focus_title(
        guest,
        |title| title.starts_with("Alt Tab Settings"),
        "native Alt Tab Settings",
    )?;
    qmp.screendump(artifact)?;
    key(guest, "Escape")
}

fn test_http_guards(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    require_plugin_action_guards(guest, auth, PLUGIN_ID, "open")
}

fn test_crash_recovery(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    open_sticky(guest, auth)?;
    let before = daemon_pid(guest)?;
    require_exec(
        guest,
        command("/usr/bin/kill", &["--signal", "KILL", &before]),
        COMMAND_TIMEOUT,
    )?;
    wait_for_visible_picker_count(guest, 0)?;
    let outcome = wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["-x", "alt-tab"]),
        ACTION_TIMEOUT,
        |outcome| {
            outcome
                .stdout
                .lines()
                .next()
                .is_some_and(|pid| pid.trim() != before)
        },
        "Alt Tab daemon restart",
    )?;
    let after = outcome
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .context("Alt Tab daemon restart returned no PID")?;
    let recovery_cursor = current_trace_cursor(guest)?;
    open_sticky(guest, auth)?;
    wait_for_probe_line(
        guest,
        recovery_cursor,
        "FOCUS_REASSERT",
        "title=qol-alt-tab-picker@",
        ACTION_TIMEOUT,
    )?;
    dismiss_sticky(guest)?;
    step_label(
        "recovery",
        StepKind::Success,
        &format!("Alt Tab recovered pid={before}->{after} without a stale picker"),
    );
    Ok(())
}

const PERFORMANCE_PROBE_TAGS: [&str; 6] = [
    "KEY_RECV",
    "NAV_GRID",
    "PREVIEW_PLANE_SHOW",
    "PREVIEW_PLANE_HIDE",
    "SHOW_PAINTED",
    "SHOW_RECV",
];

fn is_performance_probe(line: &str) -> bool {
    PERFORMANCE_PROBE_TAGS
        .iter()
        .any(|tag| line.contains(&format!(" {tag} ")))
}

fn performance_metrics(traces: &[String], fds_before: u64, fds_after: u64) -> Result<Value> {
    let mut show_received = HashMap::new();
    let mut show_painted = HashMap::new();
    let mut plane_elapsed = BTreeMap::<usize, Vec<u64>>::new();
    let mut plane_build = BTreeMap::<usize, Vec<u64>>::new();
    let mut plane_calls = 0_u64;
    let mut reused_connections = 0_u64;
    let mut plane_recoveries = 0_u64;
    let mut plane_errors = 0_u64;

    for trace in traces {
        let Some(timestamp) = trace_timestamp(trace) else {
            continue;
        };
        if trace.contains(" SHOW_RECV ") {
            if let Some(show_id) = trace_value(trace, "show_id=") {
                show_received.insert(show_id, timestamp);
            }
            continue;
        }
        if trace.contains(" SHOW_PAINTED ") {
            if let (Some(show_id), Some(frame_ms)) =
                (trace_value(trace, "show_id="), trace_value(trace, "frame="))
            {
                show_painted.insert(show_id, (timestamp, frame_ms));
            }
            continue;
        }
        if trace.contains(" PREVIEW_PLANE_HIDE ") {
            if trace.contains("outcome=error") {
                plane_errors += 1;
            }
            if let Some(attempts) = trace_value(trace, "recovery_attempts=") {
                plane_recoveries += attempts;
            }
            continue;
        }
        if !trace.contains("PREVIEW_PLANE_SHOW") || !trace.contains("outcome=ok") {
            if trace.contains("PREVIEW_PLANE_SHOW") && trace.contains("outcome=error") {
                plane_errors += 1;
            }
            continue;
        }
        let Some(items) = trace_value(trace, "items=") else {
            continue;
        };
        let Some(elapsed_ms) = trace_value(trace, "elapsed=") else {
            continue;
        };
        let Some(build_ms) = trace_value(trace, "build_ms\":") else {
            continue;
        };
        plane_elapsed
            .entry(items as usize)
            .or_default()
            .push(elapsed_ms);
        plane_build
            .entry(items as usize)
            .or_default()
            .push(build_ms);
        plane_calls += 1;
        if trace.contains("reused_connection=true") {
            reused_connections += 1;
        }
        if let Some(attempts) = trace_value(trace, "recovery_attempts=") {
            plane_recoveries += attempts;
        }
    }

    let show_to_paint = show_painted
        .iter()
        .filter_map(|(show_id, (painted_at, _))| {
            show_received
                .get(show_id)
                .map(|received_at| painted_at.saturating_sub(*received_at) as u64)
        })
        .collect::<Vec<_>>();
    let frames: Vec<u64> = show_painted.values().map(|(_, frame)| *frame).collect();
    let plane_elapsed = plane_elapsed
        .into_iter()
        .map(|(items, values)| (items.to_string(), metric_stats(&values)))
        .collect::<BTreeMap<_, _>>();
    let plane_build = plane_build
        .into_iter()
        .map(|(items, values)| (items.to_string(), metric_stats(&values)))
        .collect::<BTreeMap<_, _>>();
    let show_stats = metric_stats(&show_to_paint);
    let frame_stats = metric_stats(&frames);
    let show_count = show_stats["count"].as_u64().unwrap_or(0);
    if show_count == 0 || plane_calls == 0 {
        bail!("performance storm produced no measurable show/preview samples");
    }
    if plane_errors > 0 {
        bail!("performance storm saw {plane_errors} Cinnamon preview-plane errors");
    }
    Ok(json!({
        "fixture_count": FIXTURE_COUNT,
        "retained_cycles": PERFORMANCE_RETAINED_CYCLES,
        "key_cycles": PERFORMANCE_KEY_CYCLES,
        "show_to_paint_ms": show_stats,
        "frame_ms": frame_stats,
        "preview_plane_elapsed_ms": plane_elapsed,
        "preview_plane_build_ms": plane_build,
        "preview_plane_calls": plane_calls,
        "preview_plane_errors": plane_errors,
        "preview_plane_reused_connections": reused_connections,
        "preview_plane_recoveries": plane_recoveries,
        "preview_plane_reuse_ratio": (reused_connections as f64 / plane_calls as f64),
        "fd_count": {"before": fds_before, "after": fds_after},
    }))
}

fn metric_stats(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({"count": 0});
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let percentile = |fraction: f64| sorted[((sorted.len() - 1) as f64 * fraction) as usize];
    let sum: u64 = sorted.iter().sum();
    json!({
        "count": sorted.len(),
        "min": sorted[0],
        "p50": percentile(0.50),
        "p95": percentile(0.95),
        "max": sorted[sorted.len() - 1],
        "mean": sum as f64 / sorted.len() as f64,
    })
}

fn trace_timestamp(trace: &str) -> Option<u128> {
    trace.split_whitespace().next()?.parse().ok()
}

fn trace_value(trace: &str, key: &str) -> Option<u64> {
    let start = trace.find(key)? + key.len();
    let digits: String = trace[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn performance_trace(metrics: &Value) -> String {
    format!(
        "PERF_METRIC show_to_paint_p50_ms={} show_to_paint_p95_ms={} plane_p50_ms={} plane_p95_ms={} plane_reuse_ratio={:.3} plane_recoveries={}",
        metrics["show_to_paint_ms"]["p50"].as_u64().unwrap_or(0),
        metrics["show_to_paint_ms"]["p95"].as_u64().unwrap_or(0),
        metrics["preview_plane_elapsed_ms"][FIXTURE_COUNT.to_string()]["p50"]
            .as_u64()
            .unwrap_or(0),
        metrics["preview_plane_elapsed_ms"][FIXTURE_COUNT.to_string()]["p95"]
            .as_u64()
            .unwrap_or(0),
        metrics["preview_plane_reuse_ratio"].as_f64().unwrap_or(0.0),
        metrics["preview_plane_recoveries"].as_u64().unwrap_or(0),
    )
}

fn wait_for_visible_picker_count(guest: &mut GuestControlClient, expected: usize) -> Result<()> {
    wait_for_command(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                "xdotool search --onlyvisible --name '^qol-alt-tab-picker@' 2>/dev/null | wc -l",
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim().parse() == Ok(expected),
        &format!("{expected} visible Alt Tab picker windows"),
    )?;
    Ok(())
}

fn wait_for_fixture_count(guest: &mut GuestControlClient, expected: usize) -> Result<()> {
    wait_for_command(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                "xdotool search --onlyvisible --name '^qol-alt-tab-storm-' 2>/dev/null | wc -l",
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim().parse() == Ok(expected),
        &format!("{expected} visible Alt Tab fixture windows"),
    )?;
    Ok(())
}

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    plugin_daemon_pid(guest, &["-x", "alt-tab"], "Alt Tab daemon")
}

#[cfg(test)]
mod tests {
    use super::{
        metric_stats, performance_metrics, performance_trace, trace_timestamp, trace_value,
    };

    #[test]
    fn metric_stats_reports_ordered_percentiles() {
        let stats = metric_stats(&[9, 1, 5, 3, 7]);
        assert_eq!(stats["count"], 5);
        assert_eq!(stats["min"], 1);
        assert_eq!(stats["p50"], 5);
        assert_eq!(stats["p95"], 7);
        assert_eq!(stats["max"], 9);
    }

    #[test]
    fn performance_metrics_extracts_show_and_plane_samples() {
        let traces = vec![
            "100 pid=1 SHOW_RECV show_id=1 reverse=false".to_string(),
            "104 pid=1 SHOW_PAINTED show_id=1 frame=3ms".to_string(),
            "105 pid=1 PREVIEW_PLANE_SHOW backend=cinnamon_shell show_id=visible outcome=ok items=8 elapsed=7ms reused_connection=true recovery_attempts=0 result=\"build_ms\":2".to_string(),
            "106 pid=1 PREVIEW_PLANE_HIDE backend=cinnamon_shell reason=dismiss outcome=ok elapsed=3ms reused_connection=true recovery_attempts=2".to_string(),
        ];
        let metrics = performance_metrics(&traces, 27, 27).unwrap();
        assert_eq!(metrics["show_to_paint_ms"]["p50"], 4);
        assert_eq!(metrics["preview_plane_elapsed_ms"]["8"]["p50"], 7);
        assert_eq!(metrics["preview_plane_recoveries"], 2);
        assert!(performance_trace(&metrics).contains("plane_recoveries=2"));
    }

    #[test]
    fn trace_value_and_timestamp_ignore_non_numeric_fields() {
        let trace = "123 pid=1 show_id=visible elapsed=7ms";
        assert_eq!(trace_timestamp(trace), Some(123));
        assert_eq!(trace_value(trace, "elapsed="), Some(7));
        assert_eq!(trace_value(trace, "show_id="), None);
    }
}
