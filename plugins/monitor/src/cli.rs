use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};
use qol_windowing::display::DisplayHandle;

use crate::monitor::{
    BrightnessState, DisplayControl, GrantBackend, I2cGrantState, MonitorError, RevokeOutcome,
    BRIGHTNESS_MAX, BRIGHTNESS_MIN, BRIGHTNESS_STEP,
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
    let device = crate::config::load(config_root.as_deref().unwrap_or(std::path::Path::new("")))
        .unwrap_or_else(|error| {
            eprintln!("[plugin-monitor] device config unreadable: {error:#}");
            crate::config::DeviceConfig::default()
        });
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
        .command(daemon_command())
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

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings.")
        .usage(format!("{BINARY_NAME} settings"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if the settings URL cannot be opened.")
        .run_plain_text(|_| {
            qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
                .context("failed to open settings URL")?;
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
    let config_root_for_config = config_root.clone();
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
            "config_readable",
            "Verify the device-scope config is readable.",
            move || Ok(config_readable_result(config_root_for_config.as_deref())),
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

fn config_readable_result(config_root: Option<&std::path::Path>) -> DoctorCheckResult {
    let Some(root) = config_root else {
        return DoctorCheckResult::fail(
            "config_readable",
            "cannot locate the qol config directory",
        );
    };
    match crate::config::load(root) {
        Ok(config) => DoctorCheckResult::ok(
            "config_readable",
            format!(
                "device-scope config: {} preferred brightness, {} policy selections",
                config.preferred_brightness.len(),
                config.policy.len()
            ),
        ),
        Err(error) => DoctorCheckResult::fail(
            "config_readable",
            format!("device-scope config is unreadable: {error:#}"),
        ),
    }
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
    use crate::monitor::{BrightnessSource, DisplayCapabilities, GammaState, GrantError, HdrState};

    #[derive(Clone)]
    struct FakeControl {
        displays: Vec<DisplayHandle>,
        brightness: Arc<Mutex<BrightnessState>>,
        probe_failure: Option<ProbeFailure>,
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

        fn list_modes(
            &self,
            _handle: &DisplayHandle,
        ) -> Result<Vec<crate::monitor::DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &crate::monitor::DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
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
        for command in ["list", "get", "set", "up", "doctor"] {
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
        assert_eq!(report.checks.len(), 8);
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
            "config_readable",
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
        assert!(config.message.contains("device-scope config"));

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
            "config_readable",
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
}
