use std::process::ExitCode;
use std::sync::Arc;

use qol_headless::{Command, CommandResult, DoctorCheck, HeadlessApp};

use crate::config::PLUGIN_ID;

const BINARY_NAME: &str = "plugin-os-themes";

pub(crate) fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(run_daemon, run_action, crate::doctor::checks())
}

fn app_with_handlers<Daemon, Action>(
    daemon: Daemon,
    action: Action,
    doctor_checks: Vec<DoctorCheck>,
) -> HeadlessApp
where
    Daemon: Fn() -> CommandResult + Send + Sync + 'static,
    Action: Fn(&str) -> CommandResult + Send + Sync + 'static,
{
    let daemon = Arc::new(daemon);
    let action = Arc::new(action);

    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Run cursor effects and control OS-wide light and dark themes.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .about("Run the cursor-effects daemon.")
                .usage(format!("{BINARY_NAME} run"))
                .detail("Running the binary without arguments selects this command.")
                .output("Lifecycle diagnostics are written to stderr.")
                .exit_behavior("Runs until stopped; exits non-zero if the daemon cannot start.")
                .run_result(move |_| Ok(daemon())),
        )
        .command(action_command(
            "toggle_theme",
            "Toggle the desktop between light and dark themes.",
            "No stdout on success; the applied theme is written to stderr.",
            Arc::clone(&action),
        ))
        .command(action_command(
            "settings",
            "Open the OS Themes settings surface.",
            "No stdout on success; launch failures are written to stderr.",
            Arc::clone(&action),
        ))
        .command(action_command(
            "kill",
            "Ask the running cursor-effects daemon to stop.",
            "No output on success.",
            action,
        ))
        .doctor_checks(doctor_checks)
}

fn action_command<Action>(
    name: &'static str,
    about: &'static str,
    output: &'static str,
    handler: Arc<Action>,
) -> Command
where
    Action: Fn(&str) -> CommandResult + Send + Sync + 'static,
{
    Command::new(name)
        .about(about)
        .usage(format!("{BINARY_NAME} {name}"))
        .output(output)
        .exit_behavior("Exits non-zero if the requested operation fails.")
        .run_result(move |_| Ok(handler(name)))
}

fn run_daemon() -> CommandResult {
    result_for(crate::app::run())
}

fn run_action(action: &str) -> CommandResult {
    match action {
        "toggle_theme" => toggle_theme(),
        "settings" => result_for(crate::app::open_settings()),
        "kill" => result_for(crate::app::kill()),
        action => CommandResult::runtime_error(format!("Unknown action: {action}")),
    }
}

fn toggle_theme() -> CommandResult {
    match crate::app::toggle_theme() {
        Ok(scheme) => CommandResult::new(
            "",
            format!("[os-themes] applied {scheme:?} theme\n"),
            qol_headless::EXIT_SUCCESS,
        ),
        Err(error) => CommandResult::runtime_error(format!("{error:#}")),
    }
}

fn result_for(result: anyhow::Result<()>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::runtime_error(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use qol_headless::{DoctorCheckResult, DoctorReport, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::{ActionType, PluginManifest};

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
                    .expect("action calls poisoned")
                    .push(action.to_string());
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
    fn legacy_routes_reach_only_the_selected_operation() {
        for args in [
            Vec::<String>::new(),
            vec!["run".to_string()],
            vec!["run".to_string(), "legacy-tail".to_string()],
        ] {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute(args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS);
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 1);
            assert!(calls
                .actions
                .lock()
                .expect("action calls poisoned")
                .is_empty());
        }

        for action in ["toggle_theme", "settings", "kill"] {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls))
                .execute([action.to_string(), "legacy-tail".to_string()]);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "action={action}");
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0);
            assert_eq!(
                calls
                    .actions
                    .lock()
                    .expect("action calls poisoned")
                    .as_slice(),
                [action]
            );
        }
    }

    #[test]
    fn manifest_actions_and_arguments_are_unchanged_and_have_help() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let expected = BTreeMap::from([
            ("run", (ActionType::Run, vec!["run"])),
            ("settings", (ActionType::Settings, vec!["settings"])),
            ("toggle_theme", (ActionType::Run, vec!["toggle_theme"])),
        ]);

        assert_eq!(manifest.actions.len(), expected.len());
        for action in manifest.executable_actions() {
            let (kind, args) = expected
                .get(action.id.as_str())
                .unwrap_or_else(|| panic!("unexpected manifest action {}", action.id));
            assert_eq!(&action.kind, kind, "action={}", action.id);
            assert_eq!(
                manifest.catalog_runtime_args(&action.id),
                Some(args.iter().map(|arg| (*arg).to_string()).collect()),
                "action={}",
                action.id
            );

            let command = args.first().expect("manifest action args missing");
            let execution = app().execute(["help".to_string(), (*command).to_string()]);
            assert_eq!(
                execution.exit_code, EXIT_SUCCESS,
                "action={} stderr={}",
                action.id, execution.stderr
            );
        }
    }

    #[test]
    fn contextual_help_works_in_both_positions() {
        for command in ["run", "toggle_theme", "settings", "kill", "doctor"] {
            let first = app().execute(["help".to_string(), command.to_string()]);
            let final_token = app().execute([command.to_string(), "help".to_string()]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command={command}");
            assert_eq!(first.stdout, final_token.stdout, "command={command}");
            assert!(first.stdout.contains("Output:"), "command={command}");
            assert!(first.stdout.contains("Exit:"), "command={command}");
        }
    }

    #[test]
    fn doctor_json_has_the_shared_schema_in_both_flag_positions() {
        let before = sentinel_app(Arc::new(OperationCalls::default()))
            .execute(["--json".to_string(), "doctor".to_string()]);
        let after = sentinel_app(Arc::new(OperationCalls::default()))
            .execute(["doctor".to_string(), "--json".to_string()]);

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
            ["sentinel"]
        );
        assert!(report.checks.iter().all(|check| !check.message.is_empty()));
    }

    #[test]
    fn help_and_doctor_never_invoke_operational_handlers() {
        let cases = [
            vec!["help"],
            vec!["--help"],
            vec!["help", "run"],
            vec!["run", "help"],
            vec!["help", "toggle_theme"],
            vec!["toggle_theme", "help"],
            vec!["help", "settings"],
            vec!["settings", "help"],
            vec!["help", "kill"],
            vec!["kill", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args={args:?}");
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0, "args={args:?}");
            assert!(
                calls
                    .actions
                    .lock()
                    .expect("action calls poisoned")
                    .is_empty(),
                "args={args:?}"
            );
        }
    }

    #[test]
    fn unsupported_routes_and_arguments_are_rejected_before_operations() {
        let cases = [
            vec!["daemon"],
            vec!["--json"],
            vec!["--json", "run"],
            vec!["run", "--json"],
            vec!["--json", "toggle_theme"],
            vec!["toggle_theme", "--json"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args={args:?}");
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0, "args={args:?}");
            assert!(
                calls
                    .actions
                    .lock()
                    .expect("action calls poisoned")
                    .is_empty(),
                "args={args:?}"
            );
        }
    }
}
