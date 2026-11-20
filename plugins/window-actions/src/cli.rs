use std::process::ExitCode;
use std::sync::Arc;

use qol_headless::{Command, CommandResult, HeadlessApp};

use crate::action::WindowAction;
use crate::glide::Direction;
use crate::PLUGIN_ID;

const ACTIONS: [WindowAction; 13] = [
    WindowAction::SnapLeft,
    WindowAction::SnapRight,
    WindowAction::SnapBottom,
    WindowAction::Maximize,
    WindowAction::Minimize,
    WindowAction::Restore,
    WindowAction::Center,
    WindowAction::MoveMonitorLeft,
    WindowAction::MoveMonitorRight,
    WindowAction::Glide(Direction::Left),
    WindowAction::Glide(Direction::Right),
    WindowAction::Glide(Direction::Up),
    WindowAction::Glide(Direction::Down),
];

pub(crate) fn parse_action(id: &str) -> Option<WindowAction> {
    match id {
        "snap-left" => Some(WindowAction::SnapLeft),
        "snap-right" => Some(WindowAction::SnapRight),
        "snap-bottom" => Some(WindowAction::SnapBottom),
        "maximize" => Some(WindowAction::Maximize),
        "minimize" => Some(WindowAction::Minimize),
        "restore" => Some(WindowAction::Restore),
        "center" => Some(WindowAction::Center),
        "move-monitor-left" => Some(WindowAction::MoveMonitorLeft),
        "move-monitor-right" => Some(WindowAction::MoveMonitorRight),
        "glide-left" => Some(WindowAction::Glide(Direction::Left)),
        "glide-right" => Some(WindowAction::Glide(Direction::Right)),
        "glide-up" => Some(WindowAction::Glide(Direction::Up)),
        "glide-down" => Some(WindowAction::Glide(Direction::Down)),
        "print-work-areas" => Some(WindowAction::PrintWorkAreas),
        _ => None,
    }
}

fn about(action: WindowAction) -> &'static str {
    match action {
        WindowAction::SnapLeft => "Snap the focused window to the left.",
        WindowAction::SnapRight => "Snap the focused window to the right.",
        WindowAction::SnapBottom => "Snap the focused window to the bottom.",
        WindowAction::Maximize => "Maximize the focused window.",
        WindowAction::Minimize => "Minimize and remember the focused window.",
        WindowAction::Restore => "Restore the most recently minimized window.",
        WindowAction::Center => "Center and resize the focused window.",
        WindowAction::MoveMonitorLeft => "Move the focused window to the previous monitor.",
        WindowAction::MoveMonitorRight => "Move the focused window to the next monitor.",
        WindowAction::Glide(Direction::Left) => "Continuously move a window left.",
        WindowAction::Glide(Direction::Right) => "Continuously move a window right.",
        WindowAction::Glide(Direction::Up) => "Continuously move a window up.",
        WindowAction::Glide(Direction::Down) => "Continuously move a window down.",
        WindowAction::PrintWorkAreas => "Print each display's work area as x,y,w,h lines.",
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
    Action: Fn(WindowAction) -> CommandResult + Send + Sync + 'static,
{
    let daemon = Arc::new(daemon);
    let action = Arc::new(action);
    let mut app = HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Move, resize, minimize, restore, and continuously glide desktop windows.")
        .default_command(["daemon"])
        .command(daemon_command(daemon))
        .doctor_checks(crate::doctor::checks());

    for window_action in ACTIONS {
        app = app.command(action_command(window_action, Arc::clone(&action)));
    }
    for window_action in crate::platform::DIAGNOSTIC_ACTIONS {
        app = app.command(action_command(*window_action, Arc::clone(&action)));
    }
    app.command(settings_command())
}

fn daemon_command<Daemon>(handler: Arc<Daemon>) -> Command
where
    Daemon: Fn() -> CommandResult + Send + Sync + 'static,
{
    Command::new("daemon")
        .alias("run")
        .about("Run the daemon that owns continuous glide sessions.")
        .usage(format!("{PLUGIN_ID} daemon"))
        .detail("The legacy `run` command is an alias.")
        .output("Runtime diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero if the daemon listener cannot start.")
        .run_result(move |_| Ok(handler()))
}

fn action_command<Action>(action: WindowAction, handler: Arc<Action>) -> Command
where
    Action: Fn(WindowAction) -> CommandResult + Send + Sync + 'static,
{
    let name = action.id();
    let mut command = Command::new(name)
        .about(about(action))
        .usage(format!("{PLUGIN_ID} {name}"))
        .output("No stdout on success; diagnostics are written to stderr.")
        .exit_behavior("Exits non-zero if the platform cannot perform the action.");
    if matches!(action, WindowAction::Glide(_)) {
        command = command.detail(
            "qol-tray sends start, heartbeat, and stop phases to the running daemon for this action.",
        );
    }
    command.run_result(move |_| Ok(handler(action)))
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the Window Actions settings page in qol-tray.")
        .usage(format!("{PLUGIN_ID} settings"))
        .output("No stdout on success; opens the settings URL through the platform launcher.")
        .exit_behavior("Exits non-zero if the settings URL cannot be launched.")
        .run_result(move |_| {
            Ok(result_for(
                qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)
                    .map_err(|error| format!("failed to open settings URL: {error}")),
            ))
        })
}

fn run_daemon() -> CommandResult {
    result_for(crate::app::run())
}

fn execute_action(action: WindowAction) -> CommandResult {
    let store = crate::restore::state_store::FileMinimizedStateStore::new(
        crate::platform::state_file_path(),
    );
    let config = crate::config::load_config();
    let timer = crate::diagnostics::ActionTimer::start(action.id());
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
                    .push(action.id().to_string());
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

        for action in ACTIONS {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute([action.id().to_string()]);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "action={}", action.id());
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
            assert_eq!(
                calls.actions.lock().unwrap().as_slice(),
                [action.id()],
                "action={}",
                action.id()
            );
        }
    }

    #[test]
    fn parse_action_round_trips_every_known_id() {
        let ids = [
            "snap-left",
            "snap-right",
            "snap-bottom",
            "maximize",
            "minimize",
            "restore",
            "center",
            "move-monitor-left",
            "move-monitor-right",
            "glide-left",
            "glide-right",
            "glide-up",
            "glide-down",
            "print-work-areas",
        ];
        for id in ids {
            assert_eq!(parse_action(id).map(WindowAction::id), Some(id), "{id}");
        }
        for action in ACTIONS {
            assert_eq!(parse_action(action.id()), Some(action));
        }
    }

    #[test]
    fn parse_action_maps_glide_ids_to_directions() {
        let cases = [
            ("glide-left", Some(WindowAction::Glide(Direction::Left))),
            ("glide-right", Some(WindowAction::Glide(Direction::Right))),
            ("glide-up", Some(WindowAction::Glide(Direction::Up))),
            ("glide-down", Some(WindowAction::Glide(Direction::Down))),
            ("snap-left", Some(WindowAction::SnapLeft)),
            ("settings", None),
            ("nope", None),
        ];
        for (id, expected) in cases {
            assert_eq!(parse_action(id), expected, "{id}");
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
        for command in ["daemon", "snap-left", "glide-left", "settings", "doctor"] {
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
