use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};
use qol_windowing::display::DisplayHandle;

use crate::monitor::layout::{
    layout_rows, mode_lists, mode_rows, resolve_arrange, resolve_config_layout, resolve_mode,
    snapshot_for, ArrangeRequest, LayoutPosition, LayoutRow, ModeRow,
};
use crate::monitor::{
    BrightnessState, DisplayControl, DisplaySnapshot, GrantBackend, I2cGrantState, MonitorError,
    RevokeOutcome, BRIGHTNESS_MAX, BRIGHTNESS_MIN, BRIGHTNESS_STEP,
};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "plugin-monitor";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args: Vec<String> = args.into_iter().collect();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return app().run(vec!["daemon".to_string()]);
    }
    app().run(args)
}

fn app() -> HeadlessApp {
    let control = crate::platform::control();
    let config_root = crate::config::config_root();
    let (device, _origin) = crate::config::load_with_origin(config_root.as_deref());
    crate::platform::apply_configured_policies(&control, &device);
    app_with_config_root(
        control,
        Arc::new(crate::monitor::UdevGrantBackend),
        config_root,
    )
}

#[cfg(test)]
fn app_with<C, G>(control: C, grant: G) -> HeadlessApp
where
    C: DisplayControl + Send + Sync + 'static,
    G: GrantBackend + Send + Sync + 'static,
{
    app_with_config_root(Arc::new(control), Arc::new(grant), None)
}

fn app_with_config_root(
    control: Arc<dyn DisplayControl>,
    grant: Arc<dyn GrantBackend>,
    config_root: Option<PathBuf>,
) -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Inspect and control display brightness, gamma, and modes.")
        .default_command(["list"])
        .command(list_command(Arc::clone(&control)))
        .command(status_command(Arc::clone(&control)))
        .command(get_command(Arc::clone(&control)))
        .command(set_command(Arc::clone(&control)))
        .command(up_command(Arc::clone(&control)))
        .command(down_command(Arc::clone(&control)))
        .command(modes_command(Arc::clone(&control)))
        .command(set_mode_command(Arc::clone(&control)))
        .command(layout_command(Arc::clone(&control)))
        .command(arrange_command(Arc::clone(&control)))
        .command(primary_command(Arc::clone(&control)))
        .command(apply_layout_command(
            Arc::clone(&control),
            config_root.clone(),
        ))
        .command(daemon_command())
        .command(open_command())
        .command(grant_command(Arc::clone(&grant)))
        .command(revoke_command(Arc::clone(&grant)))
        .command(settings_command())
        .doctor_checks(doctor_checks(
            Arc::clone(&control),
            Arc::clone(&grant),
            config_root,
        ))
}

fn list_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("list")
        .about("List connected displays with their stable identity.")
        .usage(format!("{BINARY_NAME} list"))
        .output("One `connector id` line per connected display.")
        .exit_behavior("Exits non-zero if display enumeration fails.")
        .run_plain_text(move |_| {
            let handles = control
                .enumerate()
                .context("failed to enumerate displays")?;
            let lines = handles
                .iter()
                .map(|handle| format!("{} {}", handle.connector(), handle.id()))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(PlainTextOutput::text(lines))
        })
}

fn status_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("status")
        .about("Show per-display probe results.")
        .usage(format!("{BINARY_NAME} status"))
        .output("One capability block per connected display.")
        .exit_behavior("Exits non-zero if enumeration or probing fails.")
        .run_plain_text(move |_| {
            let handles = control.enumerate().context("failed to enumerate displays")?;
            let mut blocks = Vec::new();
            for handle in &handles {
                let capabilities = control
                    .probe(handle)
                    .with_context(|| format!("failed to probe {}", handle.connector()))?;
                blocks.push(format!(
                    "{}\n  id: {}\n  identity: {}\n  brightness_ddc: {}\n  brightness_gamma: {}\n  contrast: {}\n  modes: {}\n  hdr: {}",
                    handle.connector(),
                    handle.id(),
                    if handle.identity_unstable() { "unstable" } else { "stable" },
                    yes_no(capabilities.brightness_ddc),
                    yes_no(capabilities.brightness_gamma),
                    yes_no(capabilities.contrast),
                    yes_no(capabilities.modes),
                    yes_no(capabilities.hdr),
                ));
            }
            let output = if blocks.is_empty() {
                "no displays connected".to_string()
            } else {
                blocks.join("\n")
            };
            Ok(PlainTextOutput::text(output))
        })
}

fn get_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("get")
        .about("Show the brightness of a display with its source.")
        .usage(format!("{BINARY_NAME} get [display]"))
        .output("Prints `brightness=<value> source=<ddc|gamma>`.")
        .exit_behavior("Exits non-zero if the display or its brightness is unavailable.")
        .run_plain_text(move |context| {
            let state = brightness_for(control.as_ref(), context)?;
            Ok(PlainTextOutput::text(format!(
                "brightness={} source={}",
                state.value,
                state.source.label()
            )))
        })
}

fn set_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("set")
        .about("Set the brightness of a display through its selected source.")
        .usage(format!("{BINARY_NAME} set <value> [display]"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero on invalid value or when no source can set brightness.")
        .run_plain_text(move |context| {
            let value = parse_brightness_value(context)?;
            let handle =
                select_display(control.as_ref(), context.args().get(1).map(String::as_str))?;
            control
                .set_brightness(&handle, value)
                .with_context(|| format!("failed to set brightness on {}", handle.connector()))?;
            Ok(PlainTextOutput::empty())
        })
}

fn up_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("up")
        .about("Step brightness up by one step.")
        .usage(format!("{BINARY_NAME} up [display]"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero when brightness cannot be stepped.")
        .run_plain_text(move |context| {
            step_brightness(control.as_ref(), context, 1)?;
            Ok(PlainTextOutput::empty())
        })
}

fn down_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("down")
        .about("Step brightness down by one step.")
        .usage(format!("{BINARY_NAME} down [display]"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero when brightness cannot be stepped.")
        .run_plain_text(move |context| {
            step_brightness(control.as_ref(), context, -1)?;
            Ok(PlainTextOutput::empty())
        })
}

fn modes_command(control: Arc<dyn DisplayControl>) -> Command {
    let plain = Arc::clone(&control);
    let json = Arc::clone(&control);
    Command::new("modes")
        .about("List the selectable modes of a display, with the current mode marked.")
        .usage(format!("{BINARY_NAME} modes [display]"))
        .output(
            "One `connector WxH@Hz` line per mode with `current` marked, or a JSON array \
             with --json.",
        )
        .exit_behavior(
            "Exits non-zero when the display layout cannot be read, the selected display is \
             unknown, or a mode list is unreadable.",
        )
        .run_plain_text(move |context| {
            let rows =
                collect_mode_rows(plain.as_ref(), context.args().first().map(String::as_str))?;
            if rows.is_empty() {
                return Ok(PlainTextOutput::text("no modes reported"));
            }
            Ok(PlainTextOutput::text(
                rows.iter().map(mode_line).collect::<Vec<_>>().join("\n"),
            ))
        })
        .run_json(move |context| {
            let rows =
                collect_mode_rows(json.as_ref(), context.args().first().map(String::as_str))?;
            Ok(serde_json::json!(rows))
        })
}

fn set_mode_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("set-mode")
        .about("Set the resolution and refresh rate of a display.")
        .usage(format!(
            "{BINARY_NAME} set-mode <WIDTHxHEIGHT[@HZ]> [display]"
        ))
        .output("No stdout on success.")
        .exit_behavior(
            "Exits non-zero when the mode does not exist, a resolution is ambiguous without a \
             refresh rate, the display is unknown, or the write is refused.",
        )
        .run_plain_text(move |context| {
            let request = parse_mode_spec(
                context
                    .args()
                    .first()
                    .map(String::as_str)
                    .context("set-mode requires a mode as WIDTHxHEIGHT or WIDTHxHEIGHT@HZ")?,
            )?;
            apply_mode(
                control.as_ref(),
                request,
                context.args().get(1).map(String::as_str),
            )?;
            Ok(PlainTextOutput::empty())
        })
}

fn layout_command(control: Arc<dyn DisplayControl>) -> Command {
    let plain = Arc::clone(&control);
    let json = Arc::clone(&control);
    Command::new("layout")
        .about("Show the position, size, refresh rate, and primary flag of every display.")
        .usage(format!("{BINARY_NAME} layout"))
        .output(
            "One `connector id +x+y WxH@Hz` line per display with `primary` marked, or a JSON \
             array with --json.",
        )
        .exit_behavior("Exits non-zero when the display layout cannot be read.")
        .run_plain_text(move |_| {
            let snapshots = plain
                .snapshot()
                .context("failed to read the display layout")?;
            let rows = layout_rows(&snapshots);
            if rows.is_empty() {
                return Ok(PlainTextOutput::text("no displays connected"));
            }
            Ok(PlainTextOutput::text(
                rows.iter().map(layout_line).collect::<Vec<_>>().join("\n"),
            ))
        })
        .run_json(move |_| {
            let snapshots = json
                .snapshot()
                .context("failed to read the display layout")?;
            Ok(serde_json::json!(layout_rows(&snapshots)))
        })
}

fn arrange_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("arrange")
        .about("Move displays to absolute positions in one atomic write.")
        .usage(format!(
            "{BINARY_NAME} arrange <display>=<x>,<y> [more...] [--primary <display>]"
        ))
        .output("No stdout on success.")
        .exit_behavior(
            "Exits non-zero when an assignment is malformed, a display is unknown, a position is \
             outside the supported range, rectangles overlap, or the write fails.",
        )
        .run_plain_text(move |context| {
            let (requested, primary) = parse_arrange_args(context.args())?;
            let snapshots = control
                .snapshot()
                .context("failed to read the display layout")?;
            let placements = resolve_arrange(&snapshots, &requested, primary.as_deref())?;
            control
                .set_layout(&placements)
                .context("failed to apply the display layout")?;
            Ok(PlainTextOutput::empty())
        })
}

fn primary_command(control: Arc<dyn DisplayControl>) -> Command {
    Command::new("primary")
        .about("Set the primary display without moving any display.")
        .usage(format!("{BINARY_NAME} primary <display>"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero when the display is unknown or the write is refused.")
        .run_plain_text(move |context| {
            let selector = context
                .args()
                .first()
                .context("primary requires a display")?;
            let snapshots = control
                .snapshot()
                .context("failed to read the display layout")?;
            let handle = select_snapshot(&snapshots, Some(selector.as_str()))?.handle;
            let placements = resolve_arrange(&snapshots, &[], Some(handle.id()))?;
            control
                .set_layout(&placements)
                .context("failed to set the primary display")?;
            Ok(PlainTextOutput::empty())
        })
}

fn apply_layout_command(control: Arc<dyn DisplayControl>, config_root: Option<PathBuf>) -> Command {
    Command::new("apply-layout")
        .about("Apply the configured display positions and primary flag.")
        .usage(format!("{BINARY_NAME} apply-layout"))
        .output("Prints the number of applied display positions.")
        .exit_behavior(
            "Exits non-zero when the configured positions are unknown or the write is refused.",
        )
        .run_plain_text(move |_| {
            let (device, _origin) = crate::config::load_with_origin(config_root.as_deref());
            let applied = apply_config_layout(control.as_ref(), &device.layout_position)?;
            Ok(PlainTextOutput::text(apply_layout_text(applied)))
        })
}

fn apply_config_layout(
    control: &dyn DisplayControl,
    layout: &BTreeMap<String, LayoutPosition>,
) -> Result<Option<usize>> {
    if layout.is_empty() {
        return Ok(None);
    }
    let snapshots = control
        .snapshot()
        .context("failed to read the display layout")?;
    let placements = resolve_config_layout(&snapshots, layout)?;
    let applied = placements.len();
    control
        .set_layout(&placements)
        .context("failed to apply the configured display layout")?;
    Ok(Some(applied))
}

fn apply_layout_text(applied: Option<usize>) -> String {
    match applied {
        None => "no configured display positions".to_string(),
        Some(count) => format!(
            "applied {count} display position{}",
            if count == 1 { "" } else { "s" }
        ),
    }
}

fn collect_mode_rows(control: &dyn DisplayControl, selector: Option<&str>) -> Result<Vec<ModeRow>> {
    let snapshots = control
        .snapshot()
        .context("failed to read the display layout")?;
    let snapshots = match selector {
        Some(selector) => vec![select_snapshot(&snapshots, Some(selector))?],
        None => snapshots,
    };
    let mut failures = Vec::new();
    let modes = mode_lists(&snapshots, |handle| {
        control.list_modes(handle).map_err(|error| {
            failures.push(format!("{}: {error}", handle.connector()));
            error
        })
    });
    if !failures.is_empty() {
        bail!("modes are unreadable for {}", failures.join("; "));
    }
    Ok(mode_rows(
        &snapshots,
        &modes,
        mode_writes_supported(control, &snapshots),
    ))
}

fn mode_writes_supported(control: &dyn DisplayControl, snapshots: &[DisplaySnapshot]) -> bool {
    snapshots.iter().all(|snapshot| {
        control
            .probe(&snapshot.handle)
            .map(|capabilities| capabilities.modes)
            .unwrap_or(false)
    })
}

fn mode_line(row: &ModeRow) -> String {
    if row.current {
        format!(
            "{} {}x{}@{} current",
            row.connector, row.width, row.height, row.refresh_hz
        )
    } else {
        format!(
            "{} {}x{}@{}",
            row.connector, row.width, row.height, row.refresh_hz
        )
    }
}

fn layout_line(row: &LayoutRow) -> String {
    let position = format!("{:+}{:+}", row.x, row.y);
    let mode = format!("{}x{}@{}", row.width, row.height, row.refresh_hz);
    if row.primary {
        format!("{} {} {} {} primary", row.connector, row.id, position, mode)
    } else {
        format!("{} {} {} {}", row.connector, row.id, position, mode)
    }
}

fn select_snapshot(
    snapshots: &[DisplaySnapshot],
    selector: Option<&str>,
) -> Result<DisplaySnapshot> {
    let Some(selector) = selector else {
        return snapshots.first().cloned().context("no displays connected");
    };
    Ok(snapshot_for(snapshots, selector)?.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModeRequest {
    width: u32,
    height: u32,
    refresh_hz: Option<u32>,
}

fn parse_mode_spec(raw: &str) -> Result<ModeRequest> {
    let (dimensions, refresh) = match raw.split_once('@') {
        Some((dimensions, refresh)) => (dimensions, Some(refresh)),
        None => (raw, None),
    };
    let (width, height) = dimensions
        .split_once('x')
        .with_context(|| format!("mode `{raw}` must be WIDTHxHEIGHT or WIDTHxHEIGHT@HZ"))?;
    let width: u32 = width
        .parse()
        .with_context(|| format!("mode `{raw}` has a non-numeric width `{width}`"))?;
    let height: u32 = height
        .parse()
        .with_context(|| format!("mode `{raw}` has a non-numeric height `{height}`"))?;
    if width == 0 || height == 0 {
        bail!("mode `{raw}` needs a width and height above zero");
    }
    let refresh_hz = match refresh {
        Some(refresh) => {
            let refresh_hz: u32 = refresh.parse().with_context(|| {
                format!("mode `{raw}` has a non-numeric refresh rate `{refresh}`")
            })?;
            if refresh_hz == 0 {
                bail!("mode `{raw}` needs a refresh rate above zero");
            }
            Some(refresh_hz)
        }
        None => None,
    };
    Ok(ModeRequest {
        width,
        height,
        refresh_hz,
    })
}

fn apply_mode(
    control: &dyn DisplayControl,
    request: ModeRequest,
    selector: Option<&str>,
) -> Result<()> {
    let snapshots = control
        .snapshot()
        .context("failed to read the display layout")?;
    let handle = select_snapshot(&snapshots, selector)?.handle;
    let modes = control
        .list_modes(&handle)
        .with_context(|| format!("failed to list modes for {}", handle.connector()))?;
    let mode = resolve_mode(&modes, request.width, request.height, request.refresh_hz)?;
    control
        .set_mode(&handle, &mode)
        .with_context(|| format!("failed to set mode on {}", handle.connector()))
}

fn parse_arrange_args(args: &[String]) -> Result<(Vec<ArrangeRequest>, Option<String>)> {
    let mut requested = Vec::new();
    let mut primary = None;
    let mut index = 0;
    while index < args.len() {
        let token = args[index].as_str();
        if token == "--primary" {
            let value = args
                .get(index + 1)
                .context("--primary requires a display")?;
            primary = Some(value.clone());
            index += 2;
            continue;
        }
        requested.push(parse_arrange_assignment(token)?);
        index += 1;
    }
    if requested.is_empty() {
        bail!("arrange requires at least one <display>=<x>,<y>");
    }
    Ok((requested, primary))
}

fn parse_arrange_assignment(token: &str) -> Result<ArrangeRequest> {
    let (display, position) = token
        .split_once('=')
        .with_context(|| format!("`{token}` must be <display>=<x>,<y>"))?;
    if display.is_empty() {
        bail!("`{token}` needs a display before `=`");
    }
    let (x, y) = position
        .split_once(',')
        .with_context(|| format!("`{token}` must be <display>=<x>,<y>"))?;
    let x: i32 = x
        .trim()
        .parse()
        .with_context(|| format!("`{token}` has a non-integer x `{x}`"))?;
    let y: i32 = y
        .trim()
        .parse()
        .with_context(|| format!("`{token}` has a non-integer y `{y}`"))?;
    Ok(ArrangeRequest {
        id: display.to_string(),
        x,
        y,
    })
}

fn daemon_command() -> Command {
    Command::new("daemon")
        .about("Run the resident daemon that owns brightness hotkeys and session restore.")
        .usage(format!("{BINARY_NAME} daemon"))
        .output("No stdout; runs until the host sends kill.")
        .exit_behavior("Exits non-zero if the daemon listener cannot start.")
        .run_plain_text(|_| {
            crate::daemon::run().map_err(anyhow::Error::msg)?;
            Ok(PlainTextOutput::empty())
        })
}

fn open_command() -> Command {
    Command::new("open")
        .about("Open the plugin settings.")
        .usage(format!("{BINARY_NAME} open"))
        .detail("Convenience alias for the settings command.")
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if the settings URL cannot be opened.")
        .run_plain_text(|_| {
            open_tray_settings()?;
            Ok(PlainTextOutput::empty())
        })
}

fn open_tray_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
        .context("failed to open settings URL")
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings.")
        .usage(format!("{BINARY_NAME} settings"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if the settings URL cannot be opened.")
        .run_plain_text(|_| {
            open_tray_settings()?;
            Ok(PlainTextOutput::empty())
        })
}

fn grant_command(grant: Arc<dyn GrantBackend>) -> Command {
    Command::new("grant")
        .about("Grant the current user i2c access via the qol uaccess udev rule.")
        .usage(format!("{BINARY_NAME} grant"))
        .output("Prints `i2c uaccess grant active` on success.")
        .exit_behavior(
            "Exits non-zero when the grant is busy, conflicts with an operator rule, or is \
             unsupported.",
        )
        .run_plain_text(move |_| {
            grant.grant().map_err(anyhow::Error::from)?;
            Ok(PlainTextOutput::text("i2c uaccess grant active"))
        })
}

fn revoke_command(grant: Arc<dyn GrantBackend>) -> Command {
    Command::new("revoke")
        .about("Revoke the i2c uaccess grant and restore the rule directory.")
        .usage(format!("{BINARY_NAME} revoke"))
        .output("Prints `i2c uaccess grant revoked` or `no i2c uaccess grant is active`.")
        .exit_behavior(
            "Exits non-zero when the grant is mid-release, the caller is not an owner, or the \
             restore is refused.",
        )
        .run_plain_text(
            move |_| match grant.revoke().map_err(anyhow::Error::from)? {
                RevokeOutcome::Restored => Ok(PlainTextOutput::text("i2c uaccess grant revoked")),
                RevokeOutcome::NothingToRestore => {
                    Ok(PlainTextOutput::text("no i2c uaccess grant is active"))
                }
            },
        )
}

fn doctor_checks(
    control: Arc<dyn DisplayControl>,
    grant: Arc<dyn GrantBackend>,
    config_root: Option<PathBuf>,
) -> Vec<DoctorCheck> {
    let control_for_identity = Arc::clone(&control);
    let control_for_probe = Arc::clone(&control);
    let control_for_modes = Arc::clone(&control);
    let config_root_for_config = config_root.clone();
    let config_root_for_layout = config_root.clone();
    let config_root_for_hotkeys = config_root;
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by the plugin.",
            platform_supported_check,
        ),
        qol_headless::device_permission_check(),
        DoctorCheck::new(
            "i2c_grant",
            "Verify the i2c uaccess grant state.",
            move || Ok(grant_state_result(grant.state())),
        ),
        DoctorCheck::new(
            "display_identity",
            "Verify EDID-derived display identities are stable for config binding.",
            move || {
                let handles = control_for_identity
                    .enumerate()
                    .context("failed to enumerate displays")?;
                Ok(display_identity_result(&handles))
            },
        ),
        DoctorCheck::new(
            "ddc_probe",
            "Probe DDC capability per connected display.",
            move || Ok(ddc_probe_result(control_for_probe.as_ref())),
        ),
        DoctorCheck::new(
            "display_server",
            "Report the session compositor and the gamma fallback runtime note.",
            || Ok(display_server_result(crate::platform::display_server())),
        ),
        DoctorCheck::new(
            "mode_control",
            "Verify per-display resolution and refresh control is available.",
            move || {
                Ok(mode_control_result(
                    crate::platform::current_support(),
                    crate::platform::display_server(),
                    control_for_modes.as_ref(),
                ))
            },
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify the monitor config is readable from the host store.",
            move || Ok(config_readable_result(config_root_for_config.as_deref())),
        ),
        DoctorCheck::new(
            "layout_restore",
            "Verify no display layout snapshot is pending restore.",
            move || Ok(layout_restore_result(config_root_for_layout.as_deref())),
        ),
        DoctorCheck::new(
            "hotkey_bindings",
            "Verify brightness hotkey bindings are registered without chord collisions.",
            move || match config_root_for_hotkeys.as_deref() {
                Some(root) => crate::hotkeys::hotkey_registration_result(root),
                None => Ok(DoctorCheckResult::fail(
                    "hotkey_bindings",
                    "cannot locate the qol config directory",
                )),
            },
        ),
    ]
}

fn mode_control_result(
    support: crate::platform::PlatformSupport,
    server: crate::platform::DisplayServer,
    control: &dyn DisplayControl,
) -> DoctorCheckResult {
    if !support.supported {
        return DoctorCheckResult::fail(
            "mode_control",
            format!("mode control is unsupported on {}.", support.name),
        )
        .with_fix("Run the plugin on Linux or macOS.");
    }
    if support.name == "macos" {
        return DoctorCheckResult::warn(
            "mode_control",
            "mode writing is gated on macOS while arrangement is available",
        )
        .with_fix(
            "Change the resolution in System Settings; use `plugin-monitor arrange` for display \
             positions.",
        );
    }
    if server != crate::platform::DisplayServer::X11 {
        return DoctorCheckResult::warn(
            "mode_control",
            "display configuration needs an X11 session; a Wayland session cannot change display \
             modes",
        )
        .with_fix("Change display modes from an X11 session.");
    }
    let snapshots = match control.snapshot() {
        Ok(snapshots) => snapshots,
        Err(error) => {
            return DoctorCheckResult::fail(
                "mode_control",
                format!("display layout is unreadable: {error}"),
            )
        }
    };
    if snapshots.is_empty() {
        return DoctorCheckResult::warn(
            "mode_control",
            "no connected display reports selectable modes",
        );
    }
    if !snapshots.iter().any(|snapshot| snapshot.primary) {
        return DoctorCheckResult::warn(
            "mode_control",
            "RandR does not report a primary output; mode control may not apply",
        );
    }
    let mut with_modes = 0usize;
    let mut failures = Vec::new();
    for snapshot in &snapshots {
        match control.list_modes(&snapshot.handle) {
            Ok(modes) if !modes.is_empty() => with_modes += 1,
            Ok(_) => {}
            Err(error) => failures.push(format!("{}: {error}", snapshot.handle.connector())),
        }
    }
    if with_modes == 0 {
        let detail = if failures.is_empty() {
            String::new()
        } else {
            format!(" ({})", failures.join("; "))
        };
        return DoctorCheckResult::warn(
            "mode_control",
            format!("no connected display reports selectable modes{detail}"),
        );
    }
    DoctorCheckResult::ok(
        "mode_control",
        format!(
            "mode control ok: {with_modes} of {} displays report selectable modes",
            snapshots.len()
        ),
    )
}

fn layout_restore_result(config_root: Option<&std::path::Path>) -> DoctorCheckResult {
    let Some(root) = config_root else {
        return DoctorCheckResult::fail("layout_restore", "cannot locate the qol config directory");
    };
    let dir = match crate::config::session_dir(root) {
        Ok(dir) => dir,
        Err(error) => {
            return DoctorCheckResult::fail(
                "layout_restore",
                format!("cannot locate the monitor session directory: {error:#}"),
            )
        }
    };
    let store = crate::session::SessionStore::new(dir);
    let snapshot = match store.load_layout() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return DoctorCheckResult::fail(
                "layout_restore",
                format!("display layout snapshot is unreadable: {error:#}"),
            )
        }
    };
    match snapshot {
        Some(snapshot) if snapshot.mutations > 0 => {
            layout_restore_pending(&pending_layout_displays(&snapshot))
        }
        _ => layout_restore_pending(&[]),
    }
}

fn pending_layout_displays(snapshot: &crate::session::LayoutSnapshot) -> Vec<String> {
    snapshot
        .placements
        .iter()
        .map(|record| record.connector.clone())
        .collect()
}

fn layout_restore_pending(pending: &[String]) -> DoctorCheckResult {
    if pending.is_empty() {
        return DoctorCheckResult::ok(
            "layout_restore",
            "no display layout snapshot is pending restore",
        );
    }
    DoctorCheckResult::warn(
        "layout_restore",
        format!(
            "display layout restore is pending for: {}",
            pending.join(", ")
        ),
    )
    .with_fix(
        "Restart the monitor daemon on a portable host to restore the captured layout; a resident \
         host keeps the change and leaves the snapshot in place.",
    )
}

fn config_readable_result(config_root: Option<&std::path::Path>) -> DoctorCheckResult {
    let Some(root) = config_root else {
        return DoctorCheckResult::fail(
            "config_readable",
            "cannot locate the qol config directory",
        );
    };
    let (config, origin) = crate::config::load_with_origin(Some(root));
    DoctorCheckResult::ok(
        "config_readable",
        format!(
            "{}: {} preferred brightness, {} policy selections",
            origin.label(),
            config.preferred_brightness.len(),
            config.policy.len()
        ),
    )
}

fn grant_state_result(state: I2cGrantState) -> DoctorCheckResult {
    match state {
        I2cGrantState::Active { owner } => DoctorCheckResult::ok(
            "i2c_grant",
            format!("i2c uaccess grant is active for {owner}"),
        ),
        I2cGrantState::Preparing => DoctorCheckResult::warn(
            "i2c_grant",
            "i2c uaccess grant is mid-apply; run `plugin-monitor grant` to resume it",
        ),
        I2cGrantState::Releasing => DoctorCheckResult::warn(
            "i2c_grant",
            "i2c uaccess grant is mid-release; run `plugin-monitor revoke` to resume it",
        ),
        I2cGrantState::ReleaseFailed => DoctorCheckResult::fail(
            "i2c_grant",
            "i2c uaccess grant release failed; run `plugin-monitor revoke` to retry",
        ),
        I2cGrantState::Unreadable { message } => DoctorCheckResult::fail(
            "i2c_grant",
            format!("i2c uaccess grant journal is unreadable: {message}"),
        ),
        I2cGrantState::None => DoctorCheckResult::ok(
            "i2c_grant",
            "no i2c uaccess grant is active; run `plugin-monitor grant` to enable DDC access",
        ),
        I2cGrantState::Unsupported => {
            DoctorCheckResult::ok("i2c_grant", "skipped: i2c uaccess grants require Linux")
        }
    }
}

fn display_identity_result(handles: &[DisplayHandle]) -> DoctorCheckResult {
    if handles.is_empty() {
        return DoctorCheckResult::ok("display_identity", "no displays connected");
    }
    let unstable = handles
        .iter()
        .filter(|handle| handle.identity_unstable())
        .collect::<Vec<_>>();
    if unstable.is_empty() {
        return DoctorCheckResult::ok(
            "display_identity",
            "every connected display has a stable EDID identity",
        );
    }
    let connectors = unstable
        .iter()
        .map(|handle| handle.connector())
        .collect::<Vec<_>>()
        .join(", ");
    DoctorCheckResult::warn(
        "display_identity",
        format!("config binding refused for displays with unstable identity: {connectors}"),
    )
}

fn platform_supported_check() -> Result<DoctorCheckResult> {
    Ok(platform_supported_result(crate::platform::current_support()))
}

fn ddc_probe_result(control: &dyn DisplayControl) -> DoctorCheckResult {
    let handles = match control.enumerate() {
        Ok(handles) => handles,
        Err(error) => {
            return DoctorCheckResult::fail(
                "ddc_probe",
                format!("display enumeration failed: {error}"),
            );
        }
    };
    if handles.is_empty() {
        return DoctorCheckResult::ok("ddc_probe", "no displays connected");
    }
    let mut ddc_capable = 0usize;
    let mut failures = Vec::new();
    let mut entries = Vec::new();
    for handle in &handles {
        match control.probe(handle) {
            Ok(capabilities) => {
                if capabilities.brightness_ddc {
                    ddc_capable += 1;
                }
                entries.push(serde_json::json!({
                    "connector": handle.connector(),
                    "id": handle.id(),
                    "status": "ok",
                    "brightness_ddc": capabilities.brightness_ddc,
                    "brightness_gamma": capabilities.brightness_gamma,
                }));
            }
            Err(error) => {
                let taxonomy = ddc_probe_taxonomy(&error);
                failures.push((handle.connector().to_string(), taxonomy.clone()));
                entries.push(serde_json::json!({
                    "connector": handle.connector(),
                    "id": handle.id(),
                    "status": "error",
                    "error": taxonomy,
                }));
            }
        }
    }
    let details = serde_json::json!({ "displays": entries });
    if failures.is_empty() {
        return DoctorCheckResult::ok(
            "ddc_probe",
            format!(
                "DDC probe ok: {} of {} displays expose DDC brightness",
                ddc_capable,
                handles.len()
            ),
        )
        .with_details(details);
    }
    let connectors = failures
        .iter()
        .map(|(connector, _)| connector.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let result = DoctorCheckResult::fail(
        "ddc_probe",
        format!(
            "DDC probe failed on {} of {} displays: {connectors}",
            failures.len(),
            handles.len()
        ),
    )
    .with_details(details);
    if failures
        .iter()
        .any(|(_, taxonomy)| taxonomy.starts_with("permission"))
    {
        return result.with_fix(
            "Run `plugin-monitor grant` to apply the i2c uaccess rule, then retry doctor.",
        );
    }
    result
}

fn ddc_probe_taxonomy(error: &MonitorError) -> String {
    match error {
        MonitorError::Unsupported { reason, .. } => format!("unsupported: {reason}"),
        MonitorError::Refused { reason, .. } => format!("refused: {reason}"),
        MonitorError::I2c(crate::monitor::I2cError::Permission { node }) => {
            format!("permission: no i2c access to {node}")
        }
        MonitorError::I2c(crate::monitor::I2cError::NoDevice { node }) => {
            format!(
                "no-device: nothing at {node}; the connector may be unplugged or i2c-dev unloaded"
            )
        }
        MonitorError::I2c(crate::monitor::I2cError::Busy { node }) => {
            format!("busy: {node} is held by another driver")
        }
        MonitorError::I2c(crate::monitor::I2cError::UnsupportedTransport { detail }) => {
            format!("unsupported-transport: {detail}")
        }
        MonitorError::I2c(crate::monitor::I2cError::Protocol { detail }) => {
            format!("protocol: {detail}")
        }
        MonitorError::I2c(crate::monitor::I2cError::Io(error)) => format!("io: {error}"),
        MonitorError::Display(error) => format!("enumeration: {error}"),
        MonitorError::DisplayNotFound(selector) => format!("not-found: {selector}"),
    }
}

fn display_server_result(server: crate::platform::DisplayServer) -> DoctorCheckResult {
    match server {
        crate::platform::DisplayServer::X11 => DoctorCheckResult::ok(
            "display_server",
            "X11 session; gamma fallback runs through RandR with write-plus-read-back verification",
        ),
        crate::platform::DisplayServer::Wayland => DoctorCheckResult::warn(
            "display_server",
            "Wayland session; the gamma fallback is X11-RandR-only and is typed unsupported here, \
             never assumed from protocol presence. DDC brightness is unaffected.",
        )
        .with_fix("Use DDC brightness, or run an X11 session for the gamma fallback."),
        crate::platform::DisplayServer::None => DoctorCheckResult::warn(
            "display_server",
            "no X11 or Wayland display server detected in the terminal environment; the gamma \
             fallback is unavailable",
        ),
    }
}

fn platform_supported_result(support: crate::platform::PlatformSupport) -> DoctorCheckResult {
    if support.supported {
        return DoctorCheckResult::ok(
            "platform_supported",
            format!("{} is supported.", support.name),
        );
    }
    DoctorCheckResult::fail(
        "platform_supported",
        format!("{} is not declared by this plugin.", support.name),
    )
    .with_fix("Run the plugin on Linux or macOS.")
}

fn brightness_for(
    control: &dyn DisplayControl,
    context: &CommandContext,
) -> Result<BrightnessState> {
    let handle = select_display(control, context.args().first().map(String::as_str))?;
    control
        .get_brightness(&handle)
        .with_context(|| format!("failed to read brightness on {}", handle.connector()))
}

fn step_brightness(
    control: &dyn DisplayControl,
    context: &CommandContext,
    direction: i8,
) -> Result<()> {
    let handle = select_display(control, context.args().first().map(String::as_str))?;
    let current = control
        .get_brightness(&handle)
        .with_context(|| format!("failed to read brightness on {}", handle.connector()))?;
    let stepped = i16::from(current.value) + i16::from(direction) * i16::from(BRIGHTNESS_STEP);
    let next = stepped.clamp(i16::from(BRIGHTNESS_MIN), i16::from(BRIGHTNESS_MAX)) as u8;
    if next == current.value {
        return Ok(());
    }
    control
        .set_brightness(&handle, next)
        .with_context(|| format!("failed to set brightness on {}", handle.connector()))
}

fn parse_brightness_value(context: &CommandContext) -> Result<u8> {
    let raw = context
        .args()
        .first()
        .context("set requires a brightness value")?;
    let value = raw
        .parse::<u8>()
        .with_context(|| format!("brightness value must be an integer, got `{raw}`"))?;
    if !(BRIGHTNESS_MIN..=BRIGHTNESS_MAX).contains(&value) {
        anyhow::bail!(
            "brightness value must be between {BRIGHTNESS_MIN} and {BRIGHTNESS_MAX}, got `{raw}`"
        );
    }
    Ok(value)
}

fn select_display(control: &dyn DisplayControl, selector: Option<&str>) -> Result<DisplayHandle> {
    let handles = control
        .enumerate()
        .context("failed to enumerate displays")?;
    let Some(selector) = selector else {
        return handles.into_iter().next().context("no displays connected");
    };
    handles
        .into_iter()
        .find(|handle| handle.connector() == selector || handle.id().starts_with(selector))
        .ok_or_else(|| anyhow::anyhow!(MonitorError::DisplayNotFound(selector.to_string())))
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use qol_headless::{DoctorReport, DoctorStatus, EXIT_RUNTIME_ERROR, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;
    use crate::monitor::{
        BrightnessSource, DisplayCapabilities, DisplayMode, DisplayPlacement, GammaState,
        GrantError, HdrState,
    };

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn display_mode(token: u64, width: u32, height: u32, refresh_hz: u32) -> DisplayMode {
        DisplayMode {
            token,
            width,
            height,
            refresh_hz,
        }
    }

    fn display_snapshot(
        id: &str,
        connector: &str,
        bounds: (f32, f32, f32, f32),
        primary: bool,
        mode: Option<(u64, u32, u32, u32)>,
    ) -> DisplaySnapshot {
        let (x, y, width, height) = bounds;
        DisplaySnapshot {
            handle: DisplayHandle::new(id.into(), connector.into(), None, false),
            bounds: qol_windowing::MonitorBounds {
                x,
                y,
                width,
                height,
            },
            primary,
            mode: mode.map(|(token, width, height, refresh_hz)| {
                display_mode(token, width, height, refresh_hz)
            }),
        }
    }

    #[derive(Clone)]
    struct FakeControl {
        displays: Vec<DisplayHandle>,
        brightness: Arc<Mutex<BrightnessState>>,
        probe_failure: Option<ProbeFailure>,
        snapshots: Vec<DisplaySnapshot>,
        modes: BTreeMap<String, Vec<DisplayMode>>,
        mode_writes: Arc<Mutex<Vec<(String, DisplayMode)>>>,
        layout_writes: Arc<Mutex<Vec<Vec<DisplayPlacement>>>>,
        snapshot_failure: Option<&'static str>,
        modes_failure: Option<&'static str>,
        mode_write_failure: Option<&'static str>,
        layout_write_failure: Option<&'static str>,
    }

    #[derive(Clone, Copy)]
    enum ProbeFailure {
        Permission,
    }

    impl ProbeFailure {
        fn monitor_error(self) -> MonitorError {
            MonitorError::I2c(crate::monitor::I2cError::Permission {
                node: "/dev/i2c-7".into(),
            })
        }
    }

    impl FakeControl {
        fn new() -> Self {
            Self {
                displays: vec![
                    DisplayHandle::new("id-alpha".into(), "card0-DP-1".into(), None, false),
                    DisplayHandle::new("id-beta".into(), "card1-HDMI-A-1".into(), None, false),
                ],
                brightness: Arc::new(Mutex::new(BrightnessState {
                    value: 42,
                    source: BrightnessSource::Ddc,
                })),
                probe_failure: None,
                snapshots: vec![
                    display_snapshot(
                        "id-alpha",
                        "card0-DP-1",
                        (0.0, 0.0, 1920.0, 1080.0),
                        true,
                        Some((1, 1920, 1080, 60)),
                    ),
                    display_snapshot(
                        "id-beta",
                        "card1-HDMI-A-1",
                        (1920.0, 0.0, 2560.0, 1440.0),
                        false,
                        Some((2, 2560, 1440, 144)),
                    ),
                ],
                modes: BTreeMap::from([
                    (
                        "id-alpha".to_string(),
                        vec![
                            display_mode(1, 1920, 1080, 60),
                            display_mode(3, 1920, 1080, 50),
                            display_mode(4, 1280, 720, 60),
                        ],
                    ),
                    (
                        "id-beta".to_string(),
                        vec![display_mode(2, 2560, 1440, 144)],
                    ),
                ]),
                mode_writes: Arc::new(Mutex::new(Vec::new())),
                layout_writes: Arc::new(Mutex::new(Vec::new())),
                snapshot_failure: None,
                modes_failure: None,
                mode_write_failure: None,
                layout_write_failure: None,
            }
        }

        fn with_probe_failure(failure: ProbeFailure) -> Self {
            Self {
                probe_failure: Some(failure),
                ..Self::new()
            }
        }
    }

    impl DisplayControl for FakeControl {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(self.displays.clone())
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            match self.probe_failure {
                Some(failure) => Err(failure.monitor_error()),
                None => Ok(DisplayCapabilities {
                    brightness_ddc: true,
                    ..DisplayCapabilities::none()
                }),
            }
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(*self.brightness.lock().unwrap())
        }

        fn set_brightness(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            self.brightness.lock().unwrap().value = value;
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn list_modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            match self.modes_failure {
                Some(reason) => Err(MonitorError::refused("modes", reason)),
                None => Ok(self.modes.get(handle.id()).cloned().unwrap_or_default()),
            }
        }

        fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), MonitorError> {
            if let Some(reason) = self.mode_write_failure {
                return Err(MonitorError::refused("modes", reason));
            }
            self.mode_writes
                .lock()
                .unwrap()
                .push((handle.id().to_string(), mode.clone()));
            Ok(())
        }

        fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, MonitorError> {
            match self.snapshot_failure {
                Some(reason) => Err(MonitorError::refused("layout", reason)),
                None => Ok(self.snapshots.clone()),
            }
        }

        fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), MonitorError> {
            if let Some(reason) = self.layout_write_failure {
                return Err(MonitorError::refused("layout", reason));
            }
            self.layout_writes.lock().unwrap().push(placements.to_vec());
            Ok(())
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }
    }

    #[derive(Clone)]
    struct FakeGrantBackend {
        state: I2cGrantState,
        grant_error: Option<GrantError>,
        revoke_error: Option<GrantError>,
    }

    impl FakeGrantBackend {
        fn new() -> Self {
            Self {
                state: I2cGrantState::None,
                grant_error: None,
                revoke_error: None,
            }
        }

        fn with_state(state: I2cGrantState) -> Self {
            Self {
                state,
                grant_error: None,
                revoke_error: None,
            }
        }

        fn with_grant_error(error: GrantError) -> Self {
            Self {
                state: I2cGrantState::None,
                grant_error: Some(error),
                revoke_error: None,
            }
        }
    }

    impl GrantBackend for FakeGrantBackend {
        fn grant(&self) -> Result<(), GrantError> {
            match &self.grant_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn revoke(&self) -> Result<RevokeOutcome, GrantError> {
            if let Some(error) = &self.revoke_error {
                return Err(error.clone());
            }
            match self.state {
                I2cGrantState::Active { .. }
                | I2cGrantState::Preparing
                | I2cGrantState::Releasing
                | I2cGrantState::ReleaseFailed => Ok(RevokeOutcome::Restored),
                _ => Ok(RevokeOutcome::NothingToRestore),
            }
        }

        fn state(&self) -> I2cGrantState {
            self.state.clone()
        }
    }

    fn fake_app() -> HeadlessApp {
        app_with(FakeControl::new(), FakeGrantBackend::new())
    }

    fn fake_app_with(control: FakeControl, grant: FakeGrantBackend) -> HeadlessApp {
        app_with(control, grant)
    }

    #[test]
    fn list_prints_every_display_connector_and_id() {
        let execution = fake_app().execute(["list".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(
            execution.stdout,
            "card0-DP-1 id-alpha\ncard1-HDMI-A-1 id-beta\n"
        );
    }

    #[test]
    fn bare_invocation_lists_displays() {
        let execution = fake_app().execute(Vec::new());
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(
            execution.stdout,
            "card0-DP-1 id-alpha\ncard1-HDMI-A-1 id-beta\n"
        );
    }

    #[test]
    fn status_reports_probe_capabilities_per_display() {
        let execution = fake_app().execute(["status".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution.stdout.contains("card0-DP-1\n  id: id-alpha"));
        assert!(execution.stdout.contains("brightness_ddc: yes"));
        assert!(execution.stdout.contains("brightness_gamma: no"));
    }

    #[test]
    fn get_prints_value_and_source_for_the_first_display() {
        let execution = fake_app().execute(["get".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "brightness=42 source=ddc\n");
    }

    #[test]
    fn get_selects_display_by_connector_or_id_prefix() {
        let by_connector = fake_app().execute(["get".to_string(), "card1-HDMI-A-1".to_string()]);
        let by_id = fake_app().execute(["get".to_string(), "id-beta".to_string()]);
        assert_eq!(by_connector.exit_code, EXIT_SUCCESS);
        assert_eq!(by_connector.stdout, by_id.stdout);
    }

    #[test]
    fn get_unknown_display_is_a_runtime_error() {
        let execution = fake_app().execute(["get".to_string(), "card9-VGA-9".to_string()]);
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution
            .stderr
            .contains("no display matches `card9-VGA-9`"));
    }

    #[test]
    fn set_parses_and_forwards_the_value() {
        let app = fake_app();
        let execution = app.execute(["set".to_string(), "77".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "");
        let get = app.execute(["get".to_string()]);
        assert_eq!(get.stdout, "brightness=77 source=ddc\n");
    }

    #[test]
    fn set_rejects_non_numeric_and_out_of_range_values() {
        for args in [
            vec!["set".to_string()],
            vec!["set".to_string(), "abc".to_string()],
            vec!["set".to_string(), "101".to_string()],
        ] {
            let execution = fake_app().execute(args.clone());
            assert_eq!(execution.exit_code, 1, "args: {args:?}");
            assert!(execution.stdout.is_empty());
            assert!(!execution.stderr.is_empty(), "args: {args:?}");
        }
    }

    #[test]
    fn up_and_down_step_by_the_contract_step_and_clamp() {
        let app = fake_app();
        app.execute(["up".to_string()]);
        assert_eq!(
            app.execute(["get".to_string()]).stdout,
            "brightness=47 source=ddc\n"
        );
        app.execute(["down".to_string()]);
        app.execute(["down".to_string()]);
        assert_eq!(
            app.execute(["get".to_string()]).stdout,
            "brightness=37 source=ddc\n"
        );
        app.execute(["set".to_string(), "99".to_string()]);
        app.execute(["up".to_string()]);
        assert_eq!(
            app.execute(["get".to_string()]).stdout,
            "brightness=100 source=ddc\n"
        );
        app.execute(["set".to_string(), "2".to_string()]);
        app.execute(["down".to_string()]);
        assert_eq!(
            app.execute(["get".to_string()]).stdout,
            "brightness=0 source=ddc\n"
        );
    }

    #[test]
    fn help_first_and_final_are_equivalent_for_commands() {
        for command in [
            "list",
            "get",
            "set",
            "up",
            "doctor",
            "modes",
            "set-mode",
            "layout",
            "arrange",
            "primary",
            "apply-layout",
        ] {
            let first = fake_app().execute(["help".to_string(), command.to_string()]);
            let final_token = fake_app().execute([command.to_string(), "help".to_string()]);
            assert_eq!(first.exit_code, EXIT_SUCCESS, "command: {command}");
            assert_eq!(first.stdout, final_token.stdout, "command: {command}");
        }
    }

    #[test]
    fn doctor_json_registers_probe_grant_and_identity_checks() {
        let before = fake_app().execute(["--json".to_string(), "doctor".to_string()]);
        let after = fake_app().execute(["doctor".to_string(), "--json".to_string()]);
        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 10);
        let ids = report
            .checks
            .iter()
            .map(|check| check.id.as_str())
            .collect::<Vec<_>>();
        for expected in [
            "platform_supported",
            "device_permissions",
            "i2c_grant",
            "display_identity",
            "ddc_probe",
            "display_server",
            "mode_control",
            "config_readable",
            "layout_restore",
            "hotkey_bindings",
        ] {
            assert!(ids.contains(&expected), "missing check: {expected}");
        }
        assert!(report
            .checks
            .iter()
            .all(|check| !check.id.is_empty() && !check.message.is_empty()));
        assert_eq!(
            report
                .checks
                .iter()
                .find(|check| check.id == "config_readable")
                .expect("config check must exist")
                .status,
            DoctorStatus::Fail,
            "without a config root the check must say it cannot locate the config directory"
        );
    }

    #[test]
    fn doctor_with_a_config_root_reads_the_device_scope() {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config").join("qol-tray");
        let app = app_with_config_root(
            Arc::new(FakeControl::new()),
            Arc::new(FakeGrantBackend::new()),
            Some(config_root.clone()),
        );
        let report: DoctorReport = serde_json::from_str(
            &app.execute(["--json".to_string(), "doctor".to_string()])
                .stdout,
        )
        .expect("doctor output must be valid JSON");
        let config = report
            .checks
            .iter()
            .find(|check| check.id == "config_readable")
            .expect("config check must exist");
        assert_eq!(config.status, DoctorStatus::Ok);
        assert!(config.message.contains("preferred brightness"));

        let hotkeys = crate::config::hotkeys_path(&config_root).unwrap();
        std::fs::create_dir_all(hotkeys.parent().unwrap()).unwrap();
        std::fs::write(
            &hotkeys,
            serde_json::json!({
                "hotkeys": [
                    {"id": "h1", "key": "ctrl+shift+b", "plugin_uid": "plugin-monitor", "action": "brightness-up", "enabled": true},
                    {"id": "h2", "key": "ctrl+shift+b", "plugin_uid": "plugin-monitor", "action": "brightness-down", "enabled": true}
                ]
            })
            .to_string(),
        )
        .unwrap();
        let report: DoctorReport = serde_json::from_str(
            &app.execute(["--json".to_string(), "doctor".to_string()])
                .stdout,
        )
        .expect("doctor output must be valid JSON");
        let hotkeys_check = report
            .checks
            .iter()
            .find(|check| check.id == "hotkey_bindings")
            .expect("hotkey check must exist");
        assert_eq!(
            hotkeys_check.status,
            DoctorStatus::Fail,
            "a duplicated chord is a doctor failure"
        );
        assert!(hotkeys_check.message.contains("ctrl+shift+b"));
    }

    #[test]
    fn doctor_without_a_config_root_fails_hotkey_and_config_checks() {
        let app = app_with_config_root(
            Arc::new(FakeControl::new()),
            Arc::new(FakeGrantBackend::new()),
            None,
        );
        let report: DoctorReport = serde_json::from_str(
            &app.execute(["--json".to_string(), "doctor".to_string()])
                .stdout,
        )
        .expect("doctor output must be valid JSON");
        for id in ["config_readable", "hotkey_bindings"] {
            let check = report
                .checks
                .iter()
                .find(|check| check.id == id)
                .expect("check must exist");
            assert_eq!(check.status, DoctorStatus::Fail, "check: {id}");
        }
    }

    #[test]
    fn doctor_help_names_all_checks() {
        let first = fake_app().execute(["help".to_string(), "doctor".to_string()]);
        let final_token = fake_app().execute(["doctor".to_string(), "help".to_string()]);
        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Run read-only health checks."));
        for id in [
            "platform_supported",
            "device_permissions",
            "i2c_grant",
            "display_identity",
            "ddc_probe",
            "display_server",
            "mode_control",
            "config_readable",
            "layout_restore",
            "hotkey_bindings",
        ] {
            assert!(first.stdout.contains(id), "help must name {id}");
        }
    }

    #[test]
    fn daemon_command_is_registered_and_helpful() {
        let execution = fake_app().execute(["help".to_string(), "daemon".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution.stdout.contains("resident daemon"));
        assert!(execution.stdout.contains("hotkeys"));
    }

    #[test]
    fn open_command_is_registered_and_points_at_settings() {
        let execution = fake_app().execute(["help".to_string(), "open".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert!(execution.stdout.contains("settings"));
    }

    #[test]
    fn unsupported_json_is_rejected_before_settings_runs() {
        let execution = fake_app().execute(["settings".to_string(), "--json".to_string()]);
        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stderr.contains("does not support --json"));
    }

    #[test]
    fn platform_support_results_match_the_manifest_contract() {
        let cases = [
            ("linux", true, DoctorStatus::Ok, None),
            ("macos", true, DoctorStatus::Ok, None),
            (
                "windows",
                false,
                DoctorStatus::Fail,
                Some("Run the plugin on Linux or macOS."),
            ),
            (
                "other",
                false,
                DoctorStatus::Fail,
                Some("Run the plugin on Linux or macOS."),
            ),
        ];

        for (name, supported, status, fix) in cases {
            let result =
                platform_supported_result(crate::platform::PlatformSupport { name, supported });
            assert_eq!(result.status, status, "platform: {name}");
            assert_eq!(
                result.message,
                if supported {
                    format!("{name} is supported.")
                } else {
                    format!("{name} is not declared by this plugin.")
                },
                "platform: {name}"
            );
            assert_eq!(result.fix.as_deref(), fix, "platform: {name}");
        }
    }

    #[test]
    fn grant_reports_active_on_success() {
        let app = fake_app_with(FakeControl::new(), FakeGrantBackend::new());
        let execution = app.execute(["grant".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "i2c uaccess grant active\n");
    }

    #[test]
    fn grant_surfaces_busy_and_rule_conflict_errors() {
        let busy = fake_app_with(
            FakeControl::new(),
            FakeGrantBackend::with_grant_error(GrantError::Busy {
                detail: "the uaccess grant is already active; revoke it first".into(),
            }),
        );
        let execution = busy.execute(["grant".to_string()]);
        assert_eq!(execution.exit_code, EXIT_RUNTIME_ERROR);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("busy"), "{}", execution.stderr);
        assert!(execution.stderr.contains("revoke it first"));

        let conflict = fake_app_with(
            FakeControl::new(),
            FakeGrantBackend::with_grant_error(GrantError::RuleConflict {
                path: "/etc/udev/rules.d/90-qol-i2c-uaccess.rules".into(),
                expected_sha256: "a".repeat(64),
                actual_sha256: "b".repeat(64),
            }),
        );
        let execution = conflict.execute(["grant".to_string()]);
        assert_eq!(execution.exit_code, EXIT_RUNTIME_ERROR);
        assert!(
            execution.stderr.contains("expected sha256"),
            "{}",
            execution.stderr
        );
        assert!(
            execution.stderr.contains("actual sha256"),
            "{}",
            execution.stderr
        );
        assert!(execution.stderr.contains("90-qol-i2c-uaccess.rules"));
    }

    #[test]
    fn grant_unsupported_is_a_runtime_error() {
        let app = fake_app_with(
            FakeControl::new(),
            FakeGrantBackend::with_grant_error(GrantError::unsupported(
                "i2c uaccess grants require Linux",
            )),
        );
        let execution = app.execute(["grant".to_string()]);
        assert_eq!(execution.exit_code, EXIT_RUNTIME_ERROR);
        assert!(execution.stderr.contains("require Linux"));
    }

    #[test]
    fn revoke_reports_restored_and_nothing_to_restore() {
        let granted = fake_app_with(
            FakeControl::new(),
            FakeGrantBackend::with_state(I2cGrantState::Active {
                owner: "plugin-monitor".into(),
            }),
        );
        let execution = granted.execute(["revoke".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "i2c uaccess grant revoked\n");

        let empty = fake_app_with(FakeControl::new(), FakeGrantBackend::new());
        let execution = empty.execute(["revoke".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "no i2c uaccess grant is active\n");
    }

    #[test]
    fn doctor_grant_state_maps_every_state() {
        let cases = [
            (
                I2cGrantState::Active {
                    owner: "plugin-monitor".into(),
                },
                DoctorStatus::Ok,
                "active for plugin-monitor",
            ),
            (I2cGrantState::Preparing, DoctorStatus::Warn, "mid-apply"),
            (I2cGrantState::Releasing, DoctorStatus::Warn, "mid-release"),
            (
                I2cGrantState::ReleaseFailed,
                DoctorStatus::Fail,
                "release failed",
            ),
            (
                I2cGrantState::Unreadable {
                    message: "tampered".into(),
                },
                DoctorStatus::Fail,
                "tampered",
            ),
            (
                I2cGrantState::None,
                DoctorStatus::Ok,
                "no i2c uaccess grant",
            ),
            (
                I2cGrantState::Unsupported,
                DoctorStatus::Ok,
                "require Linux",
            ),
        ];
        for (state, status, needle) in cases {
            let result = grant_state_result(state);
            assert_eq!(result.id, "i2c_grant");
            assert_eq!(result.status, status, "state: {needle}");
            assert!(result.message.contains(needle), "{}", result.message);
        }
    }

    #[test]
    fn doctor_identity_warns_only_for_unstable_displays() {
        let stable = display_identity_result(&[
            DisplayHandle::new("id-a".into(), "card0-DP-1".into(), Some([1; 32]), false),
            DisplayHandle::new("id-b".into(), "card0-HDMI-A-1".into(), Some([2; 32]), false),
        ]);
        assert_eq!(stable.status, DoctorStatus::Ok);
        assert!(stable.message.contains("stable"));

        let unstable = display_identity_result(&[
            DisplayHandle::new("id-a".into(), "card0-DP-1".into(), Some([1; 32]), false),
            DisplayHandle::new("id-c".into(), "card1-DP-2".into(), None, true),
        ]);
        assert_eq!(unstable.status, DoctorStatus::Warn);
        assert!(
            unstable.message.contains("config binding refused"),
            "{}",
            unstable.message
        );
        assert!(unstable.message.contains("card1-DP-2"));

        let none = display_identity_result(&[]);
        assert_eq!(none.status, DoctorStatus::Ok);
        assert!(none.message.contains("no displays"));
    }

    #[test]
    fn doctor_identity_check_enumerates_through_the_control() {
        let app = fake_app();
        let execution = app.execute(["--json".to_string(), "doctor".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let report: DoctorReport =
            serde_json::from_str(&execution.stdout).expect("doctor output must be valid JSON");
        let identity = report
            .checks
            .iter()
            .find(|check| check.id == "display_identity")
            .expect("identity check must exist");
        assert_eq!(identity.status, DoctorStatus::Ok);
        assert!(identity.message.contains("stable EDID identity"));
    }

    #[test]
    fn ddc_probe_reports_every_display_with_capabilities() {
        let app = fake_app();
        let execution = app.execute(["--json".to_string(), "doctor".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let report: DoctorReport =
            serde_json::from_str(&execution.stdout).expect("doctor output must be valid JSON");
        let probe = report
            .checks
            .iter()
            .find(|check| check.id == "ddc_probe")
            .expect("ddc_probe check must exist");
        assert_eq!(probe.status, DoctorStatus::Ok);
        assert!(
            probe.message.contains("2 of 2 displays"),
            "{}",
            probe.message
        );
        let details = probe.details.as_ref().expect("details must be present");
        let displays = details["displays"].as_array().expect("displays list");
        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0]["connector"], "card0-DP-1");
        assert_eq!(displays[0]["status"], "ok");
        assert_eq!(displays[0]["brightness_ddc"], true);
    }

    #[test]
    fn ddc_probe_surfaces_the_failure_taxonomy_per_display() {
        let app = fake_app_with(
            FakeControl::with_probe_failure(ProbeFailure::Permission),
            FakeGrantBackend::new(),
        );
        let execution = app.execute(["--json".to_string(), "doctor".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let report: DoctorReport =
            serde_json::from_str(&execution.stdout).expect("doctor output must be valid JSON");
        let probe = report
            .checks
            .iter()
            .find(|check| check.id == "ddc_probe")
            .expect("ddc_probe check must exist");
        assert_eq!(probe.status, DoctorStatus::Fail);
        assert!(
            probe.message.contains("2 of 2 displays"),
            "{}",
            probe.message
        );
        assert!(probe.message.contains("card0-DP-1"));
        assert!(
            probe
                .fix
                .as_deref()
                .unwrap_or_default()
                .contains("plugin-monitor grant"),
            "permission failures must suggest the grant: {:?}",
            probe.fix
        );
        let details = probe.details.as_ref().expect("details must be present");
        let displays = details["displays"].as_array().expect("displays list");
        assert_eq!(displays.len(), 2);
        for display in displays {
            assert_eq!(display["status"], "error");
            assert!(
                display["error"]
                    .as_str()
                    .expect("error taxonomy")
                    .starts_with("permission: no i2c access to /dev/i2c-7"),
                "{}",
                display["error"]
            );
        }
    }

    #[test]
    fn ddc_probe_taxonomy_maps_busy_and_unsupported_errors() {
        assert!(
            ddc_probe_taxonomy(&MonitorError::I2c(crate::monitor::I2cError::Busy {
                node: "/dev/i2c-7".into()
            }))
            .starts_with("busy")
        );
        assert!(
            ddc_probe_taxonomy(&MonitorError::unsupported("brightness", "x"))
                .starts_with("unsupported")
        );
        assert!(
            ddc_probe_taxonomy(&MonitorError::refused("brightness", "y")).starts_with("refused")
        );
    }

    #[test]
    fn ddc_probe_with_no_displays_is_ok() {
        let app = fake_app_with(
            FakeControl {
                displays: vec![],
                ..FakeControl::new()
            },
            FakeGrantBackend::new(),
        );
        let execution = app.execute(["--json".to_string(), "doctor".to_string()]);
        let report: DoctorReport =
            serde_json::from_str(&execution.stdout).expect("doctor output must be valid JSON");
        let probe = report
            .checks
            .iter()
            .find(|check| check.id == "ddc_probe")
            .expect("ddc_probe check must exist");
        assert_eq!(probe.status, DoctorStatus::Ok);
        assert!(probe.message.contains("no displays connected"));
    }

    #[test]
    fn display_server_result_maps_every_session_kind() {
        let x11 = display_server_result(crate::platform::DisplayServer::X11);
        assert_eq!(x11.status, DoctorStatus::Ok);
        assert!(x11.message.contains("RandR"));
        assert!(x11.message.contains("write-plus-read-back"));

        let wayland = display_server_result(crate::platform::DisplayServer::Wayland);
        assert_eq!(wayland.status, DoctorStatus::Warn);
        assert!(wayland.message.contains("X11-RandR-only"));
        assert!(
            wayland
                .message
                .contains("never assumed from protocol presence"),
            "{}",
            wayland.message
        );
        assert!(wayland.fix.is_some());

        let none = display_server_result(crate::platform::DisplayServer::None);
        assert_eq!(none.status, DoctorStatus::Warn);
        assert!(none.message.contains("unavailable"));
    }

    #[test]
    fn modes_lists_every_mode_and_marks_the_current_one() {
        let execution = fake_app().execute(["modes".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let lines: Vec<&str> = execution.stdout.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines.contains(&"card0-DP-1 1920x1080@60 current"));
        assert!(lines.contains(&"card0-DP-1 1920x1080@50"));
        assert!(lines.contains(&"card0-DP-1 1280x720@60"));
        assert!(lines.contains(&"card1-HDMI-A-1 2560x1440@144 current"));
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.ends_with(" current"))
                .count(),
            2,
            "the current mode is marked on every display"
        );
    }

    #[test]
    fn modes_json_returns_rows_and_filters_by_display() {
        let execution = fake_app().execute(["--json".to_string(), "modes".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let value: serde_json::Value = serde_json::from_str(&execution.stdout).unwrap();
        assert!(value.is_array(), "modes --json must emit a bare array");
        let rows = value.as_array().expect("rows must be a list");
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .filter(|row| row.get("current").and_then(serde_json::Value::as_bool) == Some(true))
                .count(),
            2
        );
        assert!(rows.iter().any(|row| {
            row.get("connector").and_then(serde_json::Value::as_str) == Some("card0-DP-1")
                && row.get("width").and_then(serde_json::Value::as_u64) == Some(1920)
                && row.get("height").and_then(serde_json::Value::as_u64) == Some(1080)
                && row.get("refresh_hz").and_then(serde_json::Value::as_u64) == Some(60)
                && row.get("current").and_then(serde_json::Value::as_bool) == Some(true)
        }));
        assert!(rows.iter().all(|row| {
            row.get("selectable")
                .is_some_and(serde_json::Value::is_boolean)
                && row.get("label").is_some_and(serde_json::Value::is_string)
        }));

        let filtered = fake_app().execute([
            "--json".to_string(),
            "modes".to_string(),
            "id-alpha".to_string(),
        ]);
        assert_eq!(filtered.exit_code, EXIT_SUCCESS);
        let filtered: serde_json::Value = serde_json::from_str(&filtered.stdout).unwrap();
        let rows = filtered.as_array().expect("rows must be a list");
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| {
            row.get("connector").and_then(serde_json::Value::as_str) == Some("card0-DP-1")
        }));
    }

    #[test]
    fn modes_unknown_display_is_a_runtime_error() {
        let execution = fake_app().execute(["modes".to_string(), "card9-VGA-9".to_string()]);
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution
            .stderr
            .contains("no display matches `card9-VGA-9`"));
    }

    #[test]
    fn set_mode_parses_the_spec_and_writes_the_matching_token() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.mode_writes);
        let app = fake_app_with(control, FakeGrantBackend::new());
        let execution = app.execute(args(&["set-mode", "1920x1080@50"]));
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "");
        {
            let writes = writes.lock().unwrap();
            assert_eq!(writes.len(), 1);
            assert_eq!(writes[0].0, "id-alpha");
            assert_eq!(writes[0].1, display_mode(3, 1920, 1080, 50));
        }

        let selected = app.execute(args(&["set-mode", "2560x1440", "card1-HDMI-A-1"]));
        assert_eq!(selected.exit_code, EXIT_SUCCESS);
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[1].0, "id-beta");
        assert_eq!(writes[1].1, display_mode(2, 2560, 1440, 144));
    }

    #[test]
    fn set_mode_refuses_an_ambiguous_resolution_and_lists_candidates() {
        let execution = fake_app().execute(args(&["set-mode", "1920x1080"]));
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution
            .stderr
            .contains("ambiguous without a refresh rate"));
        assert!(execution.stderr.contains("1920x1080@50"));
        assert!(execution.stderr.contains("1920x1080@60"));
    }

    #[test]
    fn set_mode_refuses_an_unknown_resolution_with_available_modes() {
        let execution = fake_app().execute(args(&["set-mode", "800x600"]));
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stderr.contains("no mode matches 800x600"));
        assert!(execution.stderr.contains("1920x1080@60"));
    }

    #[test]
    fn set_mode_rejects_malformed_specs() {
        for arguments in [
            vec!["set-mode"],
            vec!["set-mode", "abc"],
            vec!["set-mode", "1920"],
            vec!["set-mode", "0x1080"],
            vec!["set-mode", "1920x0"],
            vec!["set-mode", "1920x1080@0"],
            vec!["set-mode", "1920xabc@60"],
        ] {
            let execution = fake_app().execute(args(&arguments));
            assert_eq!(execution.exit_code, 1, "args: {arguments:?}");
            assert!(execution.stdout.is_empty(), "args: {arguments:?}");
            assert!(!execution.stderr.is_empty(), "args: {arguments:?}");
        }
    }

    #[test]
    fn set_mode_write_refusal_is_a_runtime_error() {
        let control = FakeControl {
            mode_write_failure: Some("the mode write was rejected"),
            ..FakeControl::new()
        };
        let execution = fake_app_with(control, FakeGrantBackend::new())
            .execute(args(&["set-mode", "1920x1080@50"]));
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("the mode write was rejected"));
    }

    #[test]
    fn layout_prints_position_size_refresh_and_primary() {
        let execution = fake_app().execute(["layout".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let lines: Vec<&str> = execution.stdout.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines.contains(&"card0-DP-1 id-alpha +0+0 1920x1080@60 primary"));
        assert!(lines.contains(&"card1-HDMI-A-1 id-beta +1920+0 2560x1440@144"));
    }

    #[test]
    fn layout_json_returns_rows_with_positions_and_the_primary_flag() {
        let execution = fake_app().execute(["--json".to_string(), "layout".to_string()]);
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let value: serde_json::Value = serde_json::from_str(&execution.stdout).unwrap();
        assert!(value.is_array(), "layout --json must emit a bare array");
        let rows = value.as_array().expect("rows must be a list");
        assert_eq!(rows.len(), 2);
        let alpha = rows
            .iter()
            .find(|row| {
                row.get("connector").and_then(serde_json::Value::as_str) == Some("card0-DP-1")
            })
            .expect("alpha row");
        assert_eq!(alpha["id"], "id-alpha");
        assert_eq!(alpha["x"], 0);
        assert_eq!(alpha["y"], 0);
        assert_eq!(alpha["width"], 1920);
        assert_eq!(alpha["height"], 1080);
        assert_eq!(alpha["refresh_hz"], 60);
        assert_eq!(alpha["primary"], true);
        let beta = rows
            .iter()
            .find(|row| {
                row.get("connector").and_then(serde_json::Value::as_str) == Some("card1-HDMI-A-1")
            })
            .expect("beta row");
        assert_eq!(beta["x"], 1920);
        assert_eq!(beta["primary"], false);
    }

    #[test]
    fn layout_read_failure_is_a_runtime_error() {
        let control = FakeControl {
            snapshot_failure: Some("the display layout is unavailable"),
            ..FakeControl::new()
        };
        let execution =
            fake_app_with(control, FakeGrantBackend::new()).execute(["layout".to_string()]);
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution
            .stderr
            .contains("the display layout is unavailable"));
    }

    #[test]
    fn modes_reports_an_unreadable_mode_list_as_an_error() {
        let control = FakeControl {
            modes_failure: Some("modes are unreadable"),
            ..FakeControl::new()
        };
        let execution = fake_app_with(control, FakeGrantBackend::new())
            .execute(["--json".to_string(), "modes".to_string()]);
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("modes are unreadable"));
        assert!(execution.stderr.contains("card0-DP-1"));
    }

    #[test]
    fn display_selectors_refuse_an_ambiguous_id_and_never_match_a_prefix() {
        let control = FakeControl {
            snapshots: vec![
                display_snapshot(
                    "id-alpha",
                    "card0-DP-1",
                    (0.0, 0.0, 1920.0, 1080.0),
                    true,
                    Some((1, 1920, 1080, 60)),
                ),
                display_snapshot(
                    "id-alpha",
                    "card1-HDMI-A-1",
                    (1920.0, 0.0, 2560.0, 1440.0),
                    false,
                    Some((2, 2560, 1440, 144)),
                ),
            ],
            ..FakeControl::new()
        };
        let app = fake_app_with(control, FakeGrantBackend::new());

        let ambiguous = app.execute(["modes".to_string(), "id-alpha".to_string()]);
        assert_eq!(ambiguous.exit_code, 1);
        assert!(
            ambiguous.stderr.contains("ambiguous"),
            "{}",
            ambiguous.stderr
        );
        assert!(ambiguous.stderr.contains("card0-DP-1"));
        assert!(ambiguous.stderr.contains("card1-HDMI-A-1"));

        let prefix = app.execute(["modes".to_string(), "id-al".to_string()]);
        assert_eq!(prefix.exit_code, 1);
        assert!(prefix.stderr.contains("no display matches `id-al`"));
    }

    #[test]
    fn arrange_applies_positions_and_the_named_primary_atomically() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.layout_writes);
        let app = fake_app_with(control, FakeGrantBackend::new());
        let execution = app.execute(args(&[
            "arrange",
            "card0-DP-1=100,50",
            "card1-HDMI-A-1=1920,0",
            "--primary",
            "card0-DP-1",
        ]));
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "");
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let placements = &writes[0];
        assert_eq!(placements.len(), 2);
        let alpha = placements
            .iter()
            .find(|placement| placement.handle.id() == "id-alpha")
            .expect("alpha placement");
        assert_eq!((alpha.x, alpha.y, alpha.primary), (100, 50, true));
        let beta = placements
            .iter()
            .find(|placement| placement.handle.id() == "id-beta")
            .expect("beta placement");
        assert_eq!((beta.x, beta.y, beta.primary), (1920, 0, false));
    }

    #[test]
    fn arrange_defaults_the_primary_to_the_current_one() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.layout_writes);
        let app = fake_app_with(control, FakeGrantBackend::new());
        let execution = app.execute(args(&[
            "arrange",
            "card0-DP-1=100,50",
            "card1-HDMI-A-1=1920,0",
        ]));
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        let writes = writes.lock().unwrap();
        let placements = &writes[0];
        let alpha = placements
            .iter()
            .find(|placement| placement.handle.id() == "id-alpha")
            .expect("alpha placement");
        assert!(
            alpha.primary,
            "the current primary survives when none is requested"
        );
        let beta = placements
            .iter()
            .find(|placement| placement.handle.id() == "id-beta")
            .expect("beta placement");
        assert!(!beta.primary);
    }

    #[test]
    fn arrange_unknown_display_is_refused_without_a_write() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.layout_writes);
        let app = fake_app_with(control, FakeGrantBackend::new());
        let execution = app.execute(args(&["arrange", "card9-VGA-9=0,0"]));
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("card9-VGA-9"));
        assert!(writes.lock().unwrap().is_empty());
    }

    #[test]
    fn arrange_layout_refusal_is_a_runtime_error() {
        let control = FakeControl {
            layout_write_failure: Some("the layout was refused"),
            ..FakeControl::new()
        };
        let execution = fake_app_with(control, FakeGrantBackend::new())
            .execute(args(&["arrange", "card0-DP-1=100,50"]));
        assert_eq!(execution.exit_code, 1);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("the layout was refused"));
    }

    #[test]
    fn arrange_rejects_malformed_assignments() {
        for arguments in [
            vec!["arrange"],
            vec!["arrange", "card0-DP-1"],
            vec!["arrange", "card0-DP-1=0"],
            vec!["arrange", "card0-DP-1=x,0"],
            vec!["arrange", "card0-DP-1=0,y"],
            vec!["arrange", "=0,0"],
            vec!["arrange", "--primary"],
            vec!["arrange", "--primary", "card0-DP-1"],
        ] {
            let execution = fake_app().execute(args(&arguments));
            assert_eq!(execution.exit_code, 1, "args: {arguments:?}");
            assert!(execution.stdout.is_empty(), "args: {arguments:?}");
            assert!(!execution.stderr.is_empty(), "args: {arguments:?}");
        }
    }

    #[test]
    fn primary_sets_the_flag_without_moving_any_display() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.layout_writes);
        let app = fake_app_with(control, FakeGrantBackend::new());
        let execution = app.execute(args(&["primary", "card1-HDMI-A-1"]));
        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(execution.stdout, "");
        let writes = writes.lock().unwrap();
        let placements = &writes[0];
        let alpha = placements
            .iter()
            .find(|placement| placement.handle.id() == "id-alpha")
            .expect("alpha placement");
        assert_eq!((alpha.x, alpha.y, alpha.primary), (0, 0, false));
        let beta = placements
            .iter()
            .find(|placement| placement.handle.id() == "id-beta")
            .expect("beta placement");
        assert_eq!((beta.x, beta.y, beta.primary), (1920, 0, true));
    }

    #[test]
    fn primary_requires_a_known_display() {
        let missing = fake_app().execute(["primary".to_string()]);
        assert_eq!(missing.exit_code, 1);
        assert!(missing.stderr.contains("primary requires a display"));

        let unknown = fake_app().execute(["primary".to_string(), "card9-VGA-9".to_string()]);
        assert_eq!(unknown.exit_code, 1);
        assert!(unknown.stderr.contains("no display matches `card9-VGA-9`"));
    }

    #[test]
    fn apply_layout_with_an_empty_config_writes_nothing() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.layout_writes);
        let applied = apply_config_layout(&control, &BTreeMap::new()).unwrap();
        assert!(applied.is_none());
        assert!(writes.lock().unwrap().is_empty());
    }

    #[test]
    fn apply_layout_applies_the_configured_positions_and_reports_the_count() {
        let control = FakeControl::new();
        let writes = Arc::clone(&control.layout_writes);
        let layout = BTreeMap::from([(
            "card0-DP-1".to_string(),
            LayoutPosition {
                x: 100,
                y: 50,
                primary: true,
            },
        )]);
        let applied = apply_config_layout(&control, &layout).unwrap();
        assert_eq!(applied, Some(2));
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            (writes[0][0].x, writes[0][0].y, writes[0][0].primary),
            (100, 50, true)
        );
        assert_eq!(
            (writes[0][1].x, writes[0][1].y, writes[0][1].primary),
            (1920, 0, false)
        );
    }

    #[test]
    fn apply_layout_text_reports_the_noop_and_the_count() {
        assert_eq!(apply_layout_text(None), "no configured display positions");
        assert_eq!(apply_layout_text(Some(1)), "applied 1 display position");
        assert_eq!(apply_layout_text(Some(2)), "applied 2 display positions");
    }

    #[test]
    fn write_verbs_reject_json_that_is_not_declared() {
        for command in ["set-mode", "arrange", "primary", "apply-layout"] {
            let execution = fake_app().execute([command.to_string(), "--json".to_string()]);
            assert_eq!(execution.exit_code, EXIT_USAGE, "command: {command}");
            assert!(execution.stderr.contains("does not support --json"));
        }
    }

    #[test]
    fn mode_control_check_maps_every_platform_and_session() {
        let linux = crate::platform::PlatformSupport {
            name: "linux",
            supported: true,
        };
        let macos = crate::platform::PlatformSupport {
            name: "macos",
            supported: true,
        };
        let windows = crate::platform::PlatformSupport {
            name: "windows",
            supported: false,
        };

        let control = FakeControl::new();
        let ok = mode_control_result(linux, crate::platform::DisplayServer::X11, &control);
        assert_eq!(ok.status, DoctorStatus::Ok);
        assert!(ok.message.contains("2 of 2 displays"), "{}", ok.message);

        let no_modes = FakeControl {
            modes: BTreeMap::new(),
            ..FakeControl::new()
        };
        let warn = mode_control_result(linux, crate::platform::DisplayServer::X11, &no_modes);
        assert_eq!(warn.status, DoctorStatus::Warn);
        assert!(
            warn.message
                .contains("no connected display reports selectable modes"),
            "{}",
            warn.message
        );

        let no_primary = FakeControl {
            snapshots: vec![display_snapshot(
                "id-alpha",
                "card0-DP-1",
                (0.0, 0.0, 1920.0, 1080.0),
                false,
                Some((1, 1920, 1080, 60)),
            )],
            ..FakeControl::new()
        };
        let warn = mode_control_result(linux, crate::platform::DisplayServer::X11, &no_primary);
        assert_eq!(warn.status, DoctorStatus::Warn);
        assert!(warn.message.contains("primary"), "{}", warn.message);

        let wayland = mode_control_result(linux, crate::platform::DisplayServer::Wayland, &control);
        assert_eq!(wayland.status, DoctorStatus::Warn);
        assert!(
            wayland
                .message
                .contains("display configuration needs an X11 session"),
            "{}",
            wayland.message
        );
        assert!(wayland.fix.is_some());

        let mac = mode_control_result(macos, crate::platform::DisplayServer::None, &control);
        assert_eq!(mac.status, DoctorStatus::Warn);
        assert!(mac.message.contains("gated"), "{}", mac.message);
        assert!(mac.message.contains("arrangement is available"));

        let unsupported =
            mode_control_result(windows, crate::platform::DisplayServer::None, &control);
        assert_eq!(unsupported.status, DoctorStatus::Fail);
        assert!(unsupported.message.contains("windows"));
    }

    #[test]
    fn mode_control_check_fails_when_the_layout_is_unreadable() {
        let control = FakeControl {
            snapshot_failure: Some("the display layout is unavailable"),
            ..FakeControl::new()
        };
        let result = mode_control_result(
            crate::platform::PlatformSupport {
                name: "linux",
                supported: true,
            },
            crate::platform::DisplayServer::X11,
            &control,
        );
        assert_eq!(result.status, DoctorStatus::Fail);
        assert!(result.message.contains("the display layout is unavailable"));
    }

    #[test]
    fn layout_restore_check_warns_only_for_recorded_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config").join("qol-tray");

        let clean = layout_restore_result(Some(&config_root));
        assert_eq!(clean.status, DoctorStatus::Ok);
        assert!(clean.message.contains("no display layout snapshot"));

        let session_dir = crate::config::session_dir(&config_root).unwrap();
        let store = crate::session::SessionStore::new(session_dir);
        let snapshot = crate::session::LayoutSnapshot {
            schema_version: 1,
            layout_id: crate::session::LAYOUT_SNAPSHOT_ID.to_string(),
            placements: vec![
                crate::session::PlacementRecord {
                    id: "id-alpha".to_string(),
                    connector: "card0-DP-1".to_string(),
                    x: 0,
                    y: 0,
                    primary: true,
                },
                crate::session::PlacementRecord {
                    id: "id-beta".to_string(),
                    connector: "card1-HDMI-A-1".to_string(),
                    x: 1920,
                    y: 0,
                    primary: false,
                },
            ],
            modes: Vec::new(),
            mutations: 0,
            handoff: false,
            adopt_generation: None,
        };
        store.claim_layout(&snapshot).unwrap();

        let claimed = layout_restore_result(Some(&config_root));
        assert_eq!(
            claimed.status,
            DoctorStatus::Ok,
            "a snapshot that never recorded a mutation is not pending restore"
        );
        assert!(claimed.message.contains("no display layout snapshot"));

        store.touch_layout().unwrap();
        let pending = layout_restore_result(Some(&config_root));
        assert_eq!(pending.status, DoctorStatus::Warn);
        assert!(
            pending.message.contains("card0-DP-1"),
            "{}",
            pending.message
        );
        assert!(
            pending.message.contains("card1-HDMI-A-1"),
            "{}",
            pending.message
        );

        assert_eq!(layout_restore_result(None).status, DoctorStatus::Fail);
    }
}
