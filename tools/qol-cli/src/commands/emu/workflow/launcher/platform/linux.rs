//! Linux Mint implementation of the Launcher adversarial workflow.

use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, install_payload, require_exec,
    start_tray_and_wait_plugin, wait_for_command, wait_for_probe_fields, wait_for_probe_line,
    TraceCursor,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const PLUGIN_ID: &str = "plugin-launcher";
const FLOW_FILE: &str = "/home/qol/Documents/qol-launcher-flow-fresh.txt";
const CYCLE_COUNT: usize = 40;

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let auth = start_tray_and_wait_plugin(&mut guest, PLUGIN_ID)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;

    test_cold_open(&mut guest, &auth)?;
    test_repeated_activation(&mut guest, &auth)?;
    test_empty_result(&mut guest, &auth)?;
    let search = artifacts_dir.join("search.ppm");
    test_app_launch(&mut guest, &auth, &mut qmp, &search)?;
    let files = artifacts_dir.join("file-refresh.ppm");
    test_file_refresh(&mut guest, &auth, &mut qmp, &files)?;
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
                "CMD_RECV|LAUNCHER_(DISMISS|FILTER|INDEX|INPUT|RENDER|SHOW)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        "cold open, 40 retained cycles, input bounds, launches, live files, settings, guards, and crash recovery passed",
    );
    Ok(Verdict {
        pass: true,
        traces: probes.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![search, files, settings, final_state],
    })
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

fn key(guest: &mut GuestControlClient, value: &str) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["key", value]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn type_text(guest: &mut GuestControlClient, value: &str) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["type", "--delay", "0", value]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn wait_for_launcher(guest: &mut GuestControlClient) -> Result<()> {
    wait_for_window_title(
        guest,
        launcher_focus_args(),
        |title| title.starts_with("qol-launcher@"),
        "Launcher",
    )
}

fn wait_for_active_title(
    guest: &mut GuestControlClient,
    predicate: impl Fn(&str) -> bool,
    description: &str,
) -> Result<()> {
    wait_for_window_title(
        guest,
        &["getactivewindow", "getwindowname"],
        predicate,
        description,
    )
}

fn launcher_focus_args() -> &'static [&'static str] {
    &["getactivewindow", "getwindowname"]
}

fn wait_for_window_title(
    guest: &mut GuestControlClient,
    xdotool_args: &[&str],
    predicate: impl Fn(&str) -> bool,
    description: &str,
) -> Result<()> {
    wait_for_command(
        guest,
        command("/usr/bin/xdotool", xdotool_args),
        ACTION_TIMEOUT,
        |outcome| predicate(outcome.stdout.trim()),
        &format!("{description} to own guest focus"),
    )?;
    Ok(())
}

fn open_launcher(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = dispatch(guest, auth, "open")?;
    wait_for_probe_line(guest, cursor, "LAUNCHER_SHOW", "path=reuse", ACTION_TIMEOUT)?;
    wait_for_launcher(guest)
}

fn dismiss_launcher(guest: &mut GuestControlClient) -> Result<()> {
    let cursor = current_trace_cursor(guest)?;
    key(guest, "Escape")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "LAUNCHER_DISMISS",
        &["from=key"],
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn test_cold_open(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    open_launcher(guest, auth)?;
    dismiss_launcher(guest)?;
    step_label(
        "cold-open",
        StepKind::Success,
        "real tray action reached the retained GPUI window",
    );
    Ok(())
}

fn test_repeated_activation(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let pid_before = launcher_pid(guest)?;
    let fds_before = launcher_fd_count(guest, &pid_before)?;
    for _ in 0..CYCLE_COUNT {
        open_launcher(guest, auth)?;
        dismiss_launcher(guest)?;
    }
    let pid_after = launcher_pid(guest)?;
    if pid_before != pid_after {
        bail!("Launcher restarted during retained activation cycles");
    }
    let fds_after = launcher_fd_count(guest, &pid_after)?;
    if !within_fd_budget(fds_before, fds_after) {
        bail!("Launcher file descriptors grew from {fds_before} to {fds_after}");
    }
    step_label(
        "cycles",
        StepKind::Success,
        &format!("{CYCLE_COUNT} show/dismiss cycles retained pid={pid_after} fds={fds_after}"),
    );
    Ok(())
}

fn test_empty_result(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    open_launcher(guest, auth)?;
    let cursor = current_trace_cursor(guest)?;
    type_text(guest, "zzq_no_match_987654321")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "LAUNCHER_RENDER",
        &["results=0", "showing=true"],
        ACTION_TIMEOUT,
    )?;
    key(guest, "Return")?;
    wait_for_launcher(guest)?;
    dismiss_launcher(guest)?;
    Ok(())
}

fn test_app_launch(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    artifact: &std::path::Path,
) -> Result<()> {
    open_launcher(guest, auth)?;
    let cursor = current_trace_cursor(guest)?;
    type_text(guest, "fire")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "LAUNCHER_RENDER",
        &["mode=Apps", "selected_name=\"Firefox Web Browser\""],
        ACTION_TIMEOUT,
    )?;
    qmp.screendump(artifact)?;
    let launch_cursor = current_trace_cursor(guest)?;
    key(guest, "Return")?;
    wait_for_probe_fields(
        guest,
        launch_cursor,
        "LAUNCHER_DISMISS",
        &["from=launch", "selected_name=\"Firefox Web Browser\""],
        ACTION_TIMEOUT,
    )?;
    wait_for_active_title(guest, |title| title.contains("Mozilla Firefox"), "Firefox")
}

fn test_file_refresh(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    artifact: &std::path::Path,
) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/rm", &["--force", FLOW_FILE]),
        COMMAND_TIMEOUT,
    )?;
    let create_cursor = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command("/usr/bin/touch", &[FLOW_FILE]),
        COMMAND_TIMEOUT,
    )?;
    wait_for_probe_line(
        guest,
        create_cursor,
        "LAUNCHER_INDEX",
        "kind=files",
        ACTION_TIMEOUT,
    )?;

    open_launcher(guest, auth)?;
    key(guest, "Tab")?;
    let query_cursor = current_trace_cursor(guest)?;
    type_text(guest, "qol-launcher-flow-fresh")?;
    wait_for_probe_fields(
        guest,
        query_cursor,
        "LAUNCHER_RENDER",
        &[
            "mode=Files",
            "results=1",
            "selected_name=\"qol-launcher-flow-fresh.txt\"",
        ],
        ACTION_TIMEOUT,
    )?;
    qmp.screendump(artifact)?;

    let remove_cursor = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command("/usr/bin/unlink", &[FLOW_FILE]),
        COMMAND_TIMEOUT,
    )?;
    wait_for_probe_line(
        guest,
        remove_cursor,
        "LAUNCHER_INDEX",
        "kind=files",
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        remove_cursor,
        "LAUNCHER_RENDER",
        &["mode=Files", "results=0"],
        ACTION_TIMEOUT,
    )?;

    let recreate_cursor = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command("/usr/bin/touch", &[FLOW_FILE]),
        COMMAND_TIMEOUT,
    )?;
    wait_for_probe_line(
        guest,
        recreate_cursor,
        "LAUNCHER_INDEX",
        "kind=files",
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        recreate_cursor,
        "LAUNCHER_RENDER",
        &[
            "mode=Files",
            "results=1",
            "selected_name=\"qol-launcher-flow-fresh.txt\"",
        ],
        ACTION_TIMEOUT,
    )?;
    dismiss_launcher(guest)
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
        |title| title.starts_with("Launcher Settings"),
        "native Launcher Settings",
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
    let before = launcher_pid(guest)?;
    require_exec(
        guest,
        command("/usr/bin/kill", &["--signal", "KILL", &before]),
        COMMAND_TIMEOUT,
    )?;
    thread::sleep(Duration::from_millis(1_500));
    let cursor = dispatch(guest, auth, "open")?;
    let after = wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["-x", "launcher"]),
        ACTION_TIMEOUT,
        |outcome| {
            let pid = outcome.stdout.lines().next().map(str::trim);
            pid.is_some_and(|pid| pid != before)
        },
        "Launcher daemon restart",
    )?;
    let after = after
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .context("Launcher restart returned no PID")?;
    wait_for_probe_line(guest, cursor, "LAUNCHER_SHOW", "path=reuse", ACTION_TIMEOUT)?;
    wait_for_launcher(guest)?;
    dismiss_launcher(guest)?;
    step_label(
        "recovery",
        StepKind::Success,
        &format!("Launcher recovered pid={before}->{after}"),
    );
    Ok(())
}

fn launcher_pid(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/pgrep", &["-x", "launcher"]),
        COMMAND_TIMEOUT,
    )?;
    outcome
        .stdout
        .lines()
        .next()
        .map(str::to_string)
        .context("Launcher daemon was not running")
}

fn launcher_fd_count(guest: &mut GuestControlClient, pid: &str) -> Result<u64> {
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
    use super::{launcher_focus_args, within_fd_budget};

    #[test]
    fn fd_budget_allows_runtime_noise_but_rejects_cycle_growth() {
        let cases = [
            (30, 29, true),
            (30, 30, true),
            (30, 32, true),
            (30, 33, false),
            (u64::MAX, u64::MAX, true),
        ];
        for (before, after, expected) in cases {
            assert_eq!(
                within_fd_budget(before, after),
                expected,
                "before={before} after={after}"
            );
        }
    }

    #[test]
    fn launcher_oracle_reads_direct_x_input_focus() {
        assert_eq!(launcher_focus_args(), ["getwindowfocus", "getwindowname"]);
    }
}
