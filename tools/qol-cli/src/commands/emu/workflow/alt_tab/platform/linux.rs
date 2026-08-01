//! Linux Mint implementation of the Alt Tab adversarial workflow.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, dispatch_plugin_action, fd_count,
    install_payload, plugin_daemon_pid, require_exec, require_plugin_action_guards, spawn,
    start_tray_and_wait_plugin_with_setup, wait_for_command, wait_for_probe_fields,
    wait_for_probe_line, wait_for_window_id, wait_for_window_title, within_fd_budget, xdotool_key,
    TraceCursor,
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

fn open_sticky(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = dispatch(guest, auth, "open")?;
    wait_for_probe_line(guest, cursor, "SHOW_PAINTED", "show_id=", ACTION_TIMEOUT)?;
    wait_for_focus_title(
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
