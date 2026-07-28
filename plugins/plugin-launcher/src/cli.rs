use std::process::ExitCode;

use qol_headless::{Command, CommandResult, DoctorCheck, HeadlessApp};

use crate::ui::run::StartupIntent;

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "launcher";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Run,
    Show,
    Settings,
    Kill,
}

trait Operations: Clone + Send + Sync + 'static {
    fn run(&self, intent: StartupIntent) -> CommandResult;
    fn send_show(&self) -> bool;
    fn send_kill(&self);
    fn open_settings(&self) -> CommandResult;
}

#[derive(Clone, Copy)]
struct ProductionOperations;

impl Operations for ProductionOperations {
    fn run(&self, intent: StartupIntent) -> CommandResult {
        crate::ui::run::run(intent);
        CommandResult::success("")
    }

    fn send_show(&self) -> bool {
        crate::app::send_show()
    }

    fn send_kill(&self) {
        crate::app::send_kill();
    }

    fn open_settings(&self) -> CommandResult {
        match crate::qol::open() {
            Ok(()) => CommandResult::success(""),
            Err(error) => CommandResult::new(
                "",
                format!("[launcher] failed to open settings page: {error}\n"),
                qol_headless::EXIT_SUCCESS,
            ),
        }
    }
}

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(normalize_legacy_argv(args))
}

fn app() -> HeadlessApp {
    app_with_operations(ProductionOperations, crate::doctor::checks())
}

fn app_with_operations<O>(operations: O, doctor_checks: Vec<DoctorCheck>) -> HeadlessApp
where
    O: Operations,
{
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Search installed applications and files from a retained native launcher.")
        .default_command(["run"])
        .command(operation_command(
            "run",
            Operation::Run,
            "Run the retained launcher daemon with its window initially hidden.",
            "launcher run",
            "Lifecycle diagnostics are written to stderr.",
            operations.clone(),
        ))
        .command(operation_command(
            "--show",
            Operation::Show,
            "Show the retained launcher, starting its daemon visibly when needed.",
            "launcher --show",
            "No stdout; lifecycle diagnostics are written to stderr.",
            operations.clone(),
        ))
        .command(operation_command(
            "--settings",
            Operation::Settings,
            "Open the Launcher settings surface.",
            "launcher --settings",
            "No stdout; launch failures are written to stderr.",
            operations.clone(),
        ))
        .command(operation_command(
            "--kill",
            Operation::Kill,
            "Ask the retained launcher daemon to stop.",
            "launcher --kill",
            "No output on success.",
            operations,
        ))
        .doctor_checks(doctor_checks)
}

fn operation_command<O>(
    name: &'static str,
    requested: Operation,
    about: &'static str,
    usage: &'static str,
    output: &'static str,
    operations: O,
) -> Command
where
    O: Operations,
{
    Command::new(name)
        .about(about)
        .usage(usage)
        .detail(
            "Legacy trailing arguments are ignored; legacy operation flags keep fixed priority.",
        )
        .output(output)
        .exit_behavior("Preserves the legacy best-effort zero exit behavior.")
        .run_result(move |_| Ok(route_operation(&operations, requested)))
}

fn route_operation(operations: &impl Operations, requested: Operation) -> CommandResult {
    match requested {
        Operation::Run => operations.run(StartupIntent::Hidden),
        Operation::Show => {
            if operations.send_show() {
                return CommandResult::success("");
            }
            operations.run(StartupIntent::Visible)
        }
        Operation::Settings => operations.open_settings(),
        Operation::Kill => {
            operations.send_kill();
            CommandResult::success("")
        }
    }
}

fn normalize_legacy_argv(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let args = args.into_iter().collect::<Vec<_>>();
    let has_help = args.iter().any(|arg| arg == "help" || arg == "--help");
    let has_json = args.iter().any(|arg| arg == "--json");
    let starts_doctor = args.first().is_some_and(|arg| arg == "doctor");
    if has_help || has_json || starts_doctor || args.is_empty() {
        return args;
    }
    for flag in ["--kill", "--settings", "--show"] {
        if args.iter().any(|arg| arg == flag) {
            return vec![flag.to_string()];
        }
    }
    vec!["run".to_string()]
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use qol_headless::{DoctorCheckResult, DoctorReport, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        runs: Mutex<Vec<StartupIntent>>,
        show_signals: AtomicUsize,
        kill_signals: AtomicUsize,
        settings: AtomicUsize,
        daemon_available: bool,
    }

    #[derive(Clone)]
    struct SentinelOperations {
        calls: Arc<OperationCalls>,
    }

    impl Operations for SentinelOperations {
        fn run(&self, intent: StartupIntent) -> CommandResult {
            self.calls.runs.lock().unwrap().push(intent);
            CommandResult::success("")
        }

        fn send_show(&self) -> bool {
            self.calls.show_signals.fetch_add(1, Ordering::SeqCst);
            self.calls.daemon_available
        }

        fn send_kill(&self) {
            self.calls.kill_signals.fetch_add(1, Ordering::SeqCst);
        }

        fn open_settings(&self) -> CommandResult {
            self.calls.settings.fetch_add(1, Ordering::SeqCst);
            CommandResult::success("")
        }
    }

    fn sentinel(daemon_available: bool) -> (HeadlessApp, Arc<OperationCalls>) {
        let calls = Arc::new(OperationCalls {
            daemon_available,
            ..OperationCalls::default()
        });
        let operations = SentinelOperations {
            calls: Arc::clone(&calls),
        };
        (
            app_with_operations(operations, sentinel_doctor_checks()),
            calls,
        )
    }

    fn sentinel_doctor_checks() -> Vec<DoctorCheck> {
        crate::doctor::check_ids()
            .iter()
            .map(|id| {
                let id = *id;
                DoctorCheck::new(id, format!("Inspect {id} read-only."), move || {
                    Ok(DoctorCheckResult::ok(id, format!("{id} is healthy")))
                })
            })
            .collect()
    }

    fn assert_no_operations(calls: &OperationCalls, args: &[&str]) {
        assert!(calls.runs.lock().unwrap().is_empty(), "{args:?}");
        assert_eq!(calls.show_signals.load(Ordering::SeqCst), 0, "{args:?}");
        assert_eq!(calls.kill_signals.load(Ordering::SeqCst), 0, "{args:?}");
        assert_eq!(calls.settings.load(Ordering::SeqCst), 0, "{args:?}");
    }

    fn execute(
        app: &HeadlessApp,
        args: impl IntoIterator<Item = String>,
    ) -> qol_headless::Execution {
        app.execute(normalize_legacy_argv(args))
    }

    #[test]
    fn no_arguments_run_the_daemon_hidden() {
        let (app, calls) = sentinel(false);

        let execution = execute(&app, Vec::<String>::new());

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(
            calls.runs.lock().unwrap().as_slice(),
            [StartupIntent::Hidden]
        );
        assert_eq!(calls.show_signals.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn show_signals_the_daemon_or_starts_visible_fallback() {
        let (app, calls) = sentinel(true);
        let execution = execute(&app, ["--show".to_string(), "legacy-tail".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.show_signals.load(Ordering::SeqCst), 1);
        assert!(calls.runs.lock().unwrap().is_empty());

        let (app, calls) = sentinel(false);
        let execution = execute(&app, ["--show".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.show_signals.load(Ordering::SeqCst), 1);
        assert_eq!(
            calls.runs.lock().unwrap().as_slice(),
            [StartupIntent::Visible]
        );
    }

    #[test]
    fn settings_kill_and_flag_priority_match_the_legacy_routes() {
        let cases = [
            (vec!["--settings"], Operation::Settings),
            (vec!["--kill"], Operation::Kill),
            (vec!["--show", "--settings"], Operation::Settings),
            (vec!["--settings", "--kill"], Operation::Kill),
            (vec!["run", "--show", "--kill"], Operation::Kill),
            (vec!["legacy-prefix", "--show", "tail"], Operation::Show),
        ];

        for (args, expected) in cases {
            let (app, calls) = sentinel(false);
            let execution = execute(&app, args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_eq!(
                calls.settings.load(Ordering::SeqCst),
                usize::from(expected == Operation::Settings),
                "{args:?}"
            );
            assert_eq!(
                calls.kill_signals.load(Ordering::SeqCst),
                usize::from(expected == Operation::Kill),
                "{args:?}"
            );
            assert_eq!(
                calls.show_signals.load(Ordering::SeqCst),
                usize::from(expected == Operation::Show),
                "{args:?}"
            );
            let expected_runs: &[StartupIntent] = if expected == Operation::Show {
                &[StartupIntent::Visible]
            } else {
                &[]
            };
            assert_eq!(
                calls.runs.lock().unwrap().as_slice(),
                expected_runs,
                "{args:?}"
            );
        }
    }

    #[test]
    fn ignored_legacy_arguments_still_select_hidden_daemon_startup() {
        for args in [vec!["legacy"], vec!["legacy", "tail"], vec!["run", "tail"]] {
            let (app, calls) = sentinel(false);
            let execution = execute(&app, args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_eq!(
                calls.runs.lock().unwrap().as_slice(),
                [StartupIntent::Hidden],
                "{args:?}"
            );
            assert_eq!(calls.show_signals.load(Ordering::SeqCst), 0, "{args:?}");
        }
    }

    #[test]
    fn manifest_action_arguments_are_unchanged_and_have_help() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let expected = [("open", "--show"), ("settings", "--settings")];

        for (action, command) in expected {
            assert_eq!(
                manifest.catalog_runtime_args(action),
                Some(vec![command.to_string()]),
                "action={action}"
            );
            let execution = execute(&app(), ["help".to_string(), command.to_string()]);
            assert_eq!(execution.exit_code, EXIT_SUCCESS, "action={action}");
        }
    }

    #[test]
    fn contextual_help_is_equivalent_in_both_positions() {
        for command in ["run", "--show", "--settings", "--kill", "doctor"] {
            let first = execute(&app(), ["help".to_string(), command.to_string()]);
            let final_token = execute(&app(), [command.to_string(), "help".to_string()]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command={command}");
            assert_eq!(first.stdout, final_token.stdout, "command={command}");
            assert!(first.stdout.contains("Output:"), "command={command}");
            assert!(first.stdout.contains("Exit:"), "command={command}");
        }
    }

    #[test]
    fn doctor_json_uses_the_shared_schema_in_both_flag_positions() {
        let (app, calls) = sentinel(false);
        let before = execute(&app, ["--json".to_string(), "doctor".to_string()]);
        let after = execute(&app, ["doctor".to_string(), "--json".to_string()]);

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
            crate::doctor::check_ids()
        );
        assert_no_operations(&calls, &["doctor"]);
    }

    #[test]
    fn help_and_doctor_never_enter_gpui_ipc_or_settings_paths() {
        let cases = [
            vec!["help"],
            vec!["--help"],
            vec!["help", "run"],
            vec!["--show", "help"],
            vec!["help", "--settings"],
            vec!["--kill", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
        ];

        for args in cases {
            let (app, calls) = sentinel(false);
            let execution = execute(&app, args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_no_operations(&calls, &args);
        }
    }

    #[test]
    fn invalid_help_doctor_and_json_requests_are_rejected_before_operations() {
        let cases = [
            vec!["--json"],
            vec!["--json", "run"],
            vec!["run", "--json"],
            vec!["--show", "--json"],
            vec!["--show", "help", "tail"],
            vec!["doctor", "--show"],
        ];

        for args in cases {
            let (app, calls) = sentinel(false);
            let execution = execute(&app, args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "{args:?}");
            assert_no_operations(&calls, &args);
        }
    }
}
