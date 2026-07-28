use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::{
    CommandSpec, GuestControlClient, ProcessOutcome, ProcessState, RequestAction, ResponseResult,
};

use crate::progress::{step_label, StepKind};

use super::{DesktopWorkflow, Verdict};
use crate::commands::emu::{qmp, BootedVm};

const GUEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GUEST_HELLO_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GUEST_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const DESKTOP_READY_TIMEOUT: Duration = Duration::from_secs(90);
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const PAYLOAD_ROOT: &str = "/run/qol-payload";
const PAYLOAD_INSTALLER: &str = "/usr/local/libexec/qol-sandbox-payload";
const TRAY_BINARY: &str = "/home/qol/.local/bin/qol-tray";
const HTTP_TOKEN_PATH: &str = "/home/qol/.config/qol-tray/.http-token";
const QOL_SHOT_SOCKET_PATH: &str = "/home/qol/.local/share/qol-tray/runtime/sockets/qol-shot.sock";
const CAPTURE_MARKER: &str = "/tmp/qol-workflow-capture-start";
const PIN_DRAG_HOLD: Duration = Duration::from_millis(80);
const STATUS_SETTLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WindowGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TraceCursor {
    bytes: u64,
}

pub(super) fn run(vm: &BootedVm, workflow: DesktopWorkflow) -> Result<Verdict> {
    match workflow {
        DesktopWorkflow::QolShotCapture => qol_shot_capture(vm),
    }
}

fn qol_shot_capture(vm: &BootedVm) -> Result<Verdict> {
    let environment_id = vm.environment.id.as_str();
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), vm.guest_control_port);
    let expected_revision = vm
        .launch
        .guest_image_revision
        .as_deref()
        .context("desktop workflow has no expected guest image revision")?;
    step_label(
        "guest",
        StepKind::Pending,
        "waiting for the headless Cinnamon guest desktop",
    );
    let mut guest = GuestControlClient::connect_verified_identity(
        address,
        GUEST_CONNECT_TIMEOUT,
        GUEST_HELLO_TIMEOUT,
        environment_id,
        expected_revision,
        &vm.run_id,
    )?;
    step_label(
        "guest",
        StepKind::Success,
        &format!(
            "{} · {} · {}",
            guest.hello().image.revision,
            guest.hello().session.user,
            guest
                .hello()
                .session
                .display
                .as_deref()
                .unwrap_or("unknown display")
        ),
    );

    step_label(
        "payload",
        StepKind::Pending,
        "verifying and installing the staged binaries",
    );
    require_exec(
        &mut guest,
        command(PAYLOAD_INSTALLER, &["install", PAYLOAD_ROOT]),
        Duration::from_secs(60),
    )?;
    step_label(
        "payload",
        StepKind::Success,
        "installed inside the disposable guest overlay",
    );

    spawn(&mut guest, command(TRAY_BINARY, &[]))?;
    let token = wait_for_command(
        &mut guest,
        command("/usr/bin/cat", &[HTTP_TOKEN_PATH]),
        DESKTOP_READY_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "the qol-tray HTTP token",
    )?;
    let auth_header = format!("X-Qol-Token: {}", token.stdout.trim());
    let plugin_api = format!("{}/api/installed", local_base_url());
    wait_for_command(
        &mut guest,
        command(
            "/usr/bin/curl",
            &["--fail", "--silent", "--header", &auth_header, &plugin_api],
        ),
        DESKTOP_READY_TIMEOUT,
        |outcome| outcome.stdout.contains("qol-shot"),
        "qol-shot to appear in the tray plugin API",
    )?;
    wait_for_command(
        &mut guest,
        command("/usr/bin/test", &["-S", QOL_SHOT_SOCKET_PATH]),
        DESKTOP_READY_TIMEOUT,
        |_| true,
        "the qol-shot daemon socket",
    )?;
    wait_for_probe_line(
        &mut guest,
        TraceCursor::default(),
        "SHOT_DAEMON_APP",
        "state=ready",
        DESKTOP_READY_TIMEOUT,
    )?;
    let keepalive = verify_keepalive_window(&mut guest)?;
    step_label(
        "ready",
        StepKind::Success,
        "production tray and qol-shot daemon are ready",
    );

    spawn(
        &mut guest,
        command(
            "/usr/bin/xterm",
            &["-T", "qol-fixture-window", "-geometry", "80x24+100+100"],
        ),
    )?;
    wait_for_command(
        &mut guest,
        command(
            "/usr/bin/xdotool",
            &["search", "--name", "qol-fixture-window"],
        ),
        Duration::from_secs(15),
        |outcome| !outcome.stdout.trim().is_empty(),
        "the deterministic guest fixture window",
    )?;
    let resolution = require_exec(
        &mut guest,
        command("/usr/bin/xrandr", &["--current"]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    let (width, height) = parse_current_resolution(&resolution.stdout)
        .context("guest xrandr output did not declare its current desktop size")?;

    let mut qmp = qmp::connect_verified(vm.qmp_port, Duration::from_secs(10), &vm.run_id)?;
    let pointer_x = 300_u32.min(width.saturating_sub(1));
    let pointer_y = 220_u32.min(height.saturating_sub(1));
    qmp.move_pointer_absolute(pointer_x, pointer_y, width, height)?;

    let capture_trace = current_trace_cursor(&mut guest)?;
    require_exec(
        &mut guest,
        command(TRAY_BINARY, &["exec", "qol-shot", "screenshot"]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    wait_for_probe_line(
        &mut guest,
        capture_trace,
        "SHOT_SELECT_REVEAL",
        "state=presented",
        CAPTURE_TIMEOUT,
    )?;
    wait_for_probe_line(
        &mut guest,
        capture_trace,
        "SHOT_SELECT_OVERLAY",
        "result=mapped",
        CAPTURE_TIMEOUT,
    )?;

    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let selector = artifacts_dir.join("selector.ppm");
    qmp.screendump(&selector)?;
    require_exec(
        &mut guest,
        command("/usr/bin/touch", &[CAPTURE_MARKER]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    let selection_start = (width / 32, height / 32);
    let selection_end = (
        width.saturating_sub(selection_start.0 + 1),
        height.saturating_sub(selection_start.1 + 1),
    );
    qmp.move_pointer_absolute(selection_start.0, selection_start.1, width, height)?;
    qmp.set_left_button(true)?;
    thread::sleep(PIN_DRAG_HOLD);
    qmp.move_pointer_absolute(selection_end.0, selection_end.1, width, height)?;
    thread::sleep(PIN_DRAG_HOLD);
    qmp.set_left_button(false)?;

    let selected_probe = wait_for_probe_line(
        &mut guest,
        capture_trace,
        "SHOT_SELECT_RESULT",
        "rect=",
        CAPTURE_TIMEOUT,
    )?;
    let preview_probe = wait_for_probe_line(
        &mut guest,
        capture_trace,
        "SHOT_PREVIEW_REVEAL",
        "state=presented",
        CAPTURE_TIMEOUT,
    )?;
    let preview_latency_ms =
        screenshot_preview_latency_ms(&selected_probe.stdout, &preview_probe.stdout)?;
    let preview_dimensions = screenshot_preview_dimensions(&preview_probe.stdout)?;
    if preview_dimensions.0 > 360 || preview_dimensions.1 > 240 {
        bail!(
            "screenshot preview carried full-resolution pixels: {}x{}",
            preview_dimensions.0,
            preview_dimensions.1
        );
    }
    wait_for_probe_fields(
        &mut guest,
        capture_trace,
        "SHOW_WIN_STATE",
        &[
            "phase=after",
            "title=qol-shot-preview",
            "target_active=true",
            "map=viewable",
        ],
        CAPTURE_TIMEOUT,
    )?;
    let preview = artifacts_dir.join("preview.ppm");
    qmp.screendump(&preview)?;

    let preview_id = wait_for_window_id_matching(
        &mut guest,
        "^qol-shot-preview",
        "the visible screenshot preview",
        CAPTURE_TIMEOUT,
    )?;
    require_exec(
        &mut guest,
        command("/usr/bin/xdotool", &["windowfocus", "--sync", &preview_id]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    let pin_trace = current_trace_cursor(&mut guest)?;
    qmp.send_keys(&["i".to_string()])?;
    let pin_reveal = wait_for_probe_line(
        &mut guest,
        pin_trace,
        "SHOT_PIN_REVEAL",
        "state=presented",
        CAPTURE_TIMEOUT,
    )?;
    let pin_line = pin_reveal
        .stdout
        .lines()
        .rev()
        .find(|line| probe_line_matches(line, "SHOT_PIN_REVEAL", &["state=presented"]))
        .context("pin reveal probe did not contain its matching line")?;
    let pin_title = probe_field(pin_line, "title")
        .context("pin reveal probe did not identify the pinned window")?;
    wait_for_probe_line(
        &mut guest,
        pin_trace,
        "SHOT_PIN",
        "result=mapped",
        CAPTURE_TIMEOUT,
    )?;
    let pin_id = wait_for_window_id(&mut guest, pin_title, CAPTURE_TIMEOUT)?;
    let before_move = window_geometry(&mut guest, &pin_id)?;
    let (pin_x, pin_y) = geometry_center(before_move, width, height)?;
    let (target_x, target_y) = shifted_pointer(pin_x, pin_y, width, height)?;
    qmp.move_pointer_absolute(pin_x, pin_y, width, height)?;
    let drag_trace = current_trace_cursor(&mut guest)?;
    qmp.set_left_button(true)?;
    thread::sleep(PIN_DRAG_HOLD);
    qmp.move_pointer_absolute(target_x, target_y, width, height)?;
    thread::sleep(PIN_DRAG_HOLD);
    qmp.set_left_button(false)?;
    wait_for_probe_line(
        &mut guest,
        drag_trace,
        "SHOT_PIN_TICK",
        "mode=move",
        CAPTURE_TIMEOUT,
    )?;
    let after_move = wait_for_window_move(&mut guest, &pin_id, before_move, CAPTURE_TIMEOUT)?;
    let pinned = artifacts_dir.join("pinned.ppm");
    qmp.screendump(&pinned)?;
    let recording_cancellation =
        exercise_recording_cancellation(&mut guest, &mut qmp, width, height, &artifacts_dir)?;

    let captured = wait_for_command(
        &mut guest,
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
                "-newer",
                CAPTURE_MARKER,
                "-size",
                "+0c",
                "-printf",
                "%p\t%s\\n",
            ],
        ),
        CAPTURE_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "a saved screenshot file",
    )?;

    let probes = require_exec(
        &mut guest,
        command(
            "/usr/bin/grep",
            &[
                "-E",
                "SHOT_(CAPTURE|CAPTURE_STATUS|DAEMON_APP|FILE|FREEZE|PIN_REVEAL|PIN_TICK|PREVIEW_PLACE|PREVIEW_REVEAL|RECORD_COUNTDOWN|RECORD_TOGGLE|RECV|SCREENSHOT_READY|SELECT_OVERLAY|SELECT_RESULT|SELECT_REVEAL|WINDOW_OPEN)|SHOW_WIN_STATE",
                TRACE_LOG_PATH,
            ],
        ),
        GUEST_COMMAND_TIMEOUT,
    )?;
    let mut traces = probes
        .stdout
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    traces.push(format!("keepalive={keepalive}"));
    traces.push(format!("captured={}", captured.stdout.trim()));
    traces.push(format!(
        "pin_move={},{}->{},{}",
        before_move.x, before_move.y, after_move.x, after_move.y
    ));
    traces.push(format!("preview_latency_ms={preview_latency_ms}"));
    traces.push(format!(
        "preview_dimensions={}x{}",
        preview_dimensions.0, preview_dimensions.1
    ));
    let mut artifacts = vec![selector, preview, pinned];
    artifacts.extend(recording_cancellation);
    Ok(Verdict {
        pass: true,
        traces,
        artifacts,
    })
}

fn exercise_recording_cancellation(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    width: u32,
    height: u32,
    artifacts_dir: &Path,
) -> Result<[PathBuf; 2]> {
    let pointer_x = 300_u32.min(width.saturating_sub(1));
    let pointer_y = 220_u32.min(height.saturating_sub(1));
    qmp.move_pointer_absolute(pointer_x, pointer_y, width, height)?;
    let trace = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command(TRAY_BINARY, &["exec", "qol-shot", "record"]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    wait_for_probe_line(
        guest,
        trace,
        "SHOT_SELECT_REVEAL",
        "state=presented",
        CAPTURE_TIMEOUT,
    )?;
    qmp.click_left()?;
    wait_for_probe_fields(
        guest,
        trace,
        "SHOT_RECORD_COUNTDOWN",
        &["phase=shown", "seconds=3"],
        CAPTURE_TIMEOUT,
    )?;
    let countdown = artifacts_dir.join("recording-countdown.ppm");
    qmp.screendump(&countdown)?;
    require_exec(
        guest,
        command(TRAY_BINARY, &["exec", "qol-shot", "record"]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        trace,
        "SHOT_RECORD_TOGGLE",
        &["source=daemon", "result=countdown-cancelled"],
        CAPTURE_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        trace,
        "SHOT_CAPTURE_STATUS",
        &[
            "context=recording",
            "stage=cancelled",
            "surface=shared-toast",
            "shown=true",
        ],
        CAPTURE_TIMEOUT,
    )?;
    thread::sleep(STATUS_SETTLE);
    let cancelled = artifacts_dir.join("recording-cancelled.ppm");
    qmp.screendump(&cancelled)?;
    Ok([countdown, cancelled])
}

fn command(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        cwd: None,
        env: BTreeMap::new(),
    }
}

fn spawn(guest: &mut GuestControlClient, command: CommandSpec) -> Result<u64> {
    match guest.request(RequestAction::Spawn { command }, GUEST_COMMAND_TIMEOUT)? {
        ResponseResult::Spawned { process_id, .. } => Ok(process_id),
        result => bail!("guest spawn returned an unexpected response: {result:?}"),
    }
}

fn exec(
    guest: &mut GuestControlClient,
    command: CommandSpec,
    timeout: Duration,
) -> Result<ProcessOutcome> {
    let timeout_ms = u64::try_from(timeout.as_millis()).context("guest timeout is too large")?;
    match guest.request(
        RequestAction::Exec {
            command,
            timeout_ms,
        },
        timeout + Duration::from_secs(2),
    )? {
        ResponseResult::Process { outcome } => Ok(outcome),
        result => bail!("guest exec returned an unexpected response: {result:?}"),
    }
}

fn require_exec(
    guest: &mut GuestControlClient,
    command: CommandSpec,
    timeout: Duration,
) -> Result<ProcessOutcome> {
    let program = command.program.clone();
    let outcome = exec(guest, command, timeout)?;
    if outcome.state == ProcessState::Exited && outcome.exit_code == Some(0) {
        return Ok(outcome);
    }
    bail!(
        "guest command `{program}` failed: state={:?}, exit={:?}, stderr={}",
        outcome.state,
        outcome.exit_code,
        outcome.stderr.trim()
    )
}

fn wait_for_command(
    guest: &mut GuestControlClient,
    command: CommandSpec,
    timeout: Duration,
    predicate: impl Fn(&ProcessOutcome) -> bool,
    description: &str,
) -> Result<ProcessOutcome> {
    let deadline = Instant::now() + timeout;
    loop {
        let outcome = exec(guest, command.clone(), Duration::from_secs(2))?;
        if outcome.state == ProcessState::Exited
            && outcome.exit_code == Some(0)
            && predicate(&outcome)
        {
            return Ok(outcome);
        }
        if Instant::now() >= deadline {
            let detail = format!(
                "last state={:?}, exit={:?}, stderr={}",
                outcome.state,
                outcome.exit_code,
                outcome.stderr.trim()
            );
            bail!("timed out waiting for {description}: {detail}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_probe_line(
    guest: &mut GuestControlClient,
    cursor: TraceCursor,
    tag: &str,
    required: &str,
    timeout: Duration,
) -> Result<ProcessOutcome> {
    wait_for_probe_fields(guest, cursor, tag, &[required], timeout)
}

fn wait_for_probe_fields(
    guest: &mut GuestControlClient,
    cursor: TraceCursor,
    tag: &str,
    required: &[&str],
    timeout: Duration,
) -> Result<ProcessOutcome> {
    wait_for_command(
        guest,
        trace_tail_command(cursor)?,
        timeout,
        |outcome| {
            outcome
                .stdout
                .lines()
                .any(|line| probe_line_matches(line, tag, required))
        },
        &format!("guest probe `{tag}` containing `{}`", required.join("`, `")),
    )
}

fn current_trace_cursor(guest: &mut GuestControlClient) -> Result<TraceCursor> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/stat", &["--format=%s", TRACE_LOG_PATH]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    let bytes = outcome
        .stdout
        .trim()
        .parse()
        .context("guest trace size was not an unsigned integer")?;
    Ok(TraceCursor { bytes })
}

fn trace_tail_command(cursor: TraceCursor) -> Result<CommandSpec> {
    trace_tail_command_for(cursor, TRACE_LOG_PATH)
}

fn trace_tail_command_for(cursor: TraceCursor, path: &str) -> Result<CommandSpec> {
    let first_byte = cursor
        .bytes
        .checked_add(1)
        .context("guest trace cursor overflow")?;
    Ok(command(
        "/usr/bin/tail",
        &["-c", &format!("+{first_byte}"), path],
    ))
}

fn probe_line_matches(line: &str, tag: &str, required: &[&str]) -> bool {
    line.contains(&format!(" {tag} ")) && required.iter().all(|field| line.contains(field))
}

fn probe_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field}=");
    line.split_ascii_whitespace()
        .find_map(|token| token.strip_prefix(&prefix))
}

fn screenshot_preview_latency_ms(selected_trace: &str, preview_trace: &str) -> Result<u64> {
    let selected = matching_probe_line(selected_trace, "SHOT_SELECT_RESULT", &["rect="])
        .context("screenshot selection trace was missing")?;
    let preview = matching_probe_line(preview_trace, "SHOT_PREVIEW_REVEAL", &["state=presented"])
        .context("screenshot preview trace was missing")?;
    let selected_ms = probe_timestamp_ms(selected)?;
    let preview_ms = probe_timestamp_ms(preview)?;
    for line in preview_trace.lines() {
        if probe_line_matches(
            line,
            "SHOT_CAPTURE_STATUS",
            &["context=screenshot", "stage=saving"],
        ) && probe_timestamp_ms(line)? <= preview_ms
        {
            bail!("screenshot saving status was presented before the thumbnail preview");
        }
        if probe_line_matches(line, "SHOT_FILE", &[]) && probe_timestamp_ms(line)? <= preview_ms {
            bail!("screenshot file encoding ran before the thumbnail preview");
        }
    }
    preview_ms
        .checked_sub(selected_ms)
        .context("screenshot preview trace preceded its selection result")
}

fn screenshot_preview_dimensions(trace: &str) -> Result<(u32, u32)> {
    let capture = matching_probe_line(
        trace,
        "SHOT_CAPTURE",
        &["source=frozen-preview", "preview="],
    )
    .context("screenshot trace did not contain a bounded frozen preview")?;
    let dimensions =
        probe_field(capture, "preview").context("screenshot preview dimensions were missing")?;
    let (width, height) = dimensions
        .split_once('x')
        .context("screenshot preview dimensions were malformed")?;
    Ok((
        width
            .parse()
            .context("screenshot preview width was malformed")?,
        height
            .parse()
            .context("screenshot preview height was malformed")?,
    ))
}

fn matching_probe_line<'a>(trace: &'a str, tag: &str, required: &[&str]) -> Option<&'a str> {
    trace
        .lines()
        .rev()
        .find(|line| probe_line_matches(line, tag, required))
}

fn probe_timestamp_ms(line: &str) -> Result<u64> {
    line.split_ascii_whitespace()
        .next()
        .context("probe line had no timestamp")?
        .parse()
        .context("probe timestamp was not an unsigned integer")
}

fn wait_for_window_id(
    guest: &mut GuestControlClient,
    title: &str,
    timeout: Duration,
) -> Result<String> {
    let pattern = format!("^{title}$");
    wait_for_window_id_matching(
        guest,
        &pattern,
        &format!("the visible guest window `{title}`"),
        timeout,
    )
}

fn wait_for_window_id_matching(
    guest: &mut GuestControlClient,
    pattern: &str,
    description: &str,
    timeout: Duration,
) -> Result<String> {
    let outcome = wait_for_command(
        guest,
        command(
            "/usr/bin/xdotool",
            &["search", "--onlyvisible", "--name", pattern],
        ),
        timeout,
        |outcome| parse_window_id(&outcome.stdout).is_some(),
        description,
    )?;
    parse_window_id(&outcome.stdout)
        .map(|id| id.to_string())
        .context("xdotool returned no numeric window id")
}

fn verify_keepalive_window(guest: &mut GuestControlClient) -> Result<String> {
    let tree = require_exec(
        guest,
        command("/usr/bin/xwininfo", &["-root", "-tree"]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    let window_id = keepalive_window_id(&tree.stdout)
        .context("qol-shot daemon keepalive was not present in the X11 window tree")?;
    let hints = require_exec(
        guest,
        command("/usr/bin/xprop", &["-id", &window_id, "WM_HINTS"]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    if !keepalive_refuses_input_focus(&hints.stdout) {
        bail!(
            "qol-shot daemon keepalive {window_id} does not declare WM_HINTS input: False: {}",
            hints.stdout.trim()
        );
    }
    let details = require_exec(
        guest,
        command("/usr/bin/xwininfo", &["-id", &window_id]),
        GUEST_COMMAND_TIMEOUT,
    )?;
    if !keepalive_is_unmapped(&details.stdout) {
        bail!(
            "qol-shot daemon keepalive {window_id} is still mapped: {}",
            details.stdout.trim()
        );
    }
    Ok(format!("id={window_id} input=false map=unmapped"))
}

fn keepalive_window_id(tree: &str) -> Option<String> {
    tree.lines()
        .find(|line| line.contains("\"qol-tray-shot-keepalive-"))
        .and_then(|line| line.split_ascii_whitespace().next())
        .filter(|window_id| window_id.starts_with("0x"))
        .map(str::to_owned)
}

fn keepalive_refuses_input_focus(hints: &str) -> bool {
    hints.lines().map(str::trim).any(|line| {
        matches!(
            line,
            "input: False" | "Client accepts input or input focus: False"
        )
    })
}

fn keepalive_is_unmapped(details: &str) -> bool {
    details
        .lines()
        .any(|line| line.trim() == "Map State: IsUnMapped")
}

fn parse_window_id(output: &str) -> Option<u64> {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.parse().ok())
}

fn window_geometry(guest: &mut GuestControlClient, window_id: &str) -> Result<WindowGeometry> {
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/xdotool",
            &["getwindowgeometry", "--shell", window_id],
        ),
        GUEST_COMMAND_TIMEOUT,
    )?;
    parse_window_geometry(&outcome.stdout).context("xdotool returned invalid window geometry")
}

fn parse_window_geometry(output: &str) -> Option<WindowGeometry> {
    let value = |key: &str| {
        output.lines().find_map(|line| {
            let (name, value) = line.trim().split_once('=')?;
            (name == key).then_some(value)
        })
    };
    let geometry = WindowGeometry {
        x: value("X")?.parse().ok()?,
        y: value("Y")?.parse().ok()?,
        width: value("WIDTH")?.parse().ok()?,
        height: value("HEIGHT")?.parse().ok()?,
    };
    (geometry.width > 0 && geometry.height > 0).then_some(geometry)
}

fn geometry_center(
    geometry: WindowGeometry,
    desktop_width: u32,
    desktop_height: u32,
) -> Result<(u32, u32)> {
    let x = i64::from(geometry.x) + i64::from(geometry.width / 2);
    let y = i64::from(geometry.y) + i64::from(geometry.height / 2);
    let x = u32::try_from(x).context("pinned window center is left of the guest desktop")?;
    let y = u32::try_from(y).context("pinned window center is above the guest desktop")?;
    if x >= desktop_width || y >= desktop_height {
        bail!("pinned window center is outside the guest desktop");
    }
    Ok((x, y))
}

fn shifted_pointer(x: u32, y: u32, width: u32, height: u32) -> Result<(u32, u32)> {
    let shift = |value: u32, extent: u32, distance: u32| {
        let forward = value.saturating_add(distance).min(extent.saturating_sub(1));
        if forward != value {
            return forward;
        }
        value.saturating_sub(distance)
    };
    let target = (shift(x, width, 80), shift(y, height, 50));
    if target == (x, y) {
        bail!("guest desktop is too small to exercise pinned-window movement");
    }
    Ok(target)
}

fn wait_for_window_move(
    guest: &mut GuestControlClient,
    window_id: &str,
    before: WindowGeometry,
    timeout: Duration,
) -> Result<WindowGeometry> {
    let deadline = Instant::now() + timeout;
    loop {
        let current = window_geometry(guest, window_id)?;
        if (current.x, current.y) != (before.x, before.y) {
            return Ok(current);
        }
        if Instant::now() >= deadline {
            bail!("pinned window did not move after guest pointer drag");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn parse_current_resolution(output: &str) -> Option<(u32, u32)> {
    let marker = " current ";
    let start = output.find(marker)? + marker.len();
    let rest = &output[start..];
    let (width, rest) = rest.split_once(' ')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('x')?.trim_start();
    let height = rest
        .split(|character: char| !character.is_ascii_digit())
        .next()?;
    let width = width.parse().ok()?;
    let height = height.parse().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xrandr_current_desktop_size() {
        let output = "Screen 0: minimum 320 x 200, current 1280 x 800, maximum 16384 x 16384\n";
        assert_eq!(parse_current_resolution(output), Some((1280, 800)));
        assert_eq!(parse_current_resolution("current 0 x 0"), None);
        assert_eq!(parse_current_resolution("not xrandr"), None);
    }

    #[test]
    fn probe_matching_requires_the_exact_tag_and_every_field() {
        let line = "123 456 SHOT_PIN_REVEAL title=qol-shot-pin-7 state=presented";
        assert!(probe_line_matches(
            line,
            "SHOT_PIN_REVEAL",
            &["title=qol-shot-pin", "state=presented"]
        ));
        assert!(!probe_line_matches(line, "SHOT_PIN", &["state=presented"]));
        assert!(!probe_line_matches(
            line,
            "SHOT_PIN_REVEAL",
            &["state=mapped"]
        ));
        assert_eq!(probe_field(line, "title"), Some("qol-shot-pin-7"));
    }

    #[test]
    fn screenshot_preview_latency_rejects_status_before_thumbnail() {
        let selected = "100 pid=1 SHOT_SELECT_RESULT rect=10x10+0,0\n";
        let healthy = concat!(
            "100 pid=1 SHOT_SELECT_RESULT rect=10x10+0,0\n",
            "145 pid=1 SHOT_PREVIEW_REVEAL state=presented preview_ms=40\n"
        );
        let regressed = concat!(
            "100 pid=1 SHOT_SELECT_RESULT rect=10x10+0,0\n",
            "120 pid=1 SHOT_CAPTURE_STATUS context=screenshot stage=saving\n",
            "145 pid=1 SHOT_PREVIEW_REVEAL state=presented preview_ms=40\n"
        );
        let deferred = concat!(
            "100 pid=1 SHOT_SELECT_RESULT rect=10x10+0,0\n",
            "145 pid=1 SHOT_PREVIEW_REVEAL state=presented preview_ms=40\n",
            "150 pid=1 SHOT_CAPTURE_STATUS context=screenshot stage=saving\n"
        );
        let encoding_first = concat!(
            "100 pid=1 SHOT_SELECT_RESULT rect=10x10+0,0\n",
            "130 pid=1 SHOT_FILE source=frozen ms=20 result=ok\n",
            "145 pid=1 SHOT_PREVIEW_REVEAL state=presented preview_ms=40\n"
        );

        assert_eq!(
            screenshot_preview_latency_ms(selected, healthy).unwrap(),
            45
        );
        assert!(screenshot_preview_latency_ms(selected, regressed).is_err());
        assert!(screenshot_preview_latency_ms(selected, encoding_first).is_err());
        assert_eq!(
            screenshot_preview_latency_ms(selected, deferred).unwrap(),
            45
        );
    }

    #[test]
    fn screenshot_preview_dimensions_require_the_bounded_preview_path() {
        let trace = concat!(
            "100 pid=1 SHOT_SELECT_RESULT rect=3840x2160+0,0\n",
            "108 pid=1 SHOT_CAPTURE source=frozen-preview ms=7 ",
            "rect=3840x2160 preview=360x203\n"
        );

        assert_eq!(screenshot_preview_dimensions(trace).unwrap(), (360, 203));
        assert!(screenshot_preview_dimensions(
            "108 pid=1 SHOT_CAPTURE source=frozen ms=7 rect=3840x2160\n"
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn trace_cursor_excludes_matching_probes_written_before_the_action() {
        let stale = "100 pid=1 SHOT_SELECT_OVERLAY result=mapped\n";
        let fresh = "200 pid=1 SHOT_SELECT_OVERLAY result=timeout\n";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.log");
        std::fs::write(&path, format!("{stale}{fresh}")).unwrap();
        let command = trace_tail_command_for(
            TraceCursor {
                bytes: stale.len() as u64,
            },
            path.to_str().unwrap(),
        )
        .unwrap();
        let output = std::process::Command::new(command.program)
            .args(command.args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let trace = String::from_utf8(output.stdout).unwrap();
        assert_eq!(trace, fresh);
        assert!(!trace.lines().any(|line| probe_line_matches(
            line,
            "SHOT_SELECT_OVERLAY",
            &["result=mapped"]
        )));
    }

    #[test]
    fn parses_guest_window_identity_and_geometry() {
        assert_eq!(parse_window_id("not-an-id\n4194307\n"), Some(4_194_307));
        assert_eq!(
            parse_window_geometry("WINDOW=4194307\nX=-20\nY=72\nWIDTH=640\nHEIGHT=480\n"),
            Some(WindowGeometry {
                x: -20,
                y: 72,
                width: 640,
                height: 480,
            })
        );
        assert_eq!(
            parse_window_geometry("X=0\nY=0\nWIDTH=0\nHEIGHT=480\n"),
            None
        );
    }

    #[test]
    fn keepalive_evidence_requires_the_nonfocusable_unmapped_contract() {
        let tree = concat!(
            "  0x4800001 \"qol-tray-shot-keepalive-1502\": (\"qol-tray-shot\" \"qol-tray-shot\")  1x1+0+0  +0+0\n",
            "  0x4800002 \"qol-shot-preview-1502\": (\"qol-tray-shot\" \"qol-tray-shot\")  370x329+0+0  +0+0\n",
        );
        let hints = "WM_HINTS(WM_HINTS):\n\tinput: False\n";
        let mint_hints = "WM_HINTS(WM_HINTS):\n\tClient accepts input or input focus: False\n";
        let mapped = "  Map State: IsViewable\n";
        let unmapped = "  Map State: IsUnMapped\n";

        assert_eq!(keepalive_window_id(tree).as_deref(), Some("0x4800001"));
        assert!(keepalive_refuses_input_focus(hints));
        assert!(keepalive_refuses_input_focus(mint_hints));
        assert!(!keepalive_refuses_input_focus("WM_HINTS:  not found."));
        assert!(keepalive_is_unmapped(unmapped));
        assert!(!keepalive_is_unmapped(mapped));
    }

    #[test]
    fn pin_drag_points_stay_inside_the_guest_desktop() {
        let center = geometry_center(
            WindowGeometry {
                x: 100,
                y: 120,
                width: 400,
                height: 300,
            },
            1280,
            800,
        )
        .unwrap();
        assert_eq!(center, (300, 270));
        let shifted = shifted_pointer(center.0, center.1, 1280, 800).unwrap();
        assert_eq!(shifted, (380, 320));
        assert!(shifted.0 < 1280 && shifted.1 < 800);
    }

    #[test]
    fn guest_commands_are_absolute_typed_argv() {
        let command = command(TRAY_BINARY, &["exec", "qol-shot", "screenshot"]);
        command.validate().unwrap();
        assert_eq!(
            command.args,
            ["exec", "qol-shot", "screenshot"].map(str::to_string)
        );
    }

    #[test]
    fn evidence_paths_stay_inside_the_lane_run() {
        let root = std::path::Path::new("/runs/cases/lane-1");
        for name in [
            "selector.ppm",
            "preview.ppm",
            "pinned.ppm",
            "recording-countdown.ppm",
            "recording-cancelled.ppm",
        ] {
            let path: std::path::PathBuf = root.join("artifacts").join(name);
            assert!(path.starts_with(root));
        }
    }
}
