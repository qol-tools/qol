//! Linux Mint implementation of the Window Actions adversarial workflow.

use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, install_payload, require_exec, spawn,
    start_tray_and_wait_plugin, wait_for_command, wait_for_probe_fields, wait_for_probe_line,
    wait_for_window_id, window_geometry, TraceCursor, WindowGeometry,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(15);
const PLUGIN_ID: &str = "plugin-window-actions";

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let auth = start_tray_and_wait_plugin(&mut guest, PLUGIN_ID)?;
    wait_for_probe_line(
        &mut guest,
        TraceCursor::default(),
        "WINACT_DAEMON",
        "event=start",
        ACTION_TIMEOUT,
    )?;
    let fixtures = launch_fixtures(&mut guest)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;

    test_desktop_guard(&mut guest, &auth)?;
    let desktop_guard = artifacts_dir.join("desktop-guard.ppm");
    qmp.screendump(&desktop_guard)?;
    test_regular_actions(&mut guest, &auth, &fixtures[0])?;
    test_glide_actions(&mut guest, &auth, &fixtures[0])?;
    test_restore_stack(&mut guest, &auth, &fixtures)?;
    test_crash_recovery(&mut guest, &auth, &fixtures[0])?;

    let final_state = artifacts_dir.join("final.ppm");
    qmp.screendump(&final_state)?;
    let probes = require_exec(
        &mut guest,
        command(
            "/usr/bin/grep",
            &[
                "-E",
                "ACTION_DISPATCH|WINACT_(DAEMON|DONE|EVAL|GLIDE|RESTORE)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        "desktop guard, 32 actions, glides, restore stack, and crash recovery passed",
    );
    Ok(Verdict {
        pass: true,
        traces: probes.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![desktop_guard, final_state],
    })
}

fn launch_fixtures(guest: &mut GuestControlClient) -> Result<[String; 2]> {
    let mut ids = Vec::new();
    for (title, geometry) in [
        ("qol-window-storm-a", "80x24+180+160"),
        ("qol-window-storm-b", "80x24+360+260"),
    ] {
        spawn(
            guest,
            command("/usr/bin/xterm", &["-T", title, "-geometry", geometry]),
        )?;
        ids.push(wait_for_window_id(guest, title, FIXTURE_TIMEOUT)?);
    }
    ids.try_into()
        .map_err(|_| anyhow::anyhow!("window fixture count changed unexpectedly"))
}

fn activate(guest: &mut GuestControlClient, window_id: &str) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["windowactivate", "--sync", window_id]),
        COMMAND_TIMEOUT,
    )?;
    let expected = window_id
        .parse::<u64>()
        .context("fixture window id was not decimal")?;
    wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow"]),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim().parse() == Ok(expected),
        &format!("fixture window {window_id} to become active"),
    )?;
    Ok(())
}

fn dispatch(
    guest: &mut GuestControlClient,
    auth: &str,
    action: &str,
    body: &str,
) -> Result<TraceCursor> {
    let cursor = current_trace_cursor(guest)?;
    let url = format!(
        "{}/api/plugins/{PLUGIN_ID}/actions/{action}",
        local_base_url()
    );
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
                "POST",
                "--data",
                body,
                &url,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    Ok(cursor)
}

fn regular_action(
    guest: &mut GuestControlClient,
    auth: &str,
    action: &str,
    outcome: &str,
) -> Result<()> {
    let cursor = dispatch(guest, auth, action, "{}")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "WINACT_DONE",
        &[&format!("action={action}"), &format!("outcome={outcome}")],
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn glide_action(
    guest: &mut GuestControlClient,
    auth: &str,
    action: &str,
    phase: &str,
    outcome: &str,
) -> Result<()> {
    let body = format!(r#"{{"phase":"{phase}","source":"flow"}}"#);
    let cursor = dispatch(guest, auth, action, &body)?;
    wait_for_probe_fields(
        guest,
        cursor,
        "WINACT_GLIDE",
        &[
            &format!("phase={phase}"),
            &format!("direction={}", action.trim_start_matches("glide-")),
            &format!("outcome={outcome}"),
        ],
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn test_desktop_guard(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/wmctrl", &["-k", "on"]),
        COMMAND_TIMEOUT,
    )?;
    thread::sleep(Duration::from_millis(250));
    let desktop_id = wait_for_window_id(guest, "Desktop", FIXTURE_TIMEOUT)?;
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["windowfocus", "--sync", &desktop_id]),
        COMMAND_TIMEOUT,
    )?;
    let window_type = require_exec(
        guest,
        command(
            "/usr/bin/xprop",
            &["-id", &desktop_id, "_NET_WM_WINDOW_TYPE"],
        ),
        COMMAND_TIMEOUT,
    )?;
    if !window_type.stdout.contains("_NET_WM_WINDOW_TYPE_DESKTOP") {
        bail!(
            "desktop fixture has unexpected type: {}",
            window_type.stdout.trim()
        );
    }
    let before = window_geometry(guest, &desktop_id)?;
    let center = dispatch(guest, auth, "center", "{}")?;
    wait_for_probe_fields(
        guest,
        center,
        "WINACT_DONE",
        &[
            "action=center",
            "outcome=err",
            "Focused surface is not an app window",
        ],
        ACTION_TIMEOUT,
    )?;
    let glide = dispatch(
        guest,
        auth,
        "glide-right",
        r#"{"phase":"start","source":"flow"}"#,
    )?;
    wait_for_probe_fields(
        guest,
        glide,
        "WINACT_GLIDE",
        &[
            "phase=start",
            "direction=right",
            "outcome=err",
            "Focused surface is not an app window",
        ],
        ACTION_TIMEOUT,
    )?;
    let after = window_geometry(guest, &desktop_id)?;
    if before != after {
        bail!("desktop surface moved during guarded actions: {before:?} -> {after:?}");
    }
    require_exec(
        guest,
        command("/usr/bin/wmctrl", &["-k", "off"]),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "guard",
        StepKind::Success,
        "desktop surface rejected without geometry changes",
    );
    Ok(())
}

fn test_regular_actions(guest: &mut GuestControlClient, auth: &str, window_id: &str) -> Result<()> {
    activate(guest, window_id)?;
    let actions = [
        "center",
        "snap-left",
        "move-monitor-left",
        "move-monitor-right",
        "snap-right",
        "snap-bottom",
        "maximize",
        "snap-left",
    ];
    for _ in 0..4 {
        for action in actions {
            regular_action(guest, auth, action, "ok")?;
        }
    }
    let geometry = window_geometry(guest, window_id)?;
    if geometry.x > 16 || geometry.width < 400 {
        bail!("snap-left ended at unexpected geometry: {geometry:?}");
    }
    Ok(())
}

fn test_glide_actions(guest: &mut GuestControlClient, auth: &str, window_id: &str) -> Result<()> {
    for (action, axis, sign) in [
        ("glide-left", 'x', -1),
        ("glide-right", 'x', 1),
        ("glide-up", 'y', -1),
        ("glide-down", 'y', 1),
    ] {
        activate(guest, window_id)?;
        regular_action(guest, auth, "center", "ok")?;
        let before = window_geometry(guest, window_id)?;
        glide_action(guest, auth, action, "start", "ok")?;
        thread::sleep(Duration::from_millis(180));
        dispatch(
            guest,
            auth,
            action,
            r#"{"phase":"heartbeat","source":"flow"}"#,
        )?;
        glide_action(guest, auth, action, "stop", "ok")?;
        let after = window_geometry(guest, window_id)?;
        require_direction(action, before, after, axis, sign)?;
    }
    test_glide_reversal(guest, auth, window_id)
}

fn require_direction(
    action: &str,
    before: WindowGeometry,
    after: WindowGeometry,
    axis: char,
    sign: i32,
) -> Result<()> {
    let delta = match axis {
        'x' => after.x - before.x,
        'y' => after.y - before.y,
        _ => bail!("unsupported geometry axis"),
    };
    if delta.signum() != sign {
        bail!("{action} moved in the wrong direction: {before:?} -> {after:?}");
    }
    Ok(())
}

fn test_glide_reversal(guest: &mut GuestControlClient, auth: &str, window_id: &str) -> Result<()> {
    regular_action(guest, auth, "center", "ok")?;
    glide_action(guest, auth, "glide-left", "start", "ok")?;
    thread::sleep(Duration::from_millis(120));
    glide_action(guest, auth, "glide-right", "start", "ok")?;
    thread::sleep(Duration::from_millis(120));
    glide_action(guest, auth, "glide-right", "stop", "ok")?;
    thread::sleep(Duration::from_millis(80));
    glide_action(guest, auth, "glide-left", "stop", "ok")?;
    activate(guest, window_id)?;
    Ok(())
}

fn test_restore_stack(
    guest: &mut GuestControlClient,
    auth: &str,
    fixtures: &[String; 2],
) -> Result<()> {
    activate(guest, &fixtures[0])?;
    regular_action(guest, auth, "minimize", "ok")?;
    activate(guest, &fixtures[1])?;
    regular_action(guest, auth, "minimize", "ok")?;
    regular_action(guest, auth, "restore", "ok")?;
    require_active(guest, &fixtures[1])?;
    regular_action(guest, auth, "restore", "ok")?;
    require_active(guest, &fixtures[0])?;
    Ok(())
}

fn require_active(guest: &mut GuestControlClient, expected: &str) -> Result<()> {
    let expected_id = expected
        .parse::<u64>()
        .context("fixture window id was not decimal")?;
    let result = wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow"]),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim().parse() == Ok(expected_id),
        &format!("restored window {expected_id} to become active"),
    );
    if let Err(error) = result {
        let trace = require_exec(
            guest,
            command("/usr/bin/grep", &["WINACT_RESTORE", TRACE_LOG_PATH]),
            COMMAND_TIMEOUT,
        )?;
        bail!("{error:#}; restore trace: {}", trace.stdout.trim());
    }
    Ok(())
}

fn test_crash_recovery(guest: &mut GuestControlClient, auth: &str, window_id: &str) -> Result<()> {
    activate(guest, window_id)?;
    let before = daemon_pid(guest)?;
    glide_action(guest, auth, "glide-right", "start", "ok")?;
    require_exec(
        guest,
        command("/usr/bin/kill", &["-KILL", &before]),
        COMMAND_TIMEOUT,
    )?;
    thread::sleep(Duration::from_millis(1_350));
    require_grab_released(guest)?;
    regular_action(guest, auth, "center", "ok")?;
    let after = wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["-x", "window-actions"]),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim() != before,
        "window-actions daemon restart",
    )?;
    if after.stdout.trim() == before {
        bail!("window-actions daemon did not restart after SIGKILL");
    }
    Ok(())
}

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/pgrep", &["-x", "window-actions"]),
        COMMAND_TIMEOUT,
    )?;
    outcome
        .stdout
        .lines()
        .next()
        .map(str::to_string)
        .context("window-actions daemon was not running")
}

fn require_grab_released(guest: &mut GuestControlClient) -> Result<()> {
    let script = "const Meta = imports.gi.Meta; global.display.get_grab_op() === Meta.GrabOp.NONE";
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/gdbus",
            &[
                "call",
                "--session",
                "--dest",
                "org.Cinnamon",
                "--object-path",
                "/org/Cinnamon",
                "--method",
                "org.Cinnamon.Eval",
                script,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    if !outcome.stdout.contains("true") {
        bail!(
            "Cinnamon retained a native move grab: {}",
            outcome.stdout.trim()
        );
    }
    Ok(())
}
