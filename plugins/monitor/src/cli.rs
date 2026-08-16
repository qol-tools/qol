use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use qol_headless::{
    Command, CommandContext, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput,
};
use qol_windowing::display::DisplayHandle;

use crate::monitor::{
    BrightnessState, DisplayControl, MonitorError, BRIGHTNESS_MAX, BRIGHTNESS_MIN, BRIGHTNESS_STEP,
};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "plugin-monitor";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_control(crate::monitor::StubControl)
}

fn app_with_control<C>(control: C) -> HeadlessApp
where
    C: DisplayControl + Send + Sync + 'static,
{
    let control: Arc<dyn DisplayControl> = Arc::new(control);
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Inspect and control display brightness, gamma, and modes.")
        .default_command(["list"])
        .command(list_command(Arc::clone(&control)))
        .command(status_command(Arc::clone(&control)))
        .command(get_command(Arc::clone(&control)))
        .command(set_command(Arc::clone(&control)))
        .command(up_command(Arc::clone(&control)))
        .command(down_command(Arc::clone(&control)))
        .command(settings_command())
        .doctor_checks(doctor_checks())
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

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by the plugin.",
            platform_supported_check,
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify the plugin can run without persistent config.",
            || {
                Ok(DoctorCheckResult::ok(
                    "config_readable",
                    "No persistent config is required.",
                ))
            },
        ),
    ]
}

fn platform_supported_check() -> Result<DoctorCheckResult> {
    Ok(platform_supported_result(crate::platform::current_support()))
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

    use qol_headless::{DoctorReport, DoctorStatus, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;
    use crate::monitor::{BrightnessSource, DisplayCapabilities, GammaState, HdrState};

    #[derive(Clone)]
    struct FakeControl {
        displays: Vec<DisplayHandle>,
        brightness: Arc<Mutex<BrightnessState>>,
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
            }
        }
    }

    impl DisplayControl for FakeControl {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(self.displays.clone())
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities {
                brightness_ddc: true,
                ..DisplayCapabilities::none()
            })
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

    fn fake_app() -> HeadlessApp {
        app_with_control(FakeControl::new())
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
    fn doctor_json_is_stable_in_both_global_flag_positions() {
        let before = fake_app().execute(["--json".to_string(), "doctor".to_string()]);
        let after = fake_app().execute(["doctor".to_string(), "--json".to_string()]);
        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 2);
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
            DoctorStatus::Ok
        );
    }

    #[test]
    fn doctor_help_names_both_skeleton_checks() {
        let first = fake_app().execute(["help".to_string(), "doctor".to_string()]);
        let final_token = fake_app().execute(["doctor".to_string(), "help".to_string()]);
        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Run read-only health checks."));
        assert!(first.stdout.contains("platform_supported"));
        assert!(first.stdout.contains("config_readable"));
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
}
