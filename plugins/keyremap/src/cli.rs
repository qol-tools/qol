use std::process::ExitCode;

use anyhow::Result;
use qol_headless::{
    Command, CommandContext, CommandResult, DoctorCheck, DoctorCheckResult, HeadlessApp,
};
use qol_plugin_api::manifest::{parse_version, PluginManifest, SystemDependency};
use serde_json::json;

use crate::platform::{
    ConfigInspection, DriverState, ExtensionState, HelperState, LayoutGap, Platform,
    PlatformAdapter, Probe, SecureInputHolder, TrustStatus, INPUT_MONITORING_FIX,
};

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

const MANIFEST: &str = include_str!("../plugin.toml");
const DRIVER_NAME: &str = "Karabiner-DriverKit-VirtualHIDDevice";

fn required_driver() -> Result<SystemDependency> {
    PluginManifest::parse_and_validate(MANIFEST)?
        .dependencies
        .and_then(|dependencies| {
            dependencies
                .system
                .into_iter()
                .find(|system| system.name == DRIVER_NAME)
        })
        .ok_or_else(|| anyhow::anyhow!("plugin.toml does not declare {DRIVER_NAME}"))
}

fn install_command() -> String {
    let binary = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| PLUGIN_ID.to_string());
    format!("sudo \"{binary}\" install-hid-helper")
}

fn install_fix() -> String {
    format!("Run: {}", install_command())
}

pub(crate) fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app(Platform).run(args)
}

fn app<A>(adapter: A) -> HeadlessApp
where
    A: PlatformAdapter,
{
    app_with_handlers(adapter, || {
        qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
    })
}

fn app_with_handlers<A, Settings>(adapter: A, settings: Settings) -> HeadlessApp
where
    A: PlatformAdapter,
    Settings: Fn() -> std::io::Result<()> + Send + Sync + 'static,
{
    let launch = adapter.clone();
    let reload = adapter.clone();
    let toggle = adapter.clone();
    let kill = adapter.clone();
    let helper = adapter.clone();
    let install = adapter.clone();
    let uninstall = adapter.clone();

    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Run and control native key, mouse, and scroll remapping.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .about("Run the key-remap daemon and native event tap.")
                .usage(format!("{PLUGIN_ID} run"))
                .detail("Loads and resolves config before enabling interception.")
                .detail("Waits for Accessibility trust before installing CGEventTap.")
                .output("Lifecycle diagnostics on stderr.")
                .exit_behavior("Runs until killed; exits non-zero on unsupported platforms.")
                .run_result(move |context| {
                    no_args(context)?;
                    launch.launch()
                }),
        )
        .command(
            Command::new("reload")
                .alias("--reload")
                .about("Ask the running daemon to reload config atomically.")
                .usage(format!("{PLUGIN_ID} reload"))
                .output("The daemon delivery result on stderr.")
                .exit_behavior("Exits zero whether or not a daemon is currently running.")
                .run_result(move |context| {
                    no_args(context)?;
                    reload.reload()
                }),
        )
        .command(
            Command::new("toggle")
                .alias("--toggle")
                .about("Turn key remapping on or off.")
                .usage(format!("{PLUGIN_ID} toggle"))
                .output("The new remapping state on stderr.")
                .exit_behavior("Exits non-zero if the new state cannot be saved.")
                .run_result(move |context| {
                    no_args(context)?;
                    toggle.toggle()
                }),
        )
        .command(
            Command::new("kill")
                .alias("--kill")
                .about("Ask the running daemon to shut down.")
                .usage(format!("{PLUGIN_ID} kill"))
                .output("The daemon delivery result on stderr.")
                .exit_behavior("Exits zero whether or not a daemon is currently running.")
                .run_result(move |context| {
                    no_args(context)?;
                    kill.kill()
                }),
        )
        .command(
            Command::new("hid-helper")
                .about("Run the root keyboard helper that launchd starts.")
                .usage(format!("sudo {PLUGIN_ID} hid-helper"))
                .detail("Seizes the physical keyboards while Key Remap is running and feeds the virtual keyboard.")
                .detail("Releases every keyboard within 200 ms when Key Remap stops answering.")
                .output("Lifecycle diagnostics on stderr.")
                .exit_behavior("Runs until killed; exits non-zero when not root or not on macOS.")
                .run_result(move |context| {
                    no_args(context)?;
                    helper.hid_helper()
                }),
        )
        .command(
            Command::new("install-hid-helper")
                .about("Install the root keyboard helper so Key Remap survives Secure Input.")
                .usage(format!("sudo {PLUGIN_ID} install-hid-helper"))
                .detail("Copies this binary to /Library/PrivilegedHelperTools and loads a LaunchDaemon for it.")
                .detail("Starts the pqrs virtual keyboard daemon if nothing else runs it, and activates the driver.")
                .output("One line per completed step on stdout.")
                .exit_behavior("Exits non-zero without sudo, without the driver package, or when launchctl fails.")
                .run_result(move |context| {
                    no_args(context)?;
                    install.install_hid_helper()
                }),
        )
        .command(
            Command::new("uninstall-hid-helper")
                .about("Remove the root keyboard helper; Key Remap falls back to the event tap.")
                .usage(format!("sudo {PLUGIN_ID} uninstall-hid-helper"))
                .detail("Leaves the driver package installed, since other software may use it.")
                .output("One line per completed step on stdout.")
                .exit_behavior("Exits non-zero without sudo or when a file cannot be removed.")
                .run_result(move |context| {
                    no_args(context)?;
                    uninstall.uninstall_hid_helper()
                }),
        )
        .command(settings_command(settings))
        .doctor_checks(doctor_checks(adapter))
}

fn settings_command(settings: impl Fn() -> std::io::Result<()> + Send + Sync + 'static) -> Command {
    Command::new("settings")
        .alias("--settings")
        .about("Open the Key Remap settings page in qol-tray.")
        .usage(format!("{PLUGIN_ID} settings"))
        .output("No stdout on success; opens the settings URL through the platform launcher.")
        .exit_behavior("Exits non-zero if the settings URL cannot be launched.")
        .run_result(move |_| Ok(result_for(settings())))
}

fn no_args(context: &CommandContext) -> Result<()> {
    if let Some(argument) = context.args().first() {
        anyhow::bail!("keyremap: unexpected argument {argument:?}");
    }
    Ok(())
}

fn result_for(result: std::io::Result<()>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::new("", format!("{error}\n"), 1),
    }
}

fn doctor_checks<A>(adapter: A) -> Vec<DoctorCheck>
where
    A: PlatformAdapter,
{
    let platform = adapter.clone();
    let config = adapter.clone();
    let rules = adapter.clone();
    let trust = adapter.clone();
    let driver = adapter.clone();
    let daemon = adapter.clone();
    let helper = adapter.clone();
    let secure = adapter.clone();
    let characters = adapter;

    vec![
        DoctorCheck::new(
            "platform_supported",
            "Verify the current platform is declared by Key Remap.",
            move || Ok(platform_result(&platform)),
        ),
        DoctorCheck::new(
            "config_readable",
            "Read and deserialize config without changing config state.",
            move || config_result(&config),
        ),
        DoctorCheck::new(
            "rules_valid",
            "Validate typed remapping rules without enabling interception.",
            move || rules_result(&rules),
        ),
        DoctorCheck::new(
            "accessibility_trust",
            "Observe Accessibility trust without prompting for permission.",
            move || Ok(trust_result(&trust)),
        ),
        DoctorCheck::new(
            "virtual_hid_driver",
            "Check the virtual keyboard driver package and its system extension.",
            move || {
                supported_or(&driver, "virtual_hid_driver", |adapter| {
                    Ok(driver_result(
                        &adapter.virtual_hid_driver(),
                        &required_driver()?,
                    ))
                })
            },
        ),
        DoctorCheck::new(
            "virtual_hid_daemon",
            "Check the virtual keyboard daemon is running.",
            move || {
                supported_or(&daemon, "virtual_hid_daemon", |adapter| {
                    Ok(daemon_result(&adapter.virtual_hid_daemon()))
                })
            },
        ),
        DoctorCheck::new(
            "hid_helper",
            "Ask the root keyboard helper for its status without changing anything.",
            move || {
                supported_or(&helper, "hid_helper", |adapter| {
                    Ok(helper_result(&adapter.hid_helper_state()))
                })
            },
        ),
        DoctorCheck::new(
            "secure_input",
            "Report which app holds Secure Input and which key strategy is active.",
            move || {
                supported_or(&secure, "secure_input", |adapter| {
                    Ok(secure_input_result(
                        &adapter.secure_input(),
                        &adapter.hid_helper_state(),
                    ))
                })
            },
        ),
        DoctorCheck::new(
            "layout_characters",
            "Check every character rule can be typed in the active keyboard layout.",
            move || {
                supported_or(&characters, "layout_characters", |adapter| {
                    Ok(layout_result(&adapter.layout_gaps()))
                })
            },
        ),
    ]
}

fn supported_or<A: PlatformAdapter>(
    adapter: &A,
    id: &str,
    check: impl FnOnce(&A) -> Result<DoctorCheckResult>,
) -> Result<DoctorCheckResult> {
    if !adapter.supported() {
        return Ok(DoctorCheckResult::fail(
            id,
            "The virtual keyboard path is unavailable on this platform.",
        )
        .with_fix("Run Key Remap on macOS."));
    }
    check(adapter)
}

fn unknown(id: &str, reason: &str) -> DoctorCheckResult {
    DoctorCheckResult::warn(id, format!("Could not tell: {reason}."))
}

fn driver_result(probe: &Probe<DriverState>, required: &SystemDependency) -> DoctorCheckResult {
    let id = "virtual_hid_driver";
    let state = match probe {
        Probe::Known(state) => state,
        Probe::Unknown(reason) => return unknown(id, reason),
    };
    let download = format!(
        "Install {} {} or newer from {}, then run: {}",
        required.name,
        required.min_version,
        required.url,
        install_command()
    );
    let Some(installed) = &state.installed_version else {
        return DoctorCheckResult::warn(
            id,
            "The virtual keyboard driver is not installed, so Key Remap stops while an app holds Secure Input.",
        )
        .with_fix(download);
    };
    let current_enough = match (
        parse_version(installed),
        parse_version(&required.min_version),
    ) {
        (Some(installed), Some(minimum)) => installed >= minimum,
        _ => false,
    };
    if !current_enough {
        return DoctorCheckResult::warn(
            id,
            format!(
                "The virtual keyboard driver is {installed}, older than {}.",
                required.min_version
            ),
        )
        .with_fix(download);
    }
    match state.extension {
        ExtensionState::Activated => DoctorCheckResult::ok(
            id,
            format!("The virtual keyboard driver {installed} is installed and its extension is active."),
        ),
        ExtensionState::NotActivated | ExtensionState::Missing => DoctorCheckResult::warn(
            id,
            format!("The virtual keyboard driver {installed} is installed but its extension is not active."),
        )
        .with_fix(install_fix()),
    }
}

fn daemon_result(probe: &Probe<bool>) -> DoctorCheckResult {
    let id = "virtual_hid_daemon";
    match probe {
        Probe::Known(true) => DoctorCheckResult::ok(id, "The virtual keyboard daemon is running."),
        Probe::Known(false) => {
            DoctorCheckResult::warn(id, "The virtual keyboard daemon is not running.")
                .with_fix(install_fix())
        }
        Probe::Unknown(reason) => unknown(id, reason),
    }
}

fn helper_result(probe: &Probe<HelperState>) -> DoctorCheckResult {
    let id = "hid_helper";
    let report = match probe {
        Probe::Unknown(reason) => return unknown(id, reason),
        Probe::Known(HelperState::NotInstalled) => {
            return DoctorCheckResult::warn(
                id,
                "The keyboard helper is not installed, so Key Remap uses the event tap.",
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::NotRunning) => {
            return DoctorCheckResult::warn(
                id,
                "The keyboard helper is installed but launchd has not started it.",
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::VersionMismatch { helper, expected }) => {
            return DoctorCheckResult::warn(
                id,
                format!("The keyboard helper speaks protocol {helper} and this Key Remap speaks {expected}."),
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::Running(report)) => report,
    };
    if !report.input_monitoring {
        return DoctorCheckResult::warn(
            id,
            "The keyboard helper cannot read the keyboard: Input Monitoring is off.",
        )
        .with_fix(INPUT_MONITORING_FIX);
    }
    if !report.conflicts.is_empty() {
        return DoctorCheckResult::warn(
            id,
            format!(
                "Another app holds these keyboards: {}.",
                report.conflicts.join(", ")
            ),
        )
        .with_fix("Quit the app that grabs the keyboard, such as Karabiner-Elements.");
    }
    if !report.virtual_keyboard_ready {
        return DoctorCheckResult::warn(
            id,
            "The keyboard helper is running but the virtual keyboard is not ready.",
        )
        .with_fix(install_fix());
    }
    let seized = if report.seized.is_empty() {
        "no keyboards right now, because Key Remap is not connected".to_string()
    } else {
        report.seized.join(", ")
    };
    DoctorCheckResult::ok(
        id,
        format!("The keyboard helper is running and holds {seized}."),
    )
}

fn secure_input_result(
    holder: &Probe<Option<SecureInputHolder>>,
    helper: &Probe<HelperState>,
) -> DoctorCheckResult {
    let id = "secure_input";
    let virtual_hid = matches!(
        helper,
        Probe::Known(HelperState::Running(report)) if !report.seized.is_empty()
    );
    let strategy = if virtual_hid {
        "virtual_hid"
    } else {
        "event_tap"
    };
    match holder {
        Probe::Unknown(reason) => unknown(id, reason),
        Probe::Known(None) => DoctorCheckResult::ok(
            id,
            format!("No app holds Secure Input. Key input strategy: {strategy}."),
        ),
        Probe::Known(Some(holder)) => {
            let paused = if virtual_hid {
                "qol hotkeys are paused"
            } else {
                "Key Remap and qol hotkeys are paused"
            };
            DoctorCheckResult::warn(
                id,
                format!(
                    "{} (pid {}) holds Secure Input, so {paused}. Key input strategy: {strategy}.",
                    holder.app, holder.pid
                ),
            )
            .with_fix(format!(
                "Quit {} or turn off its secure keyboard entry.",
                holder.app
            ))
        }
    }
}

fn layout_result(probe: &Probe<Vec<LayoutGap>>) -> DoctorCheckResult {
    let id = "layout_characters";
    match probe {
        Probe::Unknown(reason) => unknown(id, reason),
        Probe::Known(gaps) if gaps.is_empty() => DoctorCheckResult::ok(
            id,
            "Every character rule can be typed in the active keyboard layout.",
        ),
        Probe::Known(gaps) => {
            let listed: Vec<String> = gaps
                .iter()
                .map(|gap| format!("{:?} ({})", gap.character, gap.rule))
                .collect();
            DoctorCheckResult::warn(
                id,
                format!(
                    "The active keyboard layout cannot type {}.",
                    listed.join(", ")
                ),
            )
            .with_fix("Switch to a layout that has these characters, or change the rules.")
        }
    }
}

fn platform_result(adapter: &impl PlatformAdapter) -> DoctorCheckResult {
    let result = if adapter.supported() {
        DoctorCheckResult::ok(
            "platform_supported",
            format!(
                "{} is declared and has a native key-remap adapter.",
                adapter.name()
            ),
        )
    } else {
        DoctorCheckResult::fail(
            "platform_supported",
            format!("{} is not declared by Key Remap.", adapter.name()),
        )
        .with_fix("Run Key Remap on macOS.")
    };
    result.with_details(json!({
        "platform": adapter.name(),
        "declared": adapter.supported(),
    }))
}

fn config_result(adapter: &impl PlatformAdapter) -> Result<DoctorCheckResult> {
    if !adapter.supported() {
        return Ok(DoctorCheckResult::fail(
            "config_readable",
            "Typed Key Remap config is unavailable on this platform.",
        )
        .with_fix("Run Key Remap on macOS."));
    }

    let inspected = adapter.inspect_config()?;
    let source = if inspected.source {
        "stored config"
    } else {
        "contract defaults"
    };
    Ok(DoctorCheckResult::ok(
        "config_readable",
        format!("Typed config loaded from {source}."),
    )
    .with_details(config_details(&inspected)))
}

fn rules_result(adapter: &impl PlatformAdapter) -> Result<DoctorCheckResult> {
    if !adapter.supported() {
        return Ok(DoctorCheckResult::fail(
            "rules_valid",
            "Rule validation is unavailable on this platform.",
        )
        .with_fix("Run Key Remap on macOS."));
    }

    let inspected = adapter.inspect_config()?;
    let details = config_details(&inspected);
    if inspected.issues.is_empty() {
        return Ok(DoctorCheckResult::ok(
            "rules_valid",
            "All configured remapping rules resolve to implemented semantics.",
        )
        .with_details(details));
    }

    Ok(DoctorCheckResult::fail(
        "rules_valid",
        format!(
            "{} invalid rule value(s): {}",
            inspected.issues.len(),
            inspected.issues.join("; ")
        ),
    )
    .with_fix("Correct or remove the reported rules in Key Remap settings.")
    .with_details(details))
}

fn config_details(inspected: &ConfigInspection) -> serde_json::Value {
    json!({
        "source": if inspected.source { "stored" } else { "defaults" },
        "enabled": inspected.enabled,
        "char_rules": inspected.char_rules,
        "char_swaps": inspected.char_swaps,
        "key_rules": inspected.key_rules,
        "mouse_rules": inspected.mouse_rules,
        "scroll_rules": inspected.scroll_rules,
        "issues": inspected.issues,
        "inspection": "read_only",
    })
}

fn trust_result(adapter: &impl PlatformAdapter) -> DoctorCheckResult {
    if !adapter.supported() {
        return DoctorCheckResult::fail(
            "accessibility_trust",
            "Accessibility trust is unavailable on this platform.",
        )
        .with_fix("Run Key Remap on macOS.");
    }

    match adapter.trust_status() {
        TrustStatus::Trusted => DoctorCheckResult::ok(
            "accessibility_trust",
            "Accessibility trust is granted; no prompt was requested.",
        ),
        TrustStatus::NotTrusted => DoctorCheckResult::warn(
            "accessibility_trust",
            "Accessibility trust is not granted; doctor did not prompt.",
        )
        .with_fix("Enable Key Remap in System Settings > Privacy & Security > Accessibility."),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use qol_headless::{
        CommandResult, DoctorReport, DoctorStatus, EXIT_RUNTIME_ERROR, EXIT_SUCCESS, EXIT_USAGE,
    };

    use super::*;
    use crate::platform::HelperReport;

    #[derive(Default)]
    struct Calls {
        launch: AtomicUsize,
        reload: AtomicUsize,
        toggle: AtomicUsize,
        kill: AtomicUsize,
        hid_helper: AtomicUsize,
        install: AtomicUsize,
        uninstall: AtomicUsize,
        settings: AtomicUsize,
        config: AtomicUsize,
        trust: AtomicUsize,
    }

    #[derive(Clone)]
    struct SentinelAdapter {
        calls: Arc<Calls>,
    }

    impl PlatformAdapter for SentinelAdapter {
        fn name(&self) -> &'static str {
            "Sentinel OS"
        }

        fn supported(&self) -> bool {
            true
        }

        fn launch(&self) -> Result<CommandResult> {
            self.calls.launch.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn reload(&self) -> Result<CommandResult> {
            self.calls.reload.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn toggle(&self) -> Result<CommandResult> {
            self.calls.toggle.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn kill(&self) -> Result<CommandResult> {
            self.calls.kill.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn hid_helper(&self) -> Result<CommandResult> {
            self.calls.hid_helper.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn install_hid_helper(&self) -> Result<CommandResult> {
            self.calls.install.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn uninstall_hid_helper(&self) -> Result<CommandResult> {
            self.calls.uninstall.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::success(""))
        }

        fn inspect_config(&self) -> Result<ConfigInspection> {
            self.calls.config.fetch_add(1, Ordering::SeqCst);
            Ok(ConfigInspection {
                source: false,
                enabled: true,
                char_rules: 0,
                char_swaps: 0,
                key_rules: 1,
                mouse_rules: 1,
                scroll_rules: 1,
                issues: Vec::new(),
            })
        }

        fn trust_status(&self) -> TrustStatus {
            self.calls.trust.fetch_add(1, Ordering::SeqCst);
            TrustStatus::Trusted
        }

        fn virtual_hid_driver(&self) -> Probe<DriverState> {
            Probe::Known(DriverState {
                installed_version: Some("8.6.0".to_string()),
                extension: ExtensionState::Activated,
            })
        }

        fn virtual_hid_daemon(&self) -> Probe<bool> {
            Probe::Known(true)
        }

        fn hid_helper_state(&self) -> Probe<HelperState> {
            Probe::Known(HelperState::Running(HelperReport {
                virtual_keyboard_ready: true,
                input_monitoring: true,
                seized: vec!["Apple Internal Keyboard / Trackpad".to_string()],
                conflicts: Vec::new(),
            }))
        }

        fn secure_input(&self) -> Probe<Option<SecureInputHolder>> {
            Probe::Known(None)
        }

        fn layout_gaps(&self) -> Probe<Vec<LayoutGap>> {
            Probe::Known(Vec::new())
        }
    }

    fn sentinel() -> (HeadlessApp, Arc<Calls>) {
        let calls = Arc::new(Calls::default());
        let settings_calls = Arc::clone(&calls);
        (
            app_with_handlers(
                SentinelAdapter {
                    calls: Arc::clone(&calls),
                },
                move || {
                    settings_calls.settings.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            ),
            calls,
        )
    }

    #[test]
    fn settings_alias_executes_the_manifest_dispatch_route() {
        let (app, calls) = sentinel();
        let execution = app.execute(["--settings".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.launch.load(Ordering::SeqCst), 0);
        assert_eq!(calls.settings.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn settings_failure_exits_nonzero_with_a_stderr_message() {
        let calls = Arc::new(Calls::default());
        let app = app_with_handlers(
            SentinelAdapter {
                calls: Arc::clone(&calls),
            },
            || {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no desktop opener",
                ))
            },
        );
        let execution = app.execute(["--settings".to_string()]);

        assert_eq!(execution.exit_code, EXIT_RUNTIME_ERROR);
        assert!(execution.stderr.contains("no desktop opener"));
    }

    #[test]
    fn result_for_maps_launch_results_without_spawning() {
        assert_eq!(result_for(Ok(())), CommandResult::success(""));

        let failure = result_for(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no desktop opener",
        )));
        assert_eq!(failure.exit_code, EXIT_RUNTIME_ERROR);
        assert!(failure.stdout.is_empty());
        assert!(failure.stderr.contains("no desktop opener"));
    }

    #[test]
    fn settings_alias_resolves_in_help_without_launching() {
        let (app, calls) = sentinel();
        let first = app.execute(["help".to_string(), "settings".to_string()]);
        let final_token = app.execute(["--settings".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(final_token.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("settings page in qol-tray"));
        assert_eq!(calls.settings.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn contextual_help_is_equivalent_and_documents_aliases() {
        let (app, _) = sentinel();
        let first = app.execute(["help".to_string(), "reload".to_string()]);
        let final_token = app.execute(["reload".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("reload config atomically"));
        assert!(first.stdout.contains("Does not support --json."));

        let settings_help = app.execute(["help".to_string(), "settings".to_string()]);
        assert_eq!(settings_help.exit_code, EXIT_SUCCESS);
        assert!(settings_help.stdout.contains("settings page in qol-tray"));
    }

    #[test]
    fn doctor_json_matches_the_shared_contract() {
        let (app, _) = sentinel();
        let before = app.execute(["--json".to_string(), "doctor".to_string()]);
        let after = app.execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        let report: DoctorReport = serde_json::from_str(&before.stdout).unwrap();
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(report.checks.len(), 9);
    }

    #[test]
    fn doctor_and_help_never_reach_operational_paths() {
        let cases = [
            vec!["help"],
            vec!["--help"],
            vec!["help", "run"],
            vec!["run", "help"],
            vec!["help", "reload"],
            vec!["--reload", "help"],
            vec!["help", "toggle"],
            vec!["--toggle", "help"],
            vec!["help", "kill"],
            vec!["--kill", "help"],
            vec!["help", "hid-helper"],
            vec!["hid-helper", "help"],
            vec!["help", "install-hid-helper"],
            vec!["install-hid-helper", "help"],
            vec!["help", "uninstall-hid-helper"],
            vec!["uninstall-hid-helper", "help"],
            vec!["help", "settings"],
            vec!["--settings", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
        ];

        for args in cases {
            let (app, calls) = sentinel();
            let execution = app.execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_eq!(calls.launch.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.reload.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.toggle.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.kill.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.hid_helper.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.install.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.uninstall.load(Ordering::SeqCst), 0, "{args:?}");
            assert_eq!(calls.settings.load(Ordering::SeqCst), 0, "{args:?}");
        }
    }

    #[test]
    fn help_does_not_even_read_config_or_trust_metadata() {
        let (app, calls) = sentinel();
        let execution = app.execute(["help".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.config.load(Ordering::SeqCst), 0);
        assert_eq!(calls.trust.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unsupported_json_cannot_launch_the_default_command() {
        for args in [vec!["--json"], vec!["--json", "run"], vec!["run", "--json"]] {
            let (app, calls) = sentinel();
            let execution = app.execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "{args:?}");
            assert_eq!(calls.launch.load(Ordering::SeqCst), 0, "{args:?}");
        }
    }

    #[test]
    fn toggle_and_alias_reach_the_adapter_exactly_once() {
        let (app, calls) = sentinel();

        assert_eq!(app.execute(["toggle".to_string()]).exit_code, EXIT_SUCCESS);
        assert_eq!(
            app.execute(["--toggle".to_string()]).exit_code,
            EXIT_SUCCESS
        );
        assert_eq!(calls.toggle.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn toggle_help_is_equivalent_and_never_reaches_the_adapter() {
        let (app, calls) = sentinel();
        let first = app.execute(["help".to_string(), "toggle".to_string()]);
        let final_token = app.execute(["toggle".to_string(), "help".to_string()]);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(final_token.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("remapping on or off"));
        assert_eq!(calls.toggle.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn operational_commands_still_reach_the_selected_adapter() {
        let (app, calls) = sentinel();

        assert_eq!(app.execute(Vec::new()).exit_code, EXIT_SUCCESS);
        assert_eq!(
            app.execute(["--reload".to_string()]).exit_code,
            EXIT_SUCCESS
        );
        assert_eq!(app.execute(["kill".to_string()]).exit_code, EXIT_SUCCESS);
        assert_eq!(calls.launch.load(Ordering::SeqCst), 1);
        assert_eq!(calls.reload.load(Ordering::SeqCst), 1);
        assert_eq!(calls.kill.load(Ordering::SeqCst), 1);
    }

    fn driver() -> SystemDependency {
        required_driver().unwrap()
    }

    #[test]
    fn the_driver_minimum_comes_from_the_manifest() {
        assert_eq!(driver().min_version, "8.6.0");
    }

    #[test]
    fn a_missing_or_old_driver_warns_with_the_download_url() {
        let missing = driver_result(
            &Probe::Known(DriverState {
                installed_version: None,
                extension: ExtensionState::Missing,
            }),
            &driver(),
        );
        assert_eq!(missing.status, DoctorStatus::Warn);
        assert!(missing.fix.unwrap().contains("github.com/pqrs-org"));

        let old = driver_result(
            &Probe::Known(DriverState {
                installed_version: Some("8.5.9".to_string()),
                extension: ExtensionState::Activated,
            }),
            &driver(),
        );
        assert_eq!(old.status, DoctorStatus::Warn);
        assert!(old.message.contains("8.5.9"));
    }

    #[test]
    fn an_inactive_extension_names_install_hid_helper() {
        let result = driver_result(
            &Probe::Known(DriverState {
                installed_version: Some("8.6.0".to_string()),
                extension: ExtensionState::NotActivated,
            }),
            &driver(),
        );
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.unwrap().contains("install-hid-helper"));
    }

    #[test]
    fn helper_states_map_to_warnings_with_the_install_fix() {
        for state in [
            HelperState::NotInstalled,
            HelperState::NotRunning,
            HelperState::VersionMismatch {
                helper: 0,
                expected: 1,
            },
        ] {
            let result = helper_result(&Probe::Known(state));
            assert_eq!(result.status, DoctorStatus::Warn);
            assert!(result.fix.unwrap().contains("install-hid-helper"));
        }
    }

    #[test]
    fn a_helper_without_input_monitoring_says_how_to_turn_it_on() {
        let result = helper_result(&Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: true,
            input_monitoring: false,
            seized: Vec::new(),
            conflicts: Vec::new(),
        })));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.unwrap().contains("Input Monitoring"));
    }

    #[test]
    fn a_keyboard_held_by_another_app_is_named() {
        let result = helper_result(&Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: true,
            input_monitoring: true,
            seized: Vec::new(),
            conflicts: vec!["Keychron K2".to_string()],
        })));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("Keychron K2"));
    }

    #[test]
    fn secure_input_names_the_app_and_the_strategy() {
        let helper = Probe::Known(HelperState::NotRunning);
        let result = secure_input_result(
            &Probe::Known(Some(SecureInputHolder {
                pid: 4242,
                app: "kitty".to_string(),
            })),
            &helper,
        );
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("kitty") && result.message.contains("4242"));
        assert!(result.message.contains("event_tap"));

        let quiet = secure_input_result(&Probe::Known(None), &helper);
        assert_eq!(quiet.status, DoctorStatus::Ok);
    }

    #[test]
    fn untypeable_characters_name_their_rule() {
        let result = layout_result(&Probe::Known(vec![LayoutGap {
            rule: "char rule ralt+3 -> あ".to_string(),
            character: "あ".to_string(),
        }]));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("char rule ralt+3"));
    }

    #[test]
    fn unknown_probes_warn_instead_of_passing() {
        fn unknown<T>() -> Probe<T> {
            Probe::Unknown("pkgutil is missing".to_string())
        }
        assert_eq!(
            driver_result(&unknown(), &driver()).status,
            DoctorStatus::Warn
        );
        assert_eq!(daemon_result(&unknown()).status, DoctorStatus::Warn);
        assert_eq!(helper_result(&unknown()).status, DoctorStatus::Warn);
        assert_eq!(layout_result(&unknown()).status, DoctorStatus::Warn);
    }
}
