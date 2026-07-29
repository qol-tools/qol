use std::process::ExitCode;
use std::sync::Arc;

use qol_headless::{Command, CommandResult, HeadlessApp};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "window-actions";
const ACTIONS: [ActionSpec; 13] = [
    ActionSpec::ordinary("snap-left", "Snap the focused window to the left."),
    ActionSpec::ordinary("snap-right", "Snap the focused window to the right."),
    ActionSpec::ordinary("snap-bottom", "Snap the focused window to the bottom."),
    ActionSpec::ordinary("maximize", "Maximize the focused window."),
    ActionSpec::ordinary("minimize", "Minimize and remember the focused window."),
    ActionSpec::ordinary("restore", "Restore the most recently minimized window."),
    ActionSpec::ordinary("center", "Center and resize the focused window."),
    ActionSpec::ordinary(
        "move-monitor-left",
        "Move the focused window to the previous monitor.",
    ),
    ActionSpec::ordinary(
        "move-monitor-right",
        "Move the focused window to the next monitor.",
    ),
    ActionSpec::continuous("glide-left", "Continuously move a window left."),
    ActionSpec::continuous("glide-right", "Continuously move a window right."),
    ActionSpec::continuous("glide-up", "Continuously move a window up."),
    ActionSpec::continuous("glide-down", "Continuously move a window down."),
];

#[derive(Clone, Copy)]
pub(crate) struct ActionSpec {
    name: &'static str,
    about: &'static str,
    continuous: bool,
}

impl ActionSpec {
    pub(crate) const fn ordinary(name: &'static str, about: &'static str) -> Self {
        Self {
            name,
            about,
            continuous: false,
        }
    }

    const fn continuous(name: &'static str, about: &'static str) -> Self {
        Self {
            name,
            about,
            continuous: true,
        }
    }
}

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(run_daemon, execute_action)
}

fn app_with_handlers<Daemon, Action>(daemon: Daemon, action: Action) -> HeadlessApp
where
    Daemon: Fn() -> CommandResult + Send + Sync + 'static,
    Action: Fn(&str) -> CommandResult + Send + Sync + 'static,
{
    let daemon = Arc::new(daemon);
    let action = Arc::new(action);
    let mut app = HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Move, resize, minimize, restore, and continuously glide desktop windows.")
        .default_command(["daemon"])
        .command(daemon_command(daemon))
        .doctor_checks(crate::doctor::checks());

    for spec in ACTIONS {
        app = app.command(action_command(spec, Arc::clone(&action)));
    }
    for spec in crate::platform::DIAGNOSTIC_ACTIONS {
        app = app.command(action_command(*spec, Arc::clone(&action)));
    }
    app
}

fn daemon_command<Daemon>(handler: Arc<Daemon>) -> Command
where
    Daemon: Fn() -> CommandResult + Send + Sync + 'static,
{
    Command::new("daemon")
        .alias("run")
        .about("Run the daemon that owns continuous glide sessions.")
        .usage(format!("{BINARY_NAME} daemon"))
        .detail("The legacy `run` command is an alias.")
        .output("Runtime diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero if the daemon listener cannot start.")
        .run_result(move |_| Ok(handler()))
}

fn action_command<Action>(spec: ActionSpec, handler: Arc<Action>) -> Command
where
    Action: Fn(&str) -> CommandResult + Send + Sync + 'static,
{
    let mut command = Command::new(spec.name)
        .about(spec.about)
        .usage(format!("{BINARY_NAME} {}", spec.name))
        .output("No stdout on success; diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero if the platform cannot perform the action.");
    if spec.continuous {
        command = command.detail(
            "qol-tray sends start, heartbeat, and stop phases to the running daemon for this action.",
        );
    }
    command.run_result(move |_| Ok(handler(spec.name)))
}

fn run_daemon() -> CommandResult {
    result_for(crate::app::run())
}

fn execute_action(action: &str) -> CommandResult {
    let store = crate::restore::state_store::FileMinimizedStateStore::new(
        crate::platform::state_file_path(),
    );
    let config = crate::config::load_config();
    let timer = crate::diagnostics::ActionTimer::start(action);
    let result = crate::platform::execute_action(action, &store, &config);
    timer.finish(&result);
    result_for(result)
}

fn result_for(result: Result<(), String>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::new("", format!("{error}\n"), 1),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use qol_headless::{DoctorReport, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        daemon: AtomicUsize,
        actions: Mutex<Vec<String>>,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let daemon_calls = Arc::clone(&calls);
        let action_calls = Arc::clone(&calls);
        app_with_handlers(
            move || {
                daemon_calls.daemon.fetch_add(1, Ordering::SeqCst);
                CommandResult::success("")
            },
            move |action| {
                action_calls
                    .actions
                    .lock()
                    .unwrap()
                    .push(action.to_string());
                CommandResult::success("")
            },
        )
    }

    #[test]
    fn daemon_aliases_and_actions_preserve_the_operational_routes() {
        for args in [
            Vec::<String>::new(),
            vec!["daemon".into()],
            vec!["run".into()],
        ] {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute(args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS);
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 1);
            assert!(calls.actions.lock().unwrap().is_empty());
        }

        for spec in ACTIONS {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute([spec.name.to_string()]);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "action={}", spec.name);
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
            assert_eq!(
                calls.actions.lock().unwrap().as_slice(),
                [spec.name],
                "action={}",
                spec.name
            );
        }
    }

    #[test]
    fn manifest_actions_have_contextual_cli_help() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        for action in manifest.executable_actions() {
            let args = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let command = args.first().expect("runtime args must name a command");
            let execution = app().execute(["help".to_string(), command.clone()]);

            assert_eq!(
                execution.exit_code, EXIT_SUCCESS,
                "action={} stderr={}",
                action.id, execution.stderr
            );
        }
    }

    #[test]
    fn contextual_help_is_equivalent_in_both_positions() {
        for command in ["daemon", "snap-left", "glide-left", "doctor"] {
            let first = app().execute(["help".to_string(), command.to_string()]);
            let final_token = app().execute([command.to_string(), "help".to_string()]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command={command}");
            assert_eq!(first.stdout, final_token.stdout, "command={command}");
            assert!(first.stdout.contains("Output:"), "command={command}");
            assert!(first.stdout.contains("Exit:"), "command={command}");
        }
    }

    #[test]
    fn doctor_json_is_stable_in_both_global_flag_positions() {
        let before = app().execute(["--json".to_string(), "doctor".to_string()]);
        let after = app().execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        assert!(before.stderr.is_empty());
        assert!(after.stderr.is_empty());
        let report: DoctorReport =
            serde_json::from_str(&before.stdout).expect("doctor output must be valid JSON");
        assert_eq!(report.plugin_id, PLUGIN_ID);
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            [
                "platform_supported",
                "config_readable",
                "required_binaries",
                "permissions",
                "restore_state",
            ]
        );
        assert!(report.checks.iter().all(|check| !check.message.is_empty()));
    }

    #[test]
    fn doctor_help_lists_the_live_read_only_contract() {
        let execution = app().execute(["doctor".to_string(), "help".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        for check in crate::doctor::check_ids() {
            assert!(execution.stdout.contains(check), "missing check {check}");
        }
        assert!(execution.stdout.contains("Run read-only health checks."));
    }

    #[test]
    fn doctor_and_help_never_invoke_action_or_daemon_handlers() {
        let cases = [
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["help", "snap-left"],
            vec!["glide-left", "help"],
            vec!["daemon", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args={args:?}");
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0, "args={args:?}");
            assert!(calls.actions.lock().unwrap().is_empty(), "args={args:?}");
        }
    }

    #[test]
    fn unsupported_json_is_rejected_before_an_action_runs() {
        for args in [["snap-left", "--json"], ["--json", "snap-left"]] {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.into_iter().map(str::to_string));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args={args:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "args={args:?}"
            );
            assert!(calls.actions.lock().unwrap().is_empty(), "args={args:?}");
        }
    }
}
