use std::process::ExitCode;

use anyhow::Result;
use qol_headless::{
    Command, CommandContext, CommandResult, DoctorCheck, DoctorCheckResult, HeadlessApp,
};
use serde_json::json;

use crate::platform::{ConfigInspection, Platform, PlatformAdapter, TrustStatus};

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "keyremap";

pub(crate) fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app(Platform).run(args)
}

fn app<A>(adapter: A) -> HeadlessApp
where
    A: PlatformAdapter,
{
    app_with_handlers(adapter, launch_settings_page)
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

    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Run and control native key, mouse, and scroll remapping.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .about("Run the key-remap daemon and native event tap.")
                .usage(format!("{BINARY_NAME} run"))
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
                .usage(format!("{BINARY_NAME} reload"))
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
                .usage(format!("{BINARY_NAME} toggle"))
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
                .usage(format!("{BINARY_NAME} kill"))
                .output("The daemon delivery result on stderr.")
                .exit_behavior("Exits zero whether or not a daemon is currently running.")
                .run_result(move |context| {
                    no_args(context)?;
                    kill.kill()
                }),
        )
        .command(settings_command(settings))
        .doctor_checks(doctor_checks(adapter))
}

fn launch_settings_page() -> std::io::Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
}

fn settings_command(settings: impl Fn() -> std::io::Result<()> + Send + Sync + 'static) -> Command {
    Command::new("settings")
        .alias("--settings")
        .about("Open the Key Remap settings page in qol-tray.")
        .usage(format!("{BINARY_NAME} settings"))
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
    let trust = adapter;

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
    ]
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

    use qol_headless::{CommandResult, DoctorReport, EXIT_RUNTIME_ERROR, EXIT_SUCCESS, EXIT_USAGE};

    use super::*;

    #[derive(Default)]
    struct Calls {
        launch: AtomicUsize,
        reload: AtomicUsize,
        toggle: AtomicUsize,
        kill: AtomicUsize,
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
        assert_eq!(report.checks.len(), 4);
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
}
