//! Linux Mint implementation of the qol-shot adversarial workflow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, SETTINGS_SURFACE_DISPLAY_NAME, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessState, RequestAction, ResponseResult};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, desktop_resolution, dispatch, exec,
    fd_count, install_payload, key, launch_fixture, plugin_daemon_pid, post_action, probe_field,
    require_exec, require_plugin_action_guards, require_visible_window_count, spawn,
    start_tray_and_wait_plugin_with_env, wait_for_command, wait_for_probe_fields,
    wait_for_probe_line, wait_for_window_id, window_geometry, within_fd_budget, TraceCursor,
    WindowGeometry, ACTION_TIMEOUT, PLUGIN_ID, PREVIEW_PREFIX,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const FIXTURE_TITLE: &str = "qol-shot-storm-fixture";
const SELECTOR_PREFIX: &str = "^qol-shot-selector-";
const CANCEL_CYCLES: usize = 24;
const SCREENSHOT_BURST: usize = 16;
const COUNTDOWN_CYCLES: usize = 4;
const TOP_BAND_MAX_ADJACENT_YAVG_DELTA: f64 = 2.0;
const CURSOR_MOTION_STEPS: u32 = 60;
const MIN_CURSOR_MOTION_FRAMES: usize = 15;
const PIN_PREFIX: &str = "^qol-shot-pin-";
const CURSOR_ADJACENT_GAP: i64 = 20;
const CURSOR_ADJACENT_MARGIN: i64 = 12;
const PLACEMENT_INPUT_SLOP: i64 = 2;
const PLACEMENT_TOLERANCE: i64 = 2;
const SELECTION_EXCURSION_PX: u32 = 24;

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let scale = workflow_scale()?;
    let placement_only = workflow_placement_only()?;
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let mut tray_environment: BTreeMap<String, String> = BTreeMap::new();
    tray_environment.insert("GPUI_X11_SCALE_FACTOR".to_string(), scale.to_string());
    let auth = start_tray_and_wait_plugin_with_env(&mut guest, PLUGIN_ID, &tray_environment)?;
    wait_for_probe_line(
        &mut guest,
        TraceCursor::default(),
        "SHOT_DAEMON_APP",
        "state=ready",
        ACTION_TIMEOUT,
    )?;
    launch_fixture(&mut guest, FIXTURE_TITLE)?;
    let (width, height) = desktop_resolution(&mut guest)?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    let mut placement = test_cursor_placement(
        &mut guest,
        &auth,
        &mut qmp,
        scale,
        (width, height),
        &artifacts_dir,
    )?;
    let mut counters: Vec<String> = Vec::new();
    let mut storm_artifacts: Vec<PathBuf> = Vec::new();
    if placement.failures.is_empty() && !placement_only {
        let storm = run_storm_checks(
            &mut guest,
            &auth,
            &mut qmp,
            (width, height),
            &artifacts_dir,
            &mut counters,
            &mut storm_artifacts,
        );
        if let Err(error) = storm {
            placement
                .failures
                .push(format!("storm checks failed: {error:#}"));
        }
    }

    let mut artifacts = Vec::new();
    let final_state = artifacts_dir.join("final.ppm");
    if let Err(error) = qmp.screendump(&final_state) {
        placement
            .failures
            .push(format!("final screenshot failed: {error:#}"));
    } else {
        artifacts.push(final_state);
    }
    if let Err(error) = require_no_capture_process(&mut guest) {
        placement
            .failures
            .push(format!("capture cleanup verification failed: {error:#}"));
    }
    let mut traces = Vec::new();
    match require_exec(
        &mut guest,
        command(
            "/usr/bin/grep",
            &[
                "-E",
                "ACTION_DISPATCH|CMD_RECV|CURSOR_APPLY|SURFACE_ACTIVATION|SHOT_(CAPTURE_DROP_QUEUED|CAPTURE_LOCK|CAPTURE_STATUS|CMD|DAEMON_APP|FILE|PIN_PLACE|PIN_TRANSITION|PREVIEW_CLOSE|PREVIEW_LAYOUT|PREVIEW_PLACE|PREVIEW_REVEAL|RECORD|SELECT|SKIP)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    ) {
        Ok(probes) => traces.extend(probes.stdout.lines().map(str::to_string)),
        Err(error) => {
            placement
                .failures
                .push(format!("placement trace export failed: {error:#}"));
        }
    }
    traces.extend(counters);
    traces.push(format!("placement_scale={scale}"));
    traces.push(format!("placement_only={placement_only}"));
    traces.extend(placement.evidence);
    traces.extend(placement.failures.iter().cloned());
    for failure in &placement.failures {
        step_label("placement", StepKind::Info, failure);
    }
    if placement.failures.is_empty() && !placement_only {
        step_label(
            "storm",
            StepKind::Success,
            &format!(
                "selector cancellation, burst coalescing, preview re-entry, recording, settings, guards, crash recovery, and placement checks completed with {} recorded placement failures",
                placement.failures.len()
            ),
        );
    } else if placement.failures.is_empty() {
        step_label(
            "placement",
            StepKind::Success,
            "all placement scenarios and assertions passed",
        );
    } else {
        step_label(
            "storm",
            StepKind::Info,
            &format!(
                "storm checks skipped or incomplete after {} recorded failures",
                placement.failures.len()
            ),
        );
    }
    artifacts.extend(storm_artifacts);
    artifacts.extend(placement.artifacts);
    Ok(Verdict {
        pass: placement.failures.is_empty(),
        traces,
        artifacts,
    })
}

fn workflow_scale() -> Result<u32> {
    match std::env::var("QOL_SHOT_WORKFLOW_SCALE") {
        Ok(value) => match value.as_str() {
            "1" => Ok(1),
            "2" => Ok(2),
            other => bail!("QOL_SHOT_WORKFLOW_SCALE must be 1 or 2, got {other}"),
        },
        Err(_) => Ok(1),
    }
}

fn workflow_placement_only() -> Result<bool> {
    match std::env::var("QOL_SHOT_WORKFLOW_PLACEMENT_ONLY") {
        Ok(value) => match value.as_str() {
            "1" => Ok(true),
            "0" => Ok(false),
            other => bail!("QOL_SHOT_WORKFLOW_PLACEMENT_ONLY must be 0 or 1, got {other}"),
        },
        Err(_) => Ok(false),
    }
}

fn run_storm_checks(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    screen: (u32, u32),
    artifacts_dir: &Path,
    counters: &mut Vec<String>,
    produced: &mut Vec<PathBuf>,
) -> Result<()> {
    test_http_guards(guest, auth)?;
    let settings_first = artifacts_dir.join("settings-first.ppm");
    produced.push(settings_first.clone());
    let outcome = test_settings(guest, auth, qmp, &settings_first);
    produced.retain(|recorded| recorded.exists());
    outcome?;
    let (fds_before, fds_after) = test_selector_cancel_storm(guest, auth)?;
    counters.push(format!("selector_cancel_cycles={CANCEL_CYCLES}"));
    counters.push(format!("fd_count={fds_before}->{fds_after}"));
    test_screenshot_burst(guest, auth)?;
    counters.push(format!("screenshot_burst={SCREENSHOT_BURST}"));
    let preview = artifacts_dir.join("preview-reentry.ppm");
    produced.push(preview.clone());
    let outcome = test_preview_reentry(guest, auth, qmp, screen.0, screen.1, &preview);
    produced.retain(|recorded| recorded.exists());
    outcome?;
    test_countdown_storm(guest, auth, qmp, screen.0, screen.1)?;
    counters.push(format!("countdown_cancel_cycles={COUNTDOWN_CYCLES}"));
    let recording = artifacts_dir.join("recording.ppm");
    produced.push(recording.clone());
    let outcome = test_recording(guest, auth, qmp, screen.0, screen.1, &recording);
    produced.retain(|recorded| recorded.exists());
    outcome?;
    let settings_reentry = artifacts_dir.join("settings-reentry.ppm");
    produced.push(settings_reentry.clone());
    let outcome = test_settings(guest, auth, qmp, &settings_reentry);
    produced.retain(|recorded| recorded.exists());
    outcome?;
    test_crash_recovery(guest, auth)?;
    Ok(())
}

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    plugin_daemon_pid(guest, &["-x", "qol-shot"], "qol-shot daemon")
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
    require_visible_window_count(guest, &format!("^{SETTINGS_SURFACE_DISPLAY_NAME}"), 1)?;
    qmp.screendump(artifact)?;
    let active = require_exec(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow", "getwindowname"]),
        COMMAND_TIMEOUT,
    )?;
    if !active
        .stdout
        .trim()
        .starts_with(SETTINGS_SURFACE_DISPLAY_NAME)
    {
        bail!(
            "{} settings surface did not take guest focus; active window was {:?}",
            SETTINGS_SURFACE_DISPLAY_NAME,
            active.stdout.trim()
        );
    }
    key(guest, "Escape")?;
    require_visible_window_count(guest, &format!("^{SETTINGS_SURFACE_DISPLAY_NAME}"), 0)
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

#[derive(Default)]
struct PlacementReport {
    failures: Vec<String>,
    evidence: Vec<String>,
    artifacts: Vec<PathBuf>,
}

struct PlacementCase {
    label: &'static str,
    pointer: (u32, u32),
    requires_reuse: bool,
    closes_visible_preview: bool,
    requires_backward: bool,
}

fn test_cursor_placement(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    scale: u32,
    screen: (u32, u32),
    artifacts_dir: &Path,
) -> Result<PlacementReport> {
    let mut report = PlacementReport::default();
    let cases = [
        PlacementCase {
            label: "offcenter",
            pointer: (screen.0 / 8, screen.1 / 6),
            requires_reuse: false,
            closes_visible_preview: false,
            requires_backward: false,
        },
        PlacementCase {
            label: "reuse",
            pointer: (screen.0 / 2, screen.1 / 4),
            requires_reuse: true,
            closes_visible_preview: true,
            requires_backward: false,
        },
        PlacementCase {
            label: "edge",
            pointer: (screen.0.saturating_sub(64), screen.1.saturating_sub(96)),
            requires_reuse: true,
            closes_visible_preview: false,
            requires_backward: true,
        },
    ];
    let mut preview_ids: Vec<String> = Vec::new();
    for case in &cases[..2] {
        let artifact = artifacts_dir.join(format!("placement-{}.ppm", case.label));
        match capture_preview_cycle(guest, auth, qmp, screen, case, &artifact, &mut report) {
            Ok(preview_id) => preview_ids.push(preview_id),
            Err(error) => {
                report.failures.push(format!(
                    "placement scenario {} failed: {error:#}",
                    case.label
                ));
                break;
            }
        }
    }
    if let Some(preview_id) = preview_ids.get(1) {
        let failures_before_pin = report.failures.len();
        let first_pointer = (screen.0 / 4, screen.1 / 2);
        let second_pointer = (screen.0 / 4, screen.1 / 4);
        let pins = pin_preview_at_pointer(
            guest,
            qmp,
            screen,
            first_pointer,
            preview_id,
            &artifacts_dir.join("placement-pin1.ppm"),
            &mut report,
        )
        .and_then(|(first_pin_id, first_expected)| {
            let second_case = PlacementCase {
                label: "secondpin",
                pointer: second_pointer,
                requires_reuse: true,
                closes_visible_preview: false,
                requires_backward: false,
            };
            capture_preview_cycle(
                guest,
                auth,
                qmp,
                screen,
                &second_case,
                &artifacts_dir.join("placement-secondpin.ppm"),
                &mut report,
            )
            .and_then(|second_preview_id| {
                pin_preview_at_pointer(
                    guest,
                    qmp,
                    screen,
                    second_pointer,
                    &second_preview_id,
                    &artifacts_dir.join("placement-pin2.ppm"),
                    &mut report,
                )
                .map(|(second_pin_id, second_expected)| {
                    (first_pin_id, first_expected, second_pin_id, second_expected)
                })
            })
        });
        match pins {
            Ok((first_pin_id, first_expected, second_pin_id, second_expected)) => {
                if first_pin_id == second_pin_id {
                    report.failures.push(format!(
                        "retained pins shared one window id: {first_pin_id}"
                    ));
                }
                if let Err(error) = verify_retained_pin_positions(
                    guest,
                    &first_pin_id,
                    first_expected,
                    &second_pin_id,
                    second_expected,
                ) {
                    report
                        .failures
                        .push(format!("retained pin positions failed: {error:#}"));
                }
                report.evidence.push(format!(
                    "placement_pin_pair first={first_pin_id} second={second_pin_id}"
                ));
                if let Err(error) = close_pinned_windows(guest, &[first_pin_id, second_pin_id]) {
                    report
                        .failures
                        .push(format!("pinned window cleanup failed: {error:#}"));
                }
                if report.failures.len() == failures_before_pin {
                    let artifact = artifacts_dir.join("placement-edge.ppm");
                    if let Err(error) = capture_preview_cycle(
                        guest,
                        auth,
                        qmp,
                        screen,
                        &cases[2],
                        &artifact,
                        &mut report,
                    ) {
                        report.failures.push(format!(
                            "placement scenario {} failed: {error:#}",
                            cases[2].label
                        ));
                    }
                }
            }
            Err(error) => {
                report
                    .failures
                    .push(format!("placement scenario pin failed: {error:#}"));
            }
        }
    }
    if let Err(error) =
        test_cold_standalone_preview(guest, qmp, scale, screen, artifacts_dir, &mut report)
    {
        report
            .failures
            .push(format!("placement scenario cold preview failed: {error:#}"));
    }
    Ok(report)
}

fn capture_preview_cycle(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    screen: (u32, u32),
    case: &PlacementCase,
    artifact: &Path,
    report: &mut PlacementReport,
) -> Result<String> {
    qmp.move_pointer_absolute(case.pointer.0, case.pointer.1, screen.0, screen.1)?;
    let cursor = dispatch(guest, auth, "screenshot")?;
    if case.closes_visible_preview {
        wait_for_probe_fields(
            guest,
            cursor,
            "SHOT_PREVIEW_CLOSE",
            &["action=screenshot"],
            ACTION_TIMEOUT,
        )?;
        wait_for_probe_fields(
            guest,
            cursor,
            "SHOT_PREVIEW_BARRIER",
            &["action=screenshot", "result=clear", "visible=0"],
            ACTION_TIMEOUT,
        )?;
    }
    let reveal = wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_SELECT_VIEWPORT",
        &["aligned=true", "shown=true", "focused=true"],
        ACTION_TIMEOUT,
    )?;
    let selector_title = probe_field(
        last_probe_line(&reveal.stdout, "SHOT_SELECT_REVEAL")
            .context("SHOT_SELECT_REVEAL probe did not contain a parseable line")?,
        "title",
    )
    .context("SHOT_SELECT_REVEAL probe did not identify the selector window")?
    .to_string();
    let selector_id = wait_for_window_id(guest, &selector_title, ACTION_TIMEOUT)?;
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["windowfocus", "--sync", &selector_id]),
        COMMAND_TIMEOUT,
    )?;
    let input_focus = require_exec(
        guest,
        command("/usr/bin/xdotool", &["getwindowfocus"]),
        COMMAND_TIMEOUT,
    )?
    .stdout
    .trim()
    .to_string();
    if input_focus != selector_id {
        report.failures.push(format!(
            "selector did not hold native input focus: title={selector_title} window={selector_id} observed={input_focus}"
        ));
    }
    report.evidence.push(format!(
        "placement_{} selector title={selector_title} window={selector_id} input_focus={input_focus}",
        case.label
    ));
    qmp.move_pointer_absolute(case.pointer.0, case.pointer.1, screen.0, screen.1)?;
    let excursion = (
        case.pointer
            .0
            .saturating_add(SELECTION_EXCURSION_PX)
            .min(screen.0 - 1),
        case.pointer
            .1
            .saturating_add(SELECTION_EXCURSION_PX)
            .min(screen.1 - 1),
    );
    qmp.set_left_button(true)?;
    qmp.move_pointer_absolute(excursion.0, excursion.1, screen.0, screen.1)?;
    qmp.move_pointer_absolute(case.pointer.0, case.pointer.1, screen.0, screen.1)?;
    qmp.set_left_button(false)?;
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
    let place = wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_PREVIEW_PLACE",
        &["cursor="],
        ACTION_TIMEOUT,
    )?;
    let place_line = last_probe_line(&place.stdout, "SHOT_PREVIEW_PLACE")
        .context("SHOT_PREVIEW_PLACE probe did not contain a parseable line")?
        .to_string();
    let sampled = probe_point(&place_line, "cursor").ok();
    let requested = probe_point(&place_line, "origin")
        .context("SHOT_PREVIEW_PLACE probe did not contain integer origin fields")?;
    let size = probe_size(&place_line, "size")
        .context("SHOT_PREVIEW_PLACE probe did not contain integer size fields")?;
    let layout = wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_PREVIEW_LAYOUT",
        &["title=", "path="],
        ACTION_TIMEOUT,
    )?;
    let layout_line = last_probe_line(&layout.stdout, "SHOT_PREVIEW_LAYOUT")
        .context("SHOT_PREVIEW_LAYOUT probe did not contain a parseable line")?
        .to_string();
    let layout_path = probe_field(&layout_line, "path")
        .context("SHOT_PREVIEW_LAYOUT probe did not claim a placement path")?
        .to_string();
    let layout_applied = probe_field(&layout_line, "applied").unwrap_or("absent");
    let cursor_apply = last_probe_line(&place.stdout, "CURSOR_APPLY").unwrap_or("absent");
    let preview_title = probe_field(&layout_line, "title")
        .context("SHOT_PREVIEW_LAYOUT probe did not identify the preview window")?
        .to_string();
    let preview_id = wait_for_window_id(guest, &preview_title, ACTION_TIMEOUT)?;
    let observed = wait_for_geometry_near(guest, &preview_id, requested, PLACEMENT_TOLERANCE)?;
    qmp.screendump(artifact)?;
    report.artifacts.push(artifact.to_path_buf());
    report.evidence.push(format!(
        "placement_{} pointer={},{} requested={},{} observed={},{} size={}x{} layout_path={layout_path} layout_applied={layout_applied} place=\"{place_line}\" layout=\"{layout_line}\" cursor_apply=\"{cursor_apply}\"",
        case.label,
        case.pointer.0,
        case.pointer.1,
        requested.0,
        requested.1,
        observed.x,
        observed.y,
        observed.width,
        observed.height
    ));
    if layout_path != "create" && layout_path != "reuse" {
        report.failures.push(format!(
            "preview placement claimed an unexpected path: expected=create|reuse claimed={layout_path}"
        ));
    }
    if case.requires_reuse && layout_path == "create" {
        report.failures.push(format!(
            "preview reuse cycle did not reuse the parked preview: claimed={layout_path} cursor_apply=\"{cursor_apply}\""
        ));
    }
    if layout_applied == "false" {
        report.failures.push(format!(
            "preview reuse claimed an unapplied layout sync: title={preview_title} layout=\"{layout_line}\""
        ));
    }
    match sampled {
        Some(sampled) => {
            if (sampled.0 - i64::from(case.pointer.0)).abs() > PLACEMENT_INPUT_SLOP
                || (sampled.1 - i64::from(case.pointer.1)).abs() > PLACEMENT_INPUT_SLOP
            {
                report.failures.push(format!(
                    "preview placement ignored the guest pointer: pointer={},{} sampled={},{}",
                    case.pointer.0, case.pointer.1, sampled.0, sampled.1
                ));
            }
            let expected = cursor_adjacent_origin(
                sampled,
                (i64::from(size.0), i64::from(size.1)),
                i64::from(screen.0),
                i64::from(screen.1),
            );
            let backward_x = expected.0 < sampled.0;
            let backward_y = expected.1 < sampled.1;
            report
                .evidence
                .push(format!(
                    "placement_{}_flips sampled={},{} expected={},{} backward_x={backward_x} backward_y={backward_y}",
                    case.label, sampled.0, sampled.1, expected.0, expected.1
                ));
            if case.requires_backward && (!backward_x || !backward_y) {
                report.failures.push(format!(
                    "edge placement did not flip backward on both axes: sampled={},{} expected={},{}",
                    sampled.0, sampled.1, expected.0, expected.1
                ));
            }
            if (expected.0 - requested.0).abs() > PLACEMENT_INPUT_SLOP
                || (expected.1 - requested.1).abs() > PLACEMENT_INPUT_SLOP
            {
                report.failures.push(format!(
                    "preview requested origin violated cursor-adjacent placement: sampled={},{} size={}x{} requested={},{} expected={},{}",
                    sampled.0, sampled.1, size.0, size.1, requested.0, requested.1, expected.0,
                    expected.1
                ));
            }
        }
        None => report.failures.push(format!(
            "preview placement sampled no cursor: pointer={},{} place=\"{place_line}\"",
            case.pointer.0, case.pointer.1
        )),
    }
    if (i64::from(observed.x) - requested.0).abs() > PLACEMENT_TOLERANCE
        || (i64::from(observed.y) - requested.1).abs() > PLACEMENT_TOLERANCE
    {
        report.failures.push(format!(
            "preview window did not land on its requested origin: requested={},{} observed={},{} size={}x{}",
            requested.0, requested.1, observed.x, observed.y, observed.width, observed.height
        ));
    }
    if observed.x < 0
        || observed.y < 0
        || i64::from(observed.x) + i64::from(observed.width) > i64::from(screen.0)
        || i64::from(observed.y) + i64::from(observed.height) > i64::from(screen.1)
    {
        report.failures.push(format!(
            "preview window escaped the guest screen: observed={},{} size={}x{} screen={}x{}",
            observed.x, observed.y, observed.width, observed.height, screen.0, screen.1
        ));
    }
    Ok(preview_id)
}

fn pin_preview_at_pointer(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    screen: (u32, u32),
    pointer: (u32, u32),
    preview_id: &str,
    artifact: &Path,
    report: &mut PlacementReport,
) -> Result<(String, (i64, i64))> {
    qmp.move_pointer_absolute(pointer.0, pointer.1, screen.0, screen.1)?;
    let cursor = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["windowfocus", "--sync", preview_id]),
        COMMAND_TIMEOUT,
    )?;
    qmp.send_keys(&["i".to_string()])?;
    let pin_place = wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_PIN_PLACE",
        &["origin="],
        ACTION_TIMEOUT,
    )?;
    let pin_place_line = last_probe_line(&pin_place.stdout, "SHOT_PIN_PLACE")
        .context("SHOT_PIN_PLACE probe did not contain a parseable line")?;
    let pin_requested = probe_point(pin_place_line, "origin")
        .context("SHOT_PIN_PLACE probe did not contain integer origin fields")?;
    let pin_reveal = wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_PIN_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    let pin_reveal_line = last_probe_line(&pin_reveal.stdout, "SHOT_PIN_REVEAL")
        .context("SHOT_PIN_REVEAL probe did not contain a parseable line")?;
    let pin_title = probe_field(pin_reveal_line, "title")
        .context("SHOT_PIN_REVEAL probe did not identify the pinned window")?
        .to_string();
    let pin_id = wait_for_window_id(guest, &pin_title, ACTION_TIMEOUT)?;
    let (observed, pin_expected) = wait_for_adjacent_geometry_near(
        guest,
        &pin_id,
        (i64::from(pointer.0), i64::from(pointer.1)),
        screen,
        PLACEMENT_TOLERANCE,
    )?;
    qmp.screendump(artifact)?;
    report.artifacts.push(artifact.to_path_buf());
    report.evidence.push(format!(
        "placement_pin pointer={},{} pin_requested={},{} expected={},{} observed={},{} size={}x{} pin_place=\"{pin_place_line}\"",
        pointer.0, pointer.1, pin_requested.0, pin_requested.1, pin_expected.0, pin_expected.1,
        observed.x, observed.y, observed.width, observed.height
    ));
    if (pin_requested.0 - pin_expected.0).abs() > PLACEMENT_TOLERANCE
        || (pin_requested.1 - pin_expected.1).abs() > PLACEMENT_TOLERANCE
    {
        report.failures.push(format!(
            "pinned window requested origin ignored the current cursor: pointer={},{} requested={},{} expected={},{}",
            pointer.0, pointer.1, pin_requested.0, pin_requested.1, pin_expected.0, pin_expected.1
        ));
    }
    if (i64::from(observed.x) - pin_expected.0).abs() > PLACEMENT_TOLERANCE
        || (i64::from(observed.y) - pin_expected.1).abs() > PLACEMENT_TOLERANCE
    {
        report.failures.push(format!(
            "pinned window did not land on its cursor-adjacent expected origin: expected={},{} observed={},{} size={}x{}",
            pin_expected.0, pin_expected.1, observed.x, observed.y, observed.width, observed.height
        ));
    }
    Ok((pin_id, pin_expected))
}

fn verify_retained_pin_positions(
    guest: &mut GuestControlClient,
    first_pin_id: &str,
    first_expected: (i64, i64),
    second_pin_id: &str,
    second_expected: (i64, i64),
) -> Result<()> {
    let first = window_geometry(guest, first_pin_id)?;
    let second = window_geometry(guest, second_pin_id)?;
    if (i64::from(first.x) - first_expected.0).abs() > PLACEMENT_TOLERANCE
        || (i64::from(first.y) - first_expected.1).abs() > PLACEMENT_TOLERANCE
    {
        bail!(
            "first retained pin moved: expected={},{} observed={},{}",
            first_expected.0,
            first_expected.1,
            first.x,
            first.y
        );
    }
    if (i64::from(second.x) - second_expected.0).abs() > PLACEMENT_TOLERANCE
        || (i64::from(second.y) - second_expected.1).abs() > PLACEMENT_TOLERANCE
    {
        bail!(
            "second retained pin moved: expected={},{} observed={},{}",
            second_expected.0,
            second_expected.1,
            second.x,
            second.y
        );
    }
    Ok(())
}

fn close_pinned_windows(guest: &mut GuestControlClient, pin_ids: &[String]) -> Result<()> {
    for pin_id in pin_ids {
        require_exec(
            guest,
            command("/usr/bin/xdotool", &["windowfocus", "--sync", pin_id]),
            COMMAND_TIMEOUT,
        )?;
        key(guest, "Escape")?;
    }
    require_visible_window_count(guest, PIN_PREFIX, 0)
}

fn test_cold_standalone_preview(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    scale: u32,
    screen: (u32, u32),
    artifacts_dir: &Path,
    report: &mut PlacementReport,
) -> Result<()> {
    let pointer = (screen.0 / 8, screen.1 / 6);
    qmp.move_pointer_absolute(pointer.0, pointer.1, screen.0, screen.1)?;
    let daemon_pid = plugin_daemon_pid(guest, &["-x", "qol-shot"], "qol-shot daemon")?;
    let binary = require_exec(
        guest,
        command(
            "/usr/bin/readlink",
            &["-f", &format!("/proc/{daemon_pid}/exe")],
        ),
        COMMAND_TIMEOUT,
    )?
    .stdout
    .trim()
    .to_string();
    let screenshot = wait_for_command(
        guest,
        command(
            "/usr/bin/find",
            &[
                "/home/qol/Pictures",
                "-maxdepth",
                "1",
                "-type",
                "f",
                "-name",
                "screenshot-*.png",
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "a saved screenshot for the standalone preview",
    )?;
    let path = screenshot
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("saved screenshot discovery returned no path")?
        .to_string();
    let cursor = current_trace_cursor(guest)?;
    let mut standalone = command(&binary, &["preview", path.as_str()]);
    standalone
        .env
        .insert("GPUI_X11_SCALE_FACTOR".to_string(), scale.to_string());
    let process_id = spawn(guest, standalone)?;
    let place = wait_for_probe_fields(
        guest,
        cursor,
        "SHOT_PREVIEW_PLACE",
        &["monitor_origin=", "native_scale="],
        ACTION_TIMEOUT,
    )?;
    let place_line = last_probe_line(&place.stdout, "SHOT_PREVIEW_PLACE")
        .context("SHOT_PREVIEW_PLACE probe did not contain a parseable line")?
        .to_string();
    let sampled = probe_point(&place_line, "cursor").ok();
    let requested = probe_point(&place_line, "origin")
        .context("SHOT_PREVIEW_PLACE probe did not contain integer origin fields")?;
    let native_scale = probe_field(&place_line, "native_scale")
        .context("SHOT_PREVIEW_PLACE probe did not contain native_scale")?;
    let emitted_scale = native_scale
        .parse::<f64>()
        .with_context(|| format!("probe native_scale {native_scale} was not a number"))?;
    if (emitted_scale - f64::from(scale)).abs() > 0.001 {
        report.failures.push(format!(
            "cold standalone preview emitted unexpected native scale: selected={scale} emitted={emitted_scale}"
        ));
    }
    let standalone_pid = wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["-x", "qol-shot"]),
        ACTION_TIMEOUT,
        |outcome| {
            outcome
                .stdout
                .lines()
                .any(|pid| pid.trim() != daemon_pid && !pid.trim().is_empty())
        },
        "the standalone preview process",
    )?;
    let standalone_pid = standalone_pid
        .stdout
        .lines()
        .map(str::trim)
        .find(|pid| *pid != daemon_pid && !pid.is_empty())
        .context("standalone preview process was not found")?
        .to_string();
    let window_id = wait_for_window_id(
        guest,
        &format!("qol-shot-preview-{standalone_pid}-.*"),
        ACTION_TIMEOUT,
    )?;
    let (observed, expected) = match sampled {
        Some(sampled) => wait_for_adjacent_geometry_near(
            guest,
            &window_id,
            sampled,
            screen,
            PLACEMENT_TOLERANCE,
        )?,
        None => {
            report.failures.push(format!(
                "cold standalone preview sampled no cursor: pointer={},{} place=\"{place_line}\"",
                pointer.0, pointer.1
            ));
            return Ok(());
        }
    };
    let artifact = artifacts_dir.join("placement-cold.ppm");
    qmp.screendump(&artifact)?;
    report.artifacts.push(artifact);
    report.evidence.push(format!(
        "placement_cold path=standalone-cli process={standalone_pid} window={window_id} native_scale={emitted_scale} requested={},{} expected={},{} observed={},{} size={}x{} place=\"{place_line}\"",
        requested.0,
        requested.1,
        expected.0,
        expected.1,
        observed.x,
        observed.y,
        observed.width,
        observed.height
    ));
    if (requested.0 - expected.0).abs() > PLACEMENT_TOLERANCE
        || (requested.1 - expected.1).abs() > PLACEMENT_TOLERANCE
    {
        report.failures.push(format!(
            "cold standalone requested origin violated cursor-adjacent placement: sampled={},{} size={}x{} requested={},{} expected={},{}",
            sampled.unwrap_or_default().0,
            sampled.unwrap_or_default().1,
            observed.width,
            observed.height,
            requested.0,
            requested.1,
            expected.0,
            expected.1
        ));
    }
    if (i64::from(observed.x) - expected.0).abs() > PLACEMENT_TOLERANCE
        || (i64::from(observed.y) - expected.1).abs() > PLACEMENT_TOLERANCE
    {
        report.failures.push(format!(
            "cold standalone preview did not land on its expected origin: expected={},{} observed={},{} size={}x{}",
            expected.0,
            expected.1,
            observed.x,
            observed.y,
            observed.width,
            observed.height
        ));
    }
    match sampled {
        Some(sampled) => {
            if (sampled.0 - i64::from(pointer.0)).abs() > PLACEMENT_INPUT_SLOP
                || (sampled.1 - i64::from(pointer.1)).abs() > PLACEMENT_INPUT_SLOP
            {
                report.failures.push(format!(
                    "cold standalone preview ignored the guest pointer: pointer={},{} sampled={},{}",
                    pointer.0, pointer.1, sampled.0, sampled.1
                ));
            }
        }
        None => {
            report.failures.push(format!(
                "cold standalone preview sampled no cursor: pointer={},{}",
                pointer.0, pointer.1
            ));
        }
    }
    let held_focus = require_exec(
        guest,
        command("/usr/bin/xdotool", &["getwindowfocus"]),
        COMMAND_TIMEOUT,
    )?
    .stdout
    .trim()
    .to_string();
    if held_focus != window_id {
        require_exec(
            guest,
            command("/usr/bin/xdotool", &["windowfocus", "--sync", &window_id]),
            COMMAND_TIMEOUT,
        )?;
        let refocused = require_exec(
            guest,
            command("/usr/bin/xdotool", &["getwindowfocus"]),
            COMMAND_TIMEOUT,
        )?
        .stdout
        .trim()
        .to_string();
        if refocused != window_id {
            report.failures.push(format!(
                "standalone preview did not hold native input focus: window={window_id} observed={refocused}"
            ));
            return Ok(());
        }
    }
    key(guest, "Escape")?;
    let close_deadline = Instant::now() + ACTION_TIMEOUT;
    loop {
        let search = exec(
            guest,
            command(
                "/usr/bin/xdotool",
                &["search", "--onlyvisible", "--pid", &standalone_pid],
            ),
            Duration::from_secs(2),
        )?;
        if search.state == ProcessState::Exited && search.exit_code == Some(1) {
            break;
        }
        if Instant::now() >= close_deadline {
            report.failures.push(format!(
                "standalone preview window stayed visible after Escape: pid={standalone_pid} search={:?}",
                search.stdout.trim()
            ));
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let wait_budget = ACTION_TIMEOUT + Duration::from_secs(2);
    let wait_request = RequestAction::Wait {
        process_id,
        timeout_ms: u64::try_from(ACTION_TIMEOUT.as_millis())
            .context("guest wait timeout is too large")?,
    };
    let outcome = match guest.request(wait_request, wait_budget) {
        Ok(ResponseResult::Process { outcome }) => outcome,
        Ok(result) => bail!("guest wait returned an unexpected response: {result:?}"),
        Err(error) => bail!("standalone preview wait did not reap: {error:#}"),
    };
    if outcome.state != ProcessState::Exited {
        report.failures.push(format!(
            "standalone preview process did not exit: state={:?} exit={:?} process_id={process_id}",
            outcome.state, outcome.exit_code
        ));
        return Ok(());
    }
    if outcome.exit_code != Some(0) {
        report.failures.push(format!(
            "standalone preview process exited unsuccessfully: state={:?} exit={:?} process_id={process_id}",
            outcome.state, outcome.exit_code
        ));
        return Ok(());
    }
    report.evidence.push(format!(
        "placement_cold_exit process_id={process_id} state={:?} exit=0",
        outcome.state
    ));
    Ok(())
}

fn cursor_adjacent_origin(
    cursor: (i64, i64),
    window: (i64, i64),
    screen_width: i64,
    screen_height: i64,
) -> (i64, i64) {
    let axis = |available: i64, cursor: i64, window: i64| {
        let minimum = CURSOR_ADJACENT_MARGIN;
        let maximum = (available - CURSOR_ADJACENT_MARGIN - window).max(minimum);
        let forward = cursor + CURSOR_ADJACENT_GAP;
        if forward <= maximum {
            forward.max(minimum)
        } else {
            (cursor - CURSOR_ADJACENT_GAP - window).clamp(minimum, maximum)
        }
    };
    (
        axis(screen_width, cursor.0, window.0),
        axis(screen_height, cursor.1, window.1),
    )
}

fn wait_for_geometry_near(
    guest: &mut GuestControlClient,
    window_id: &str,
    expected: (i64, i64),
    tolerance: i64,
) -> Result<WindowGeometry> {
    let deadline = Instant::now() + ACTION_TIMEOUT;
    loop {
        let geometry = window_geometry(guest, window_id)?;
        if (i64::from(geometry.x) - expected.0).abs() <= tolerance
            && (i64::from(geometry.y) - expected.1).abs() <= tolerance
        {
            return Ok(geometry);
        }
        if Instant::now() >= deadline {
            return Ok(geometry);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_adjacent_geometry_near(
    guest: &mut GuestControlClient,
    window_id: &str,
    cursor: (i64, i64),
    screen: (u32, u32),
    tolerance: i64,
) -> Result<(WindowGeometry, (i64, i64))> {
    let deadline = Instant::now() + ACTION_TIMEOUT;
    loop {
        let geometry = window_geometry(guest, window_id)?;
        let expected = cursor_adjacent_origin(
            cursor,
            (i64::from(geometry.width), i64::from(geometry.height)),
            i64::from(screen.0),
            i64::from(screen.1),
        );
        if (i64::from(geometry.x) - expected.0).abs() <= tolerance
            && (i64::from(geometry.y) - expected.1).abs() <= tolerance
        {
            return Ok((geometry, expected));
        }
        if Instant::now() >= deadline {
            return Ok((geometry, expected));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn last_probe_line<'a>(trace: &'a str, tag: &str) -> Option<&'a str> {
    let marker = format!(" {tag} ");
    trace.lines().rev().find(|line| line.contains(&marker))
}

fn probe_point(line: &str, field: &str) -> Result<(i64, i64)> {
    let value =
        probe_field(line, field).with_context(|| format!("probe field {field} was missing"))?;
    let (x, y) = value
        .split_once(',')
        .with_context(|| format!("probe field {field} was not a coordinate pair"))?;
    Ok((x.parse()?, y.parse()?))
}

fn probe_size(line: &str, field: &str) -> Result<(u32, u32)> {
    let value =
        probe_field(line, field).with_context(|| format!("probe field {field} was missing"))?;
    let (width, height) = value
        .split_once('x')
        .with_context(|| format!("probe field {field} was not a size pair"))?;
    Ok((width.parse()?, height.parse()?))
}

#[cfg(test)]
mod tests {
    use super::{
        cursor_adjacent_origin, cursor_motion_frame_count, ffmpeg_timing_adjustments, probe_point,
        probe_size, probe_usize, top_band_yavg_stats,
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
    fn probe_point_and_size_read_integer_pairs() {
        let line = "42 pid=1 SHOT_PREVIEW_PLACE cursor=160,133 origin=180,153 size=370x330";

        assert_eq!(probe_point(line, "origin").unwrap(), (180, 153));
        assert_eq!(probe_point(line, "cursor").unwrap(), (160, 133));
        assert_eq!(probe_size(line, "size").unwrap(), (370, 330));
    }

    #[test]
    fn probe_point_rejects_missing_cursor_and_malformed_pairs() {
        let absent = "42 pid=1 SHOT_PREVIEW_PLACE cursor=none origin=180,153";
        let malformed = "42 pid=1 SHOT_PREVIEW_PLACE cursor=160x133";

        assert!(probe_point(absent, "cursor").is_err());
        assert!(probe_point(malformed, "cursor").is_err());
    }

    #[test]
    fn cursor_adjacent_origin_matches_forward_backward_and_clamped_placement() {
        assert_eq!(
            cursor_adjacent_origin((160, 133), (370, 330), 1280, 800),
            (180, 153)
        );
        assert_eq!(
            cursor_adjacent_origin((1278, 798), (370, 330), 1280, 800),
            (888, 448)
        );
        assert_eq!(
            cursor_adjacent_origin((5, 5), (1200, 700), 1280, 800),
            (25, 25)
        );
    }

    #[test]
    fn ffmpeg_timing_adjustments_use_the_final_progress_sample() {
        let output = "frame=70 dup=4 drop=8\rframe=631 dup=88 drop=44\n";

        assert_eq!(ffmpeg_timing_adjustments(output), (88, 44));
        assert_eq!(ffmpeg_timing_adjustments("frame=587 fps=60"), (0, 0));
    }
}
