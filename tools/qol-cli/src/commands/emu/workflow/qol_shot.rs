use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessState};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, exec, install_payload, require_exec,
    spawn, start_tray_and_wait_plugin, wait_for_command, wait_for_probe_fields,
    wait_for_probe_line, wait_for_window_id, TraceCursor,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const PLUGIN_ID: &str = "qol-shot";
const FIXTURE_TITLE: &str = "qol-shot-storm-fixture";
const SELECTOR_PREFIX: &str = "^qol-shot-selector-";
const PREVIEW_PREFIX: &str = "^qol-shot-preview";
const CANCEL_CYCLES: usize = 24;
const SCREENSHOT_BURST: usize = 16;
const COUNTDOWN_CYCLES: usize = 4;

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let auth = start_tray_and_wait_plugin(&mut guest, PLUGIN_ID)?;
    wait_for_probe_line(
        &mut guest,
        TraceCursor::default(),
        "SHOT_DAEMON_APP",
        "state=ready",
        ACTION_TIMEOUT,
    )?;
    launch_fixture(&mut guest)?;
    let (width, height) = desktop_resolution(&mut guest)?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    test_http_guards(&mut guest, &auth)?;
    let settings_first = artifacts_dir.join("settings-first.ppm");
    test_settings(&mut guest, &auth, &mut qmp, &settings_first)?;
    let (fds_before, fds_after) = test_selector_cancel_storm(&mut guest, &auth)?;
    test_screenshot_burst(&mut guest, &auth)?;
    let preview = artifacts_dir.join("preview-reentry.ppm");
    test_preview_reentry(&mut guest, &auth, &mut qmp, width, height, &preview)?;
    test_countdown_storm(&mut guest, &auth, &mut qmp, width, height)?;
    let recording = artifacts_dir.join("recording.ppm");
    test_recording(&mut guest, &auth, &mut qmp, width, height, &recording)?;
    let settings_reentry = artifacts_dir.join("settings-reentry.ppm");
    test_settings(&mut guest, &auth, &mut qmp, &settings_reentry)?;
    test_crash_recovery(&mut guest, &auth)?;

    let final_state = artifacts_dir.join("final.ppm");
    qmp.screendump(&final_state)?;
    require_no_capture_process(&mut guest)?;
    let probes = require_exec(
        &mut guest,
        command(
            "/usr/bin/grep",
            &[
                "-E",
                "ACTION_DISPATCH|CMD_RECV|SURFACE_ACTIVATION|SHOT_(CAPTURE_DROP_QUEUED|CAPTURE_LOCK|CAPTURE_STATUS|CMD|DAEMON_APP|FILE|PREVIEW_CLOSE|PREVIEW_REVEAL|RECORD|SELECT|SKIP)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    let mut traces = probes
        .stdout
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    traces.push(format!("selector_cancel_cycles={CANCEL_CYCLES}"));
    traces.push(format!("screenshot_burst={SCREENSHOT_BURST}"));
    traces.push(format!("countdown_cancel_cycles={COUNTDOWN_CYCLES}"));
    traces.push(format!("fd_count={fds_before}->{fds_after}"));
    step_label(
        "storm",
        StepKind::Success,
        "selector cancellation, burst coalescing, preview re-entry, recording, settings, guards, and crash recovery passed",
    );
    Ok(Verdict {
        pass: true,
        traces,
        artifacts: vec![
            settings_first,
            preview,
            recording,
            settings_reentry,
            final_state,
        ],
    })
}

fn launch_fixture(guest: &mut GuestControlClient) -> Result<()> {
    spawn(
        guest,
        command(
            "/usr/bin/xterm",
            &["-T", FIXTURE_TITLE, "-geometry", "80x24+100+100"],
        ),
    )?;
    wait_for_window_id(guest, FIXTURE_TITLE, Duration::from_secs(15))?;
    Ok(())
}

fn post_action(guest: &mut GuestControlClient, auth: Option<&str>, action: &str) -> Result<()> {
    let url = format!(
        "{}/api/plugins/{PLUGIN_ID}/actions/{action}",
        local_base_url()
    );
    let mut args = vec![
        "--fail",
        "--silent",
        "--show-error",
        "--header",
        "Content-Type: application/json",
        "--request",
        "POST",
        "--data",
        "{}",
    ];
    if let Some(auth) = auth {
        args.extend(["--header", auth]);
    }
    args.push(&url);
    require_exec(guest, command("/usr/bin/curl", &args), ACTION_TIMEOUT)?;
    Ok(())
}

fn dispatch(guest: &mut GuestControlClient, auth: &str, action: &str) -> Result<TraceCursor> {
    let cursor = current_trace_cursor(guest)?;
    post_action(guest, Some(auth), action)?;
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

fn desktop_resolution(guest: &mut GuestControlClient) -> Result<(u32, u32)> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/xrandr", &["--current"]),
        COMMAND_TIMEOUT,
    )?;
    parse_current_resolution(&outcome.stdout)
        .context("xrandr did not report a valid current resolution")
}

fn parse_current_resolution(output: &str) -> Option<(u32, u32)> {
    let line = output.lines().find(|line| line.starts_with("Screen 0:"))?;
    let current = line.split_once(" current ")?.1;
    let dimensions = current.split(',').next()?.trim();
    let (width, height) = dimensions.split_once(" x ")?;
    let dimensions = (width.parse().ok()?, height.parse().ok()?);
    (dimensions.0 > 0 && dimensions.1 > 0).then_some(dimensions)
}

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/pgrep", &["-x", "qol-shot"]),
        COMMAND_TIMEOUT,
    )?;
    outcome
        .stdout
        .lines()
        .next()
        .map(str::to_string)
        .context("qol-shot daemon was not running")
}

fn daemon_fd_count(guest: &mut GuestControlClient, pid: &str) -> Result<u64> {
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

fn visible_window_count(guest: &mut GuestControlClient, pattern: &str) -> Result<usize> {
    let outcome = exec(
        guest,
        command(
            "/usr/bin/xdotool",
            &["search", "--onlyvisible", "--name", pattern],
        ),
        Duration::from_secs(2),
    )?;
    if outcome.state != ProcessState::Exited {
        bail!("xdotool window search did not exit");
    }
    match outcome.exit_code {
        Some(0) => Ok(outcome.stdout.lines().count()),
        Some(1) => Ok(0),
        exit => bail!(
            "xdotool window search failed: exit={exit:?}, stderr={}",
            outcome.stderr.trim()
        ),
    }
}

fn require_visible_window_count(
    guest: &mut GuestControlClient,
    pattern: &str,
    expected: usize,
) -> Result<()> {
    let deadline = Instant::now() + ACTION_TIMEOUT;
    loop {
        let count = visible_window_count(guest, pattern)?;
        if count == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("visible window count for {pattern} was {count}, expected {expected}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn require_no_capture_process(guest: &mut GuestControlClient) -> Result<()> {
    let outcome = exec(
        guest,
        command("/usr/bin/pgrep", &["-x", "ffmpeg"]),
        Duration::from_secs(2),
    )?;
    if outcome.state == ProcessState::Exited && outcome.exit_code == Some(1) {
        return Ok(());
    }
    bail!(
        "ffmpeg capture process remained: state={:?} exit={:?} stdout={}",
        outcome.state,
        outcome.exit_code,
        outcome.stdout.trim()
    )
}

fn require_selector_clear(guest: &mut GuestControlClient, cursor: TraceCursor) -> Result<()> {
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_BARRIER",
        &["result=clear", "visible=0"],
        ACTION_TIMEOUT,
    )?;
    require_exec(
        guest,
        command(
            "/usr/bin/xdotool",
            &["mousemove", "300", "220", "click", "1"],
        ),
        COMMAND_TIMEOUT,
    )?;
    wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow", "getwindowname"]),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim() == FIXTURE_TITLE,
        "input to pass through the parked selector",
    )?;
    Ok(())
}

fn cancel_selector(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = dispatch(guest, auth, "screenshot")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    require_visible_window_count(guest, SELECTOR_PREFIX, 1)?;
    key(guest, "Escape")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_RESULT",
        &["source=daemon-app", "rect=none"],
        ACTION_TIMEOUT,
    )?;
    require_selector_clear(guest, cursor)
}

fn test_selector_cancel_storm(guest: &mut GuestControlClient, auth: &str) -> Result<(u64, u64)> {
    cancel_selector(guest, auth)?;
    let pid_before = daemon_pid(guest)?;
    let fds_before = daemon_fd_count(guest, &pid_before)?;
    for _ in 0..CANCEL_CYCLES {
        cancel_selector(guest, auth)?;
    }
    let pid_after = daemon_pid(guest)?;
    if pid_before != pid_after {
        bail!("qol-shot restarted during selector cancellation cycles");
    }
    let fds_after = daemon_fd_count(guest, &pid_after)?;
    if !within_fd_budget(fds_before, fds_after) {
        bail!("qol-shot file descriptors grew from {fds_before} to {fds_after}");
    }
    step_label(
        "selectors",
        StepKind::Success,
        &format!("{CANCEL_CYCLES} retained cancel cycles pid={pid_after} fds={fds_after}"),
    );
    Ok((fds_before, fds_after))
}

fn within_fd_budget(before: u64, after: u64) -> bool {
    after <= before.saturating_add(2)
}

fn test_screenshot_burst(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cursor = dispatch(guest, auth, "screenshot")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    for _ in 0..SCREENSHOT_BURST {
        post_action(guest, Some(auth), "screenshot")?;
    }
    require_visible_window_count(guest, SELECTOR_PREFIX, 1)?;
    key(guest, "Escape")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_CAPTURE_DROP_QUEUED",
        &[&format!("count={SCREENSHOT_BURST}")],
        ACTION_TIMEOUT,
    )?;
    require_selector_clear(guest, cursor)
}

fn test_preview_reentry(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    width: u32,
    height: u32,
    artifact: &Path,
) -> Result<()> {
    qmp.move_pointer_absolute(
        300_u32.min(width.saturating_sub(1)),
        220_u32.min(height.saturating_sub(1)),
        width,
        height,
    )?;
    let cursor = dispatch(guest, auth, "screenshot")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    qmp.click_left()?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_PREVIEW_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SCREENSHOT_READY",
        &["result=saved"],
        ACTION_TIMEOUT,
    )?;
    require_visible_window_count(guest, PREVIEW_PREFIX, 1)?;
    qmp.screendump(artifact)?;

    let preview_cursor = dispatch(guest, auth, "preview")?;
    wait_for_probe_fields(
        guest,
        preview_cursor,
        "SHOT_SKIP",
        &["action=preview", "reason=preview-showing"],
        ACTION_TIMEOUT,
    )?;
    require_visible_window_count(guest, PREVIEW_PREFIX, 1)?;

    let screenshot_cursor = dispatch(guest, auth, "screenshot")?;
    wait_for_probe_fields(
        guest,
        screenshot_cursor,
        "SHOT_PREVIEW_CLOSE",
        &["action=screenshot"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        screenshot_cursor,
        "SHOT_PREVIEW_BARRIER",
        &["action=screenshot", "result=clear", "visible=0"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        screenshot_cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    key(guest, "Escape")?;
    wait_for_probe_fields(
        guest,
        screenshot_cursor,
        "SHOT_SELECT_RESULT",
        &["rect=none"],
        ACTION_TIMEOUT,
    )?;
    require_selector_clear(guest, screenshot_cursor)
}

fn test_countdown_storm(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    width: u32,
    height: u32,
) -> Result<()> {
    for _ in 0..COUNTDOWN_CYCLES {
        qmp.move_pointer_absolute(
            300_u32.min(width.saturating_sub(1)),
            220_u32.min(height.saturating_sub(1)),
            width,
            height,
        )?;
        let cursor = dispatch(guest, auth, "record")?;
        wait_for_probe_fields(
            guest,
            cursor,
            "SHOT_SELECT_REVEAL",
            &["state=presented"],
            ACTION_TIMEOUT,
        )?;
        qmp.click_left()?;
        wait_for_probe_fields(
            guest,
            cursor,
            "SHOT_RECORD_COUNTDOWN",
            &["phase=shown", "seconds=3"],
            ACTION_TIMEOUT,
        )?;
        post_action(guest, Some(auth), "record")?;
        wait_for_probe_fields(
            guest,
            cursor,
            "SHOT_RECORD_TOGGLE",
            &["source=daemon", "result=countdown-cancelled"],
            ACTION_TIMEOUT,
        )?;
        wait_for_probe_fields(
            guest,
            cursor,
            "SHOT_CAPTURE_LOCK",
            &["action=daemon-flow", "result=released"],
            ACTION_TIMEOUT,
        )?;
        require_no_capture_process(guest)?;
    }
    Ok(())
}

fn set_recording_config(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let url = format!("{}/api/plugins/{PLUGIN_ID}/config", local_base_url());
    let body = r#"{"audio":{"enabled":false,"inputs":[],"mic_device":"default","system_device":"default"},"video":{"crf":30,"preset":"ultrafast","framerate":30,"format":"mkv"},"capture":{"include_window_frame":true,"pin_border":true,"open_folder_after_save":false,"saved_feedback":"toast"},"shortcuts":{"copy_command":"copy_path"}}"#;
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
    Ok(())
}

fn test_recording(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    width: u32,
    height: u32,
    artifact: &Path,
) -> Result<()> {
    set_recording_config(guest, auth)?;
    qmp.move_pointer_absolute(
        300_u32.min(width.saturating_sub(1)),
        220_u32.min(height.saturating_sub(1)),
        width,
        height,
    )?;
    let cursor = dispatch(guest, auth, "record")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    qmp.click_left()?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_RECORD_TOGGLE",
        &["source=daemon", "result=started"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_RECORD_STARTED",
        &["segments=0"],
        ACTION_TIMEOUT,
    )?;
    thread::sleep(Duration::from_millis(900));
    qmp.screendump(artifact)?;

    let stop_cursor = dispatch(guest, auth, "record")?;
    wait_for_probe_fields(
        guest,
        stop_cursor,
        "SHOT_RECORD_TOGGLE",
        &["source=daemon", "result=stopped"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        stop_cursor,
        "SHOT_CAPTURE_STATUS",
        &["context=recording", "stage=saved"],
        ACTION_TIMEOUT,
    )?;
    wait_for_command(
        guest,
        command(
            "/usr/bin/find",
            &[
                "/home/qol/Videos",
                "-maxdepth",
                "1",
                "-type",
                "f",
                "-name",
                "recording-*.mkv",
                "-size",
                "+0c",
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "a non-empty recording output",
    )?;
    require_no_capture_process(guest)
}

fn test_settings(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    artifact: &Path,
) -> Result<()> {
    let cursor = dispatch(guest, auth, "settings")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SURFACE_ACTIVATION",
        &["plugin=qol-shot", "phase=activate", "visible_windows=1"],
        ACTION_TIMEOUT,
    )?;
    require_visible_window_count(guest, "^QoL Shot Settings", 1)?;
    qmp.screendump(artifact)?;
    let active = require_exec(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow", "getwindowname"]),
        COMMAND_TIMEOUT,
    )?;
    if !active.stdout.trim().starts_with("QoL Shot Settings") {
        bail!(
            "native QoL Shot settings opened without focus; active window was {:?}",
            active.stdout.trim()
        );
    }
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
            format!(
                "{}/api/plugins/not-a-plugin/actions/screenshot",
                local_base_url()
            ),
            Some(auth),
            "404",
        ),
        (
            format!(
                "{}/api/plugins/{PLUGIN_ID}/actions/screenshot",
                local_base_url()
            ),
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
    let cursor = dispatch(guest, auth, "screenshot")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    let before = daemon_pid(guest)?;
    require_exec(
        guest,
        command("/usr/bin/kill", &["--signal", "KILL", &before]),
        COMMAND_TIMEOUT,
    )?;
    require_visible_window_count(guest, SELECTOR_PREFIX, 0)?;
    let outcome = wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["-x", "qol-shot"]),
        ACTION_TIMEOUT,
        |outcome| {
            outcome
                .stdout
                .lines()
                .next()
                .is_some_and(|pid| pid.trim() != before)
        },
        "qol-shot daemon restart",
    )?;
    let after = outcome
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .context("qol-shot daemon restart returned no PID")?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_DAEMON_APP",
        &["state=ready"],
        ACTION_TIMEOUT,
    )?;
    cancel_selector(guest, auth)?;
    step_label(
        "recovery",
        StepKind::Success,
        &format!("qol-shot recovered pid={before}->{after} without a stale selector"),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_current_resolution, within_fd_budget};

    #[test]
    fn current_resolution_parser_rejects_missing_zero_and_malformed_values() {
        let cases = [
            (
                "Screen 0: minimum 320 x 200, current 1280 x 800, maximum 16384 x 16384\n",
                Some((1280, 800)),
            ),
            ("Screen 0: current 0 x 800, maximum 10 x 10\n", None),
            ("Screen 0: current wide x 800, maximum 10 x 10\n", None),
            ("not xrandr\n", None),
        ];
        for (output, expected) in cases {
            assert_eq!(parse_current_resolution(output), expected, "{output}");
        }
    }

    #[test]
    fn fd_budget_allows_runtime_noise_without_hiding_growth() {
        let cases = [
            (30, 29, true),
            (30, 30, true),
            (30, 32, true),
            (30, 33, false),
            (u64::MAX, u64::MAX, true),
        ];
        for (before, after, expected) in cases {
            assert_eq!(within_fd_budget(before, after), expected);
        }
    }
}
