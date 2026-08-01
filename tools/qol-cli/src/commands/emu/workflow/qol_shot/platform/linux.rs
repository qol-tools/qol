//! Linux Mint implementation of the qol-shot adversarial workflow.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessState};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, desktop_resolution, exec, fd_count,
    install_payload, plugin_daemon_pid, require_exec, require_plugin_action_guards, spawn,
    start_tray_and_wait_plugin, wait_for_command, wait_for_probe_fields, wait_for_probe_line,
    wait_for_window_id, within_fd_budget, xdotool_key, TraceCursor,
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
const TOP_BAND_MAX_ADJACENT_YAVG_DELTA: f64 = 2.0;
const CURSOR_MOTION_STEPS: u32 = 60;
const MIN_CURSOR_MOTION_FRAMES: usize = 15;

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
    xdotool_key(guest, value, false)
}

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    plugin_daemon_pid(guest, &["-x", "qol-shot"], "qol-shot daemon")
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
    let fds_before = fd_count(guest, &pid_before)?;
    for _ in 0..CANCEL_CYCLES {
        cancel_selector(guest, auth)?;
    }
    let pid_after = daemon_pid(guest)?;
    if pid_before != pid_after {
        bail!("qol-shot restarted during selector cancellation cycles");
    }
    let fds_after = fd_count(guest, &pid_after)?;
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
    let body = r#"{"audio":{"enabled":false,"inputs":[],"mic_device":"default","system_device":"default"},"video":{"crf":30,"preset":"ultrafast","framerate":60,"format":"mkv"},"capture":{"include_window_frame":true,"pin_border":true,"open_folder_after_save":false,"saved_feedback":"toast"},"shortcuts":{"copy_command":"copy_path"}}"#;
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
        width.saturating_sub(100),
        height.saturating_sub(100),
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
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_RECORD_START_BACKEND",
        &["backend=cinnamon_after_paint", "outcome=ready"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_RECORD_CAPTURE_READY",
        &["backend=cinnamon_after_paint", "len="],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_RECORD_START_PLAN",
        &[
            "cursor=xfixes_actor_fallback_builtin",
            "cursor_update=before_paint",
        ],
        ACTION_TIMEOUT,
    )?;
    let fixture = wait_for_window_id(guest, FIXTURE_TITLE, ACTION_TIMEOUT)?;
    for (x, y) in [(200, 200), (500, 300)].into_iter().cycle().take(12) {
        require_exec(
            guest,
            command(
                "/usr/bin/xdotool",
                &[
                    "windowmove",
                    fixture.as_str(),
                    &x.to_string(),
                    &y.to_string(),
                ],
            ),
            COMMAND_TIMEOUT,
        )?;
    }
    require_exec(
        guest,
        command(
            "/usr/bin/xdotool",
            &["windowmove", fixture.as_str(), "100", "100"],
        ),
        COMMAND_TIMEOUT,
    )?;
    move_pointer_smoothly(guest)?;
    thread::sleep(Duration::from_millis(300));
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
    wait_for_probe_fields(
        guest,
        stop_cursor,
        "SHOT_RECORD_FINALIZE",
        &[
            "stage=converted",
            "backend=cinnamon_after_paint",
            "format=mkv",
        ],
        ACTION_TIMEOUT,
    )?;
    let cursor_stats = wait_for_probe_fields(
        guest,
        stop_cursor,
        "SHOT_RECORD_CURSOR_STATS",
        &["paints=", "changes="],
        ACTION_TIMEOUT,
    )?;
    let cursor_position_changes =
        probe_usize(&cursor_stats.stdout, "SHOT_RECORD_CURSOR_STATS", "changes=")?;
    let recording = wait_for_command(
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
    let recording = recording
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("recording output discovery returned no path")?;
    let stream = require_exec(
        guest,
        command(
            "/usr/bin/ffprobe",
            &[
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height",
                "-of",
                "default=nw=1",
                recording,
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    for expected in ["codec_name=h264", "width=1280", "height=800"] {
        if !stream.stdout.lines().any(|line| line.trim() == expected) {
            bail!(
                "recording stream did not contain {expected}: {}",
                stream.stdout.trim()
            );
        }
    }
    let conversion_log = require_exec(
        guest,
        command("/usr/bin/tail", &["-c", "32768", "/tmp/record-region.log"]),
        COMMAND_TIMEOUT,
    )?;
    let (duplicated, dropped) = ffmpeg_timing_adjustments(&conversion_log.stdout);
    if duplicated != 0 || dropped != 0 {
        bail!(
            "recording conversion rewrote capture cadence: duplicated={duplicated} dropped={dropped}"
        );
    }
    let band = require_exec(
        guest,
        command(
            "/usr/bin/ffmpeg",
            &[
                "-v",
                "error",
                "-i",
                recording,
                "-vf",
                "crop=iw:80:0:0,signalstats,metadata=print:file=-",
                "-f",
                "null",
                "-",
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    let (frames, max_delta) = top_band_yavg_stats(&band.stdout)?;
    if frames < 10 {
        bail!("recording exposed only {frames} top-band frame samples");
    }
    if max_delta > TOP_BAND_MAX_ADJACENT_YAVG_DELTA {
        bail!(
            "recording top band flickered: max adjacent YAVG delta {max_delta:.6} exceeded {TOP_BAND_MAX_ADJACENT_YAVG_DELTA:.6}"
        );
    }
    let cursor_motion = require_exec(
        guest,
        command(
            "/usr/bin/ffmpeg",
            &[
                "-hide_banner",
                "-v",
                "verbose",
                "-sseof",
                "-1.4",
                "-i",
                recording,
                "-vf",
                "crop=1000:120:100:640,tblend=all_mode=difference,bbox=min_val=24",
                "-an",
                "-f",
                "null",
                "-",
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    let cursor_motion_frames = cursor_motion_frame_count(&cursor_motion.stderr);
    if cursor_motion_frames < MIN_CURSOR_MOTION_FRAMES {
        bail!(
            "recorded cursor was choppy: only {cursor_motion_frames} changed frames during {CURSOR_MOTION_STEPS}-step motion ({cursor_position_changes} positions observed by Cinnamon); expected at least {MIN_CURSOR_MOTION_FRAMES}"
        );
    }
    step_label(
        "recording",
        StepKind::Success,
        &format!(
            "Cinnamon after-paint capture encoded {frames} frames with top-band max delta {max_delta:.6}, {cursor_motion_frames} cursor-motion frames, {cursor_position_changes} observed positions, and no conversion timing rewrites"
        ),
    );
    require_no_capture_process(guest)
}

fn top_band_yavg_stats(output: &str) -> Result<(usize, f64)> {
    let values = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("lavfi.signalstats.YAVG="))
        .map(|value| {
            value
                .parse::<f64>()
                .with_context(|| format!("invalid top-band YAVG value {value:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if values.is_empty() {
        bail!("ffmpeg returned no top-band YAVG samples");
    }
    let max_delta = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .fold(0.0_f64, f64::max);
    Ok((values.len(), max_delta))
}

fn cursor_motion_frame_count(output: &str) -> usize {
    output
        .lines()
        .filter(|line| line.contains("Parsed_bbox") && line.contains(" x1:"))
        .count()
}

fn ffmpeg_timing_adjustments(output: &str) -> (usize, usize) {
    output
        .split(['\r', '\n'])
        .filter_map(|line| {
            let duplicated = progress_usize(line, "dup=")?;
            let dropped = progress_usize(line, "drop=")?;
            Some((duplicated, dropped))
        })
        .next_back()
        .unwrap_or((0, 0))
}

fn progress_usize(line: &str, prefix: &str) -> Option<usize> {
    line.split_ascii_whitespace()
        .find_map(|field| field.strip_prefix(prefix))?
        .parse()
        .ok()
}

fn probe_usize(output: &str, tag: &str, prefix: &str) -> Result<usize> {
    output
        .lines()
        .find(|line| line.contains(&format!(" {tag} ")))
        .and_then(|line| {
            line.split_ascii_whitespace()
                .find_map(|field| field.strip_prefix(prefix))
        })
        .map(|value| value.trim_matches('"'))
        .context("probe did not contain the expected numeric field")?
        .parse()
        .with_context(|| format!("probe field {prefix} was not an unsigned integer"))
}

fn move_pointer_smoothly(guest: &mut GuestControlClient) -> Result<()> {
    let mut motion = command("/usr/bin/xdotool", &[]);
    for step in 0..CURSOR_MOTION_STEPS {
        let x = 150 + step * 900 / CURSOR_MOTION_STEPS.saturating_sub(1);
        motion.args.extend([
            "mousemove".to_string(),
            x.to_string(),
            "700".to_string(),
            "sleep".to_string(),
            "0.016".to_string(),
        ]);
    }
    require_exec(guest, motion, ACTION_TIMEOUT)?;
    Ok(())
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
    require_plugin_action_guards(guest, auth, PLUGIN_ID, "screenshot")
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
    use super::{
        cursor_motion_frame_count, ffmpeg_timing_adjustments, probe_usize, top_band_yavg_stats,
    };

    #[test]
    fn top_band_stats_report_largest_adjacent_frame_change() {
        let output = "frame:0\nlavfi.signalstats.YAVG=51.3265\nframe:1\nlavfi.signalstats.YAVG=51.3394\nframe:2\nlavfi.signalstats.YAVG=65.0\n";

        let (frames, max_delta) = top_band_yavg_stats(output).unwrap();

        assert_eq!(frames, 3);
        assert!((max_delta - 13.6606).abs() < 0.000_001);
    }

    #[test]
    fn top_band_stats_reject_missing_and_malformed_samples() {
        assert!(top_band_yavg_stats("frame:0").is_err());
        assert!(top_band_yavg_stats("lavfi.signalstats.YAVG=bright").is_err());
    }

    #[test]
    fn cursor_motion_count_ignores_unchanged_bbox_frames() {
        let output = "[Parsed_bbox_2] n:0 pts_time:0.0\n[Parsed_bbox_2] n:1 pts_time:0.03 x1:10 x2:20 y1:4 y2:16 w:11 h:13\nother x1:30\n";

        assert_eq!(cursor_motion_frame_count(output), 1);
    }

    #[test]
    fn probe_metric_reads_cursor_position_changes() {
        let output = "42 pid=9 SHOT_RECORD_CURSOR_STATS polls=91 changes=31\n";

        assert_eq!(
            probe_usize(output, "SHOT_RECORD_CURSOR_STATS", "changes=").unwrap(),
            31
        );
    }

    #[test]
    fn ffmpeg_timing_adjustments_use_the_final_progress_sample() {
        let output = "frame=70 dup=4 drop=8\rframe=631 dup=88 drop=44\n";

        assert_eq!(ffmpeg_timing_adjustments(output), (88, 44));
        assert_eq!(ffmpeg_timing_adjustments("frame=587 fps=60"), (0, 0));
    }
}
