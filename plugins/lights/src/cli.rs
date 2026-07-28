use std::process::ExitCode;
use std::sync::Arc;

use qol_headless::{Command, CommandResult, DoctorCheck, HeadlessApp};

use crate::runtime::actions;

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "plugin-lights";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(
        |args| result_for(crate::runtime::entrypoint(args)),
        || result_for(crate::daemon::run_from_env()),
        crate::doctor::checks(),
    )
}

fn app_with_handlers<Runtime, Daemon>(
    runtime: Runtime,
    daemon: Daemon,
    doctor_checks: Vec<DoctorCheck>,
) -> HeadlessApp
where
    Runtime: Fn(Vec<String>) -> CommandResult + Send + Sync + 'static,
    Daemon: Fn() -> CommandResult + Send + Sync + 'static,
{
    let runtime = Arc::new(runtime);
    let mut app = HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about(
            "Control Zigbee lights through a serial coordinator and daemon-backed live transport.",
        )
        .default_command(["launch"])
        .command(launch_command(Arc::clone(&runtime)))
        .command(daemon_command(daemon))
        .doctor_checks(doctor_checks);

    for action in actions::ALL_ACTIONS {
        app = app.command(action_command(action, Arc::clone(&runtime)));
    }
    app
}

fn launch_command<Runtime>(handler: Arc<Runtime>) -> Command
where
    Runtime: Fn(Vec<String>) -> CommandResult + Send + Sync + 'static,
{
    Command::new("launch")
        .about("Run the host daemon when a daemon socket is injected, otherwise open settings.")
        .usage(format!("{BINARY_NAME} launch"))
        .detail("This is the no-argument compatibility route used by qol-tray.")
        .output("Daemon and settings diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero if daemon startup or settings activation fails.")
        .run_result(move |context| Ok(handler(context.args().to_vec())))
}

fn daemon_command<Daemon>(handler: Daemon) -> Command
where
    Daemon: Fn() -> CommandResult + Send + Sync + 'static,
{
    Command::new("daemon")
        .alias("run")
        .about("Run the long-lived coordinator, action socket, device monitor, and live websocket.")
        .usage(format!("{BINARY_NAME} daemon"))
        .detail("Requires the daemon socket environment supplied by qol-tray.")
        .output("Lifecycle diagnostics are written to stderr.")
        .exit_behavior("Runs until stopped; exits non-zero if coordinator or socket startup fails.")
        .run_result(move |context| {
            reject_args("daemon", context.args())?;
            Ok(handler())
        })
}

fn action_command<Runtime>(action: &'static str, handler: Arc<Runtime>) -> Command
where
    Runtime: Fn(Vec<String>) -> CommandResult + Send + Sync + 'static,
{
    let mut command = Command::new(action)
        .about(action_about(action))
        .usage(format!("{BINARY_NAME} {action}"))
        .output("No stdout on success; operational diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero if settings, coordinator startup, or light control fails.");
    if let Some(detail) = action_detail(action) {
        command = command.detail(detail);
    }
    command.run_result(move |context| {
        let mut args = Vec::with_capacity(context.args().len() + 1);
        args.push(action.to_string());
        args.extend(context.args().iter().cloned());
        Ok(handler(args))
    })
}

fn action_about(action: &str) -> &'static str {
    match action {
        actions::TOGGLE_MAIN => "Toggle power on the configured main light target.",
        actions::ON_MAIN => "Turn on the configured main light target.",
        actions::OFF_MAIN => "Turn off the configured main light target.",
        actions::BRIGHTER_MAIN => "Increase main-target brightness by ten percentage points.",
        actions::DIMMER_MAIN => "Decrease main-target brightness by ten percentage points.",
        actions::WARMER_MAIN => "Increase the main target's color-temperature mirek value.",
        actions::COOLER_MAIN => "Decrease the main target's color-temperature mirek value.",
        actions::PRESET_1 => "Apply configured light preset 1.",
        actions::PRESET_2 => "Apply configured light preset 2.",
        actions::PRESET_3 => "Apply configured light preset 3.",
        actions::PRESET_4 => "Apply configured light preset 4.",
        actions::PRESET_5 => "Apply configured light preset 5.",
        actions::PRESET_6 => "Apply configured light preset 6.",
        actions::PRESET_7 => "Apply configured light preset 7.",
        actions::PRESET_8 => "Apply configured light preset 8.",
        actions::SETTINGS => "Open Lights settings through the native host integration.",
        actions::PAIR => "Permit one Zigbee device to join for up to 60 seconds.",
        actions::STOP_PAIR => "Close Zigbee permit-join immediately.",
        actions::SET_COLOR_MAIN => "Apply the current live color to the main target.",
        actions::SET_BRIGHTNESS_MAIN => {
            "Apply the current live brightness and color to the main target."
        }
        actions::SET_COLORTEMP_MAIN => {
            "Apply the current live color temperature to the main target."
        }
        actions::RELOAD => "Reload config or reconnect an unavailable coordinator.",
        _ => "Execute a declared Lights action.",
    }
}

fn action_detail(action: &str) -> Option<&'static str> {
    match action {
        actions::SET_COLOR_MAIN => {
            Some("Reads live_color_hex from the current host-injected plugin config.")
        }
        actions::SET_BRIGHTNESS_MAIN => {
            Some("Reads live_brightness and live_color_hex from the current plugin config.")
        }
        actions::SET_COLORTEMP_MAIN => {
            Some("Reads live_mirek from the current host-injected plugin config.")
        }
        actions::PAIR | actions::STOP_PAIR => {
            Some("Pairing is performed by the existing coordinator-backed runtime path.")
        }
        actions::RELOAD => {
            Some("An unavailable daemon reopens the coordinator; a ready daemon reloads config.")
        }
        _ => None,
    }
}

fn reject_args(command: &str, args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    anyhow::bail!("{command} does not accept arguments: {}", args.join(" "))
}

fn result_for(result: anyhow::Result<()>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::runtime_error(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use qol_headless::{DoctorCheckResult, DoctorReport, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        runtime: Mutex<Vec<Vec<String>>>,
        daemon: AtomicUsize,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let runtime_calls = Arc::clone(&calls);
        let daemon_calls = Arc::clone(&calls);
        app_with_handlers(
            move |args| {
                runtime_calls.runtime.lock().unwrap().push(args);
                CommandResult::success("")
            },
            move || {
                daemon_calls.daemon.fetch_add(1, Ordering::SeqCst);
                CommandResult::success("")
            },
            vec![DoctorCheck::new(
                "sentinel",
                "Run a deterministic read-only sentinel check.",
                || Ok(DoctorCheckResult::ok("sentinel", "read-only")),
            )],
        )
    }

    #[test]
    fn no_arguments_preserve_the_legacy_launch_route() {
        let calls = Arc::new(OperationCalls::default());
        let execution = sentinel_app(Arc::clone(&calls)).execute(Vec::<String>::new());

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(
            calls.runtime.lock().unwrap().as_slice(),
            [Vec::<String>::new()]
        );
        assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn explicit_daemon_commands_use_the_daemon_route() {
        for command in ["daemon", "run"] {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute([command.to_string()]);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "command={command}");
            assert!(calls.runtime.lock().unwrap().is_empty());
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn manifest_action_arguments_reach_the_legacy_runtime_intact() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        for action in manifest.executable_actions() {
            let args = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute(args.clone());

            assert_eq!(
                execution.exit_code, EXIT_SUCCESS,
                "action={} stderr={}",
                action.id, execution.stderr
            );
            assert_eq!(
                calls.runtime.lock().unwrap().as_slice(),
                [args],
                "action={}",
                action.id
            );
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn action_value_arguments_are_forwarded_without_cli_interpretation() {
        let calls = Arc::new(OperationCalls::default());
        let args = vec![
            actions::SET_COLOR_MAIN.to_string(),
            "#203040".to_string(),
            "42".to_string(),
        ];
        let execution = sentinel_app(Arc::clone(&calls)).execute(args.clone());

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.runtime.lock().unwrap().as_slice(), [args]);
        assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn every_manifest_action_has_contextual_help() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        for action in manifest.executable_actions() {
            let command = manifest
                .catalog_runtime_args(&action.id)
                .and_then(|args| args.into_iter().next())
                .expect("executable action must name a command");
            let first = app().execute(["help".to_string(), command.clone()]);
            let final_token = app().execute([command.clone(), "help".to_string()]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "action={}", action.id);
            assert_eq!(first.stdout, final_token.stdout, "action={}", action.id);
            assert!(first.stdout.contains("Output:"), "action={}", action.id);
            assert!(first.stdout.contains("Exit:"), "action={}", action.id);
        }
    }

    #[test]
    fn doctor_json_uses_the_shared_schema_in_both_flag_positions() {
        let calls = Arc::new(OperationCalls::default());
        let app = sentinel_app(Arc::clone(&calls));
        let mut outputs = Vec::new();
        for args in [
            vec!["--json".to_string(), "doctor".to_string()],
            vec!["doctor".to_string(), "--json".to_string()],
        ] {
            let execution = app.execute(args);
            let report: DoctorReport =
                serde_json::from_str(&execution.stdout).expect("doctor output must be valid JSON");

            assert_eq!(execution.exit_code, EXIT_SUCCESS);
            assert!(execution.stderr.is_empty());
            assert_eq!(report.plugin_id, PLUGIN_ID);
            assert_eq!(
                report
                    .checks
                    .iter()
                    .map(|check| check.id.as_str())
                    .collect::<Vec<_>>(),
                ["sentinel"]
            );
            assert!(report.checks.iter().all(|check| !check.message.is_empty()));
            outputs.push(execution.stdout);
        }
        assert_eq!(outputs[0], outputs[1]);
        assert!(calls.runtime.lock().unwrap().is_empty());
        assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn help_and_doctor_never_invoke_operational_handlers() {
        let cases = [
            vec!["help"],
            vec!["help", "daemon"],
            vec!["daemon", "help"],
            vec!["help", actions::PAIR],
            vec![actions::PAIR, "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.into_iter().map(str::to_string));

            assert_eq!(execution.exit_code, EXIT_SUCCESS);
            assert!(calls.runtime.lock().unwrap().is_empty());
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn json_rejection_happens_before_action_dispatch() {
        let calls = Arc::new(OperationCalls::default());
        let execution = sentinel_app(Arc::clone(&calls))
            .execute(["--json".to_string(), actions::TOGGLE_MAIN.to_string()]);

        assert_eq!(execution.exit_code, EXIT_USAGE);
        assert!(execution.stdout.is_empty());
        assert!(execution.stderr.contains("does not support --json"));
        assert!(calls.runtime.lock().unwrap().is_empty());
        assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
    }
}
