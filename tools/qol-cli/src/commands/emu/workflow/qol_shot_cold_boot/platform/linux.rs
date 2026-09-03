use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use qol_conventions::TRACE_LOG_PATH;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_after_reboot, connect_desktop_guest, desktop_resolution, dispatch,
    install_payload, kill_guest_screensaver, launch_fixture, reboot_guest_cleanly, require_exec,
    require_visible_window_count, wait_for_autostart_tray, wait_for_plugin, wait_for_probe_fields,
    write_autostart, TraceCursor, ACTION_TIMEOUT, BOOT_READY_TIMEOUT, PLUGIN_ID, PREVIEW_PREFIX,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const BOOT_SETTLE: Duration = Duration::from_secs(45);
const SETTLE_AFTER_REVEAL: Duration = Duration::from_secs(6);
const FIXTURE_TITLE: &str = "qol-shot-cold-fixture";
const TRACE_PATTERN: &str = "SHOT_DAEMON_APP|SHOT_PREWARM|OVERRIDE_REDIRECT|SHOT_WINDOW_OPEN|SHOT_PREVIEW_REVEAL|SHOT_PARKED_REVEAL|SHOT_PREVIEW_CLOSE|PREPARE_WIN|PICKER_OVERLAY|SHOW_WIN_STATE|FOCUS_REASSERT";

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    write_autostart(&mut guest)?;
    reboot_guest_cleanly(&mut guest)?;
    drop(guest);
    step_label(
        "reboot",
        StepKind::Pending,
        "rebooting so the tray and the qol-shot daemon autostart cold",
    );
    thread::sleep(BOOT_SETTLE);

    let boot_started = Instant::now();
    let mut guest = connect_after_reboot(vm)?;
    let auth = wait_for_autostart_tray(&mut guest)?;
    wait_for_plugin(&mut guest, PLUGIN_ID, &auth)?;
    wait_for_probe_fields(
        &mut guest,
        TraceCursor::default(),
        "SHOT_DAEMON_APP",
        &["state=ready"],
        BOOT_READY_TIMEOUT,
    )?;
    kill_guest_screensaver(&mut guest)?;
    launch_fixture(&mut guest, FIXTURE_TITLE)?;
    let (width, height) = desktop_resolution(&mut guest)?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;

    qmp.move_pointer_absolute(
        300_u32.min(width.saturating_sub(1)),
        220_u32.min(height.saturating_sub(1)),
        width,
        height,
    )?;
    let cursor = dispatch(&mut guest, &auth, "screenshot")?;
    wait_for_probe_fields(
        &mut guest,
        cursor,
        "SHOT_SELECT_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    qmp.set_left_button(true)?;
    for step in 0..8 {
        let x = 300 + (640 - 300) * step / 7;
        let y = 220 + (460 - 220) * step / 7;
        qmp.move_pointer_absolute(x, y, width, height)?;
        thread::sleep(Duration::from_millis(30));
    }
    qmp.set_left_button(false)?;

    wait_for_probe_fields(
        &mut guest,
        cursor,
        "SHOT_PREVIEW_REVEAL",
        &["state=presented"],
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        &mut guest,
        cursor,
        "SHOT_SCREENSHOT_READY",
        &["result=saved"],
        ACTION_TIMEOUT,
    )?;
    require_visible_window_count(&mut guest, PREVIEW_PREFIX, 1)?;
    let preview_cold = artifacts_dir.join("preview-cold.ppm");
    qmp.screendump(&preview_cold)?;

    thread::sleep(SETTLE_AFTER_REVEAL);
    require_visible_window_count(&mut guest, PREVIEW_PREFIX, 1)?;
    let preview_cold_settled = artifacts_dir.join("preview-cold-settled.ppm");
    qmp.screendump(&preview_cold_settled)?;

    let probes = require_exec(
        &mut guest,
        command("/usr/bin/grep", &["-E", TRACE_PATTERN, TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    let mut traces = probes
        .stdout
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let boot_to_capture_ms = boot_started.elapsed().as_millis();
    traces.push(format!("boot_to_capture_ms={boot_to_capture_ms}"));

    let final_state = artifacts_dir.join("final.ppm");
    qmp.screendump(&final_state)?;
    step_label(
        "cold-preview",
        StepKind::Success,
        "the first cold-boot capture showed its preview immediately and it stayed visible past the reveal guard",
    );
    Ok(Verdict {
        pass: true,
        traces,
        artifacts: vec![preview_cold, preview_cold_settled, final_state],
    })
}
