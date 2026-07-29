//! Linux Mint implementation of the Alt Tab adversarial workflow.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, install_payload, require_exec, spawn,
    start_tray_and_wait_plugin, wait_for_command, wait_for_probe_fields, wait_for_probe_line,
    wait_for_window_id, TraceCursor,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const PLUGIN_ID: &str = "plugin-alt-tab";
const PICKER_PREFIX: &str = "qol-alt-tab-picker@";
const FIXTURE_COUNT: usize = 8;
const RETAINED_CYCLES: usize = 50;
const KEY_CYCLES: usize = 240;

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let auth = start_tray_and_wait_plugin(&mut guest, PLUGIN_ID)?;
    launch_fixtures(&mut guest)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;

    test_hold_mode(&mut guest, &auth)?;
    set_sticky_config(&mut guest, &auth)?;
    let picker = artifacts_dir.join("picker.ppm");
    test_sticky_input_storm(&mut guest, &auth, &mut qmp, &picker)?;
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
                "ACTIVATE_WIN|CMD_RECV|DISMISS|FOCUS_REASSERT|KEY_RECV|NAV_GRID|SHOW_(CYCLE_FAST|LIST|PAINTED|RECV)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        "hold mode, 8-window churn, 240 keys, 50 retained cycles, settings, guards, and crash recovery passed",
    );
    Ok(Verdict {
        pass: true,
        traces: probes.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![picker, settings, final_state],
    })
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
                "{}",
                &url,
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    Ok(cursor)
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
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["key", "--clearmodifiers", value]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn open_sticky(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = dispatch(guest, auth, "open")?;
    wait_for_probe_line(guest, cursor, "SHOW_PAINTED", "show_id=", ACTION_TIMEOUT)?;
    wait_for_active_title(
        guest,
        |title| title.starts_with(PICKER_PREFIX),
        "Alt Tab picker",
    )
}

fn dismiss_sticky(guest: &mut GuestControlClient) -> Result<()> {
    let cursor = current_trace_cursor(guest)?;
    key(guest, "Escape")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "DISMISS",
        &["from=key/escape", "active=true"],
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn wait_for_active_title(
    guest: &mut GuestControlClient,
    predicate: impl Fn(&str) -> bool,
    description: &str,
) -> Result<()> {
    let result = wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow", "getwindowname"]),
        ACTION_TIMEOUT,
        |outcome| predicate(outcome.stdout.trim()),
        &format!("{description} to own guest focus"),
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

fn test_hold_mode(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = dispatch(guest, auth, "open")?;
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
        "default action painted, dismissed on modifier release, and activated a fixture",
    );
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
    wait_for_active_title(
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
    wait_for_active_title(
        guest,
        |title| title.starts_with("Alt Tab Settings"),
        "native Alt Tab Settings",
    )?;
    qmp.screendump(artifact)?;
    key(guest, "Escape")
}

fn test_http_guards(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cases = [
        (
            format!(
                "{}/api/plugins/{PLUGIN_ID}/actions/not-real",
                local_base_url()
            ),
            Some(auth),
            "400",
        ),
        (
            format!("{}/api/plugins/not-a-plugin/actions/open", local_base_url()),
            Some(auth),
            "404",
        ),
        (
            format!("{}/api/plugins/{PLUGIN_ID}/actions/open", local_base_url()),
            None,
            "401",
        ),
    ];
    for (url, auth, expected) in cases {
        let mut args = vec![
            "--silent",
            "--output",
            "/dev/null",
            "--write-out",
            "%{http_code}",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
            "--data",
            "{}",
        ];
        if let Some(auth) = auth {
            args.extend(["--header", auth]);
        }
        args.push(&url);
        let outcome = require_exec(guest, command("/usr/bin/curl", &args), COMMAND_TIMEOUT)?;
        if outcome.stdout.trim() != expected {
            bail!(
                "HTTP guard returned {}, expected {expected} for {url}",
                outcome.stdout.trim()
            );
        }
    }
    Ok(())
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

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/pgrep", &["-x", "alt-tab"]),
        COMMAND_TIMEOUT,
    )?;
    outcome
        .stdout
        .lines()
        .next()
        .map(str::to_string)
        .context("Alt Tab daemon was not running")
}

fn fd_count(guest: &mut GuestControlClient, pid: &str) -> Result<u64> {
    let path = format!("/proc/{pid}/fd");
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/find",
            &[&path, "-maxdepth", "1", "-type", "l", "-printf", ".\n"],
        ),
        COMMAND_TIMEOUT,
    )?;
    Ok(outcome.stdout.lines().count() as u64)
}

fn within_fd_budget(before: u64, after: u64) -> bool {
    after <= before.saturating_add(2)
}

#[cfg(test)]
mod tests {
    use super::within_fd_budget;

    #[test]
    fn fd_budget_allows_runtime_noise_without_hiding_growth() {
        for (before, after, expected) in [
            (28, 27, true),
            (28, 28, true),
            (28, 30, true),
            (28, 31, false),
            (u64::MAX, u64::MAX, true),
        ] {
            assert_eq!(within_fd_budget(before, after), expected);
        }
    }
}
