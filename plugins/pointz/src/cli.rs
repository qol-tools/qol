use std::process::ExitCode;
use std::sync::Arc;

use qol_headless::{Command, CommandResult, DoctorCheck, HeadlessApp};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "pointzerver";
const DIRECT_ACTIONS: [(&str, &str); 5] = [
    ("settings", "Open the PointZ settings surface."),
    (
        "begin_pairing",
        "Allow the next compatible discovery request to pair.",
    ),
    ("ping", "Send a liveness request to the running daemon."),
    (
        "connection_status",
        "Request current daemon connection status.",
    ),
    (
        "connection_info",
        "Request hostname, IP address, and port metadata.",
    ),
];

trait Operations: Clone + Send + Sync + 'static {
    fn run_server(&self) -> CommandResult;
    fn send_action(&self, action: &str) -> bool;
    fn send_kill(&self) -> bool;
    fn open_settings(&self);
}

#[derive(Clone, Copy)]
struct ProductionOperations;

impl Operations for ProductionOperations {
    fn run_server(&self) -> CommandResult {
        crate::app::run();
        CommandResult::success("")
    }

    fn send_action(&self, action: &str) -> bool {
        crate::app::daemon::send_action(action)
    }

    fn send_kill(&self) -> bool {
        crate::app::daemon::send_kill()
    }

    fn open_settings(&self) {
        crate::qol::open_settings();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LegacyRoute<'a> {
    Server,
    Kill,
    Action(&'a str),
}

pub(crate) fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args = args.into_iter().collect::<Vec<_>>();
    app_for_args(&args, ProductionOperations, crate::doctor::checks()).run(args)
}

fn app_for_args<O>(args: &[String], operations: O, doctor_checks: Vec<DoctorCheck>) -> HeadlessApp
where
    O: Operations,
{
    let args = Arc::new(args.to_vec());
    let app = HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Run and control the PointZ remote-input server.")
        .default_command(["server"])
        .command(server_command(operations.clone(), Arc::clone(&args)))
        .command(action_group(operations.clone(), Arc::clone(&args)))
        .command(legacy_command(
            "kill",
            "Ask the running PointZ daemon to stop.",
            operations.clone(),
            Arc::clone(&args),
        ));
    let app = DIRECT_ACTIONS.into_iter().fold(app, |app, (name, about)| {
        app.command(legacy_command(
            name,
            about,
            operations.clone(),
            Arc::clone(&args),
        ))
    });
    app.fallback_command(legacy_command(
        "legacy-action",
        "Forward an arbitrary legacy action to the running PointZ daemon.",
        operations,
        args,
    ))
    .doctor_checks(doctor_checks)
}

fn server_command<O>(operations: O, args: Arc<Vec<String>>) -> Command
where
    O: Operations,
{
    Command::new("server")
        .about("Run the PointZ services when selected by the no-argument default.")
        .usage(BINARY_NAME)
        .detail("Running the binary without arguments starts the server.")
        .detail("An explicit server token retains legacy direct-action forwarding.")
        .output("Lifecycle or daemon-delivery diagnostics are written to stderr.")
        .exit_behavior("Runs until stopped when selected by the no-argument default.")
        .run_result(move |_| Ok(run_legacy(&operations, &args)))
}

fn action_group<O>(operations: O, args: Arc<Vec<String>>) -> Command
where
    O: Operations,
{
    let command = legacy_command(
        "action",
        "Send a named action to the running PointZ daemon.",
        operations.clone(),
        Arc::clone(&args),
    )
    .alias("--action")
    .usage(format!("{BINARY_NAME} --action <name>"))
    .detail("Legacy routing ignores tokens after the selected action.");
    let command = command.subcommand(legacy_command(
        "kill",
        "Ask the running PointZ daemon to stop.",
        operations.clone(),
        Arc::clone(&args),
    ));
    DIRECT_ACTIONS
        .into_iter()
        .fold(command, |command, (name, about)| {
            command.subcommand(legacy_command(
                name,
                about,
                operations.clone(),
                Arc::clone(&args),
            ))
        })
}

fn legacy_command<O>(
    name: impl Into<String>,
    about: impl Into<String>,
    operations: O,
    args: Arc<Vec<String>>,
) -> Command
where
    O: Operations,
{
    let name = name.into();
    Command::new(name.clone())
        .about(about)
        .usage(format!("{BINARY_NAME} {name}"))
        .output("The selected legacy route writes its result to stderr.")
        .exit_behavior("Exits zero whether or not a daemon is currently running.")
        .run_result(move |_| Ok(run_legacy(&operations, &args)))
}

fn legacy_route(args: &[String]) -> LegacyRoute<'_> {
    if args.iter().any(|argument| argument == "kill") {
        return LegacyRoute::Kill;
    }
    if let Some(position) = args.iter().position(|argument| argument == "--action") {
        return args
            .get(position + 1)
            .map(|action| LegacyRoute::Action(action))
            .unwrap_or(LegacyRoute::Server);
    }
    args.first()
        .map(|action| LegacyRoute::Action(action))
        .unwrap_or(LegacyRoute::Server)
}

fn run_legacy(operations: &impl Operations, args: &[String]) -> CommandResult {
    match legacy_route(args) {
        LegacyRoute::Server => operations.run_server(),
        LegacyRoute::Kill => route_kill(operations),
        LegacyRoute::Action(action) => route_action(operations, action),
    }
}

fn route_action(operations: &impl Operations, action: &str) -> CommandResult {
    if operations.send_action(action) {
        return CommandResult::new(
            "",
            format!("[pointz] action '{action}' sent\n"),
            qol_headless::EXIT_SUCCESS,
        );
    }
    if action == "settings" {
        operations.open_settings();
    }
    CommandResult::new(
        "",
        "[pointz] no daemon running, handling locally\n",
        qol_headless::EXIT_SUCCESS,
    )
}

fn route_kill(operations: &impl Operations) -> CommandResult {
    let message = if operations.send_kill() {
        "[pointz] kill sent\n"
    } else {
        "[pointz] no daemon running\n"
    };
    CommandResult::new("", message, qol_headless::EXIT_SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use qol_headless::{DoctorReport, Execution, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        server: AtomicUsize,
        daemon_actions: Mutex<Vec<String>>,
        kill: AtomicUsize,
        settings: AtomicUsize,
        secret_load_or_create: AtomicUsize,
        input_initialized: AtomicUsize,
        udp_bound: AtomicUsize,
    }

    #[derive(Clone)]
    struct SentinelOperations {
        calls: Arc<OperationCalls>,
        daemon_available: Arc<AtomicBool>,
    }

    impl Operations for SentinelOperations {
        fn run_server(&self) -> CommandResult {
            self.calls.server.fetch_add(1, Ordering::SeqCst);
            self.calls
                .secret_load_or_create
                .fetch_add(1, Ordering::SeqCst);
            self.calls.input_initialized.fetch_add(1, Ordering::SeqCst);
            self.calls.udp_bound.fetch_add(2, Ordering::SeqCst);
            CommandResult::success("")
        }

        fn send_action(&self, action: &str) -> bool {
            self.calls
                .daemon_actions
                .lock()
                .expect("daemon action calls poisoned")
                .push(action.to_string());
            self.daemon_available.load(Ordering::SeqCst)
        }

        fn send_kill(&self) -> bool {
            self.calls.kill.fetch_add(1, Ordering::SeqCst);
            self.daemon_available.load(Ordering::SeqCst)
        }

        fn open_settings(&self) {
            self.calls.settings.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn sentinel() -> (SentinelOperations, Arc<OperationCalls>, Arc<AtomicBool>) {
        let calls = Arc::new(OperationCalls::default());
        let daemon_available = Arc::new(AtomicBool::new(true));
        (
            SentinelOperations {
                calls: Arc::clone(&calls),
                daemon_available: Arc::clone(&daemon_available),
            },
            calls,
            daemon_available,
        )
    }

    fn execute(operations: SentinelOperations, args: &[&str]) -> Execution {
        let args = args
            .iter()
            .map(|argument| (*argument).to_string())
            .collect::<Vec<_>>();
        app_for_args(&args, operations, crate::doctor::checks()).execute(args)
    }

    fn assert_no_operations(calls: &OperationCalls, args: &[&str]) {
        assert_eq!(calls.server.load(Ordering::SeqCst), 0, "{args:?}");
        assert_eq!(calls.kill.load(Ordering::SeqCst), 0, "{args:?}");
        assert_eq!(calls.settings.load(Ordering::SeqCst), 0, "{args:?}");
        assert_eq!(
            calls.secret_load_or_create.load(Ordering::SeqCst),
            0,
            "{args:?}"
        );
        assert_eq!(
            calls.input_initialized.load(Ordering::SeqCst),
            0,
            "{args:?}"
        );
        assert_eq!(calls.udp_bound.load(Ordering::SeqCst), 0, "{args:?}");
        assert!(
            calls
                .daemon_actions
                .lock()
                .expect("daemon action calls poisoned")
                .is_empty(),
            "{args:?}"
        );
    }

    #[test]
    fn legacy_route_selection_preserves_every_precedence_rule() {
        let cases = [
            (vec![], LegacyRoute::Server),
            (vec!["--action"], LegacyRoute::Server),
            (vec!["prefix", "--action"], LegacyRoute::Server),
            (vec!["settings", "tail"], LegacyRoute::Action("settings")),
            (
                vec!["prefix", "--action", "settings", "tail"],
                LegacyRoute::Action("settings"),
            ),
            (
                vec!["--action", "first", "--action", "second"],
                LegacyRoute::Action("first"),
            ),
            (
                vec!["future_action", "tail"],
                LegacyRoute::Action("future_action"),
            ),
            (vec!["prefix", "kill", "tail"], LegacyRoute::Kill),
            (vec!["--action", "settings", "kill"], LegacyRoute::Kill),
        ];
        for (args, expected) in cases {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(legacy_route(&args), expected, "{args:?}");
        }
    }

    #[test]
    fn no_arguments_and_bare_action_still_run_the_full_server_path() {
        for args in [vec![], vec!["--action"], vec!["prefix", "--action"]] {
            let (operations, calls, _) = sentinel();

            let execution = execute(operations, &args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_eq!(calls.server.load(Ordering::SeqCst), 1, "{args:?}");
            assert_eq!(
                calls.secret_load_or_create.load(Ordering::SeqCst),
                1,
                "{args:?}"
            );
            assert_eq!(calls.input_initialized.load(Ordering::SeqCst), 1);
            assert_eq!(calls.udp_bound.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn arbitrary_direct_actions_and_ignored_tails_are_preserved() {
        let cases = [
            (vec!["future_action"], "future_action"),
            (vec!["future_action", "ignored", "tail"], "future_action"),
            (vec!["server"], "server"),
            (
                vec!["prefix", "--action", "future_action", "ignored"],
                "future_action",
            ),
        ];
        for (args, expected) in cases {
            let (operations, calls, _) = sentinel();

            let execution = execute(operations, &args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_eq!(
                calls
                    .daemon_actions
                    .lock()
                    .expect("daemon action calls poisoned")
                    .as_slice(),
                [expected],
                "{args:?}"
            );
            assert_eq!(calls.server.load(Ordering::SeqCst), 0, "{args:?}");
        }
    }

    #[test]
    fn daemon_recognized_actions_keep_direct_and_action_flag_routes() {
        for (name, _) in DIRECT_ACTIONS {
            for args in [vec![name, "ignored"], vec!["--action", name, "ignored"]] {
                let (operations, calls, _) = sentinel();

                let execution = execute(operations, &args);

                assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
                assert_eq!(
                    calls
                        .daemon_actions
                        .lock()
                        .expect("daemon action calls poisoned")
                        .as_slice(),
                    [name],
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn any_kill_token_keeps_global_precedence() {
        let cases = [
            vec!["kill"],
            vec!["settings", "kill", "ignored"],
            vec!["--action", "settings", "kill"],
            vec!["prefix", "--action", "future_action", "kill"],
        ];
        for args in cases {
            let (operations, calls, _) = sentinel();

            let execution = execute(operations, &args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_eq!(calls.kill.load(Ordering::SeqCst), 1, "{args:?}");
            assert!(
                calls
                    .daemon_actions
                    .lock()
                    .expect("daemon action calls poisoned")
                    .is_empty(),
                "{args:?}"
            );
        }
    }

    #[test]
    fn settings_falls_back_locally_only_when_daemon_delivery_fails() {
        let (operations, calls, daemon_available) = sentinel();
        daemon_available.store(false, Ordering::SeqCst);

        let execution = execute(operations, &["--action", "settings", "ignored"]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(calls.settings.load(Ordering::SeqCst), 1);
        assert_eq!(
            calls
                .daemon_actions
                .lock()
                .expect("daemon action calls poisoned")
                .as_slice(),
            ["settings"]
        );
    }

    #[test]
    fn manifest_action_has_contextual_help_in_both_positions() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let args = manifest
            .catalog_runtime_args("settings")
            .expect("settings runtime args missing");
        let mut first = vec!["help".to_string()];
        first.extend(args.iter().cloned());
        let mut final_token = args;
        final_token.push("help".to_string());
        let first_refs = first.iter().map(String::as_str).collect::<Vec<_>>();
        let final_refs = final_token.iter().map(String::as_str).collect::<Vec<_>>();
        let (operations, calls, _) = sentinel();

        let first = execute(operations.clone(), &first_refs);
        let final_token = execute(operations, &final_refs);

        assert_eq!(first.exit_code, EXIT_SUCCESS);
        assert_eq!(first.stdout, final_token.stdout);
        assert!(first.stdout.contains("Open the PointZ settings surface."));
        assert_no_operations(&calls, &["help", "--action", "settings"]);
    }

    #[test]
    fn contextual_help_is_real_for_cataloged_actions() {
        for command in [
            "server",
            "action",
            "settings",
            "begin_pairing",
            "ping",
            "connection_status",
            "connection_info",
            "kill",
            "doctor",
        ] {
            let (operations, calls, _) = sentinel();

            let first = execute(operations.clone(), &["help", command]);
            let final_token = execute(operations, &[command, "help"]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command={command}");
            assert_eq!(first.stdout, final_token.stdout, "command={command}");
            assert!(first.stdout.contains("Output:"), "command={command}");
            assert!(first.stdout.contains("Exit:"), "command={command}");
            assert_no_operations(&calls, &["help", command]);
        }
    }

    #[test]
    fn doctor_json_uses_the_shared_schema_in_both_flag_positions() {
        let (operations, calls, _) = sentinel();

        let before = execute(operations.clone(), &["--json", "doctor"]);
        let after = execute(operations, &["doctor", "--json"]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
        assert!(before.stderr.is_empty());
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
    fn help_and_doctor_never_enter_operational_or_secret_paths() {
        let cases = [
            vec!["help"],
            vec!["--help"],
            vec!["help", "server"],
            vec!["server", "help"],
            vec!["help", "--action", "settings"],
            vec!["--action", "settings", "help"],
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
        ];
        for args in cases {
            let (operations, calls, _) = sentinel();

            let execution = execute(operations, &args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "{args:?}");
            assert_no_operations(&calls, &args);
        }
    }

    #[test]
    fn unknown_contextual_help_fails_before_the_legacy_fallback() {
        let cases = [
            vec!["help", "future_action"],
            vec!["future_action", "help"],
            vec!["help", "--action", "future_action"],
            vec!["--action", "future_action", "help"],
        ];
        for args in cases {
            let (operations, calls, _) = sentinel();

            let execution = execute(operations, &args);

            assert_eq!(execution.exit_code, EXIT_USAGE, "{args:?}");
            assert_no_operations(&calls, &args);
        }
    }

    #[test]
    fn output_mode_and_malformed_help_gate_before_legacy_operations() {
        let cases = [
            vec!["--json"],
            vec!["--json", "future_action"],
            vec!["future_action", "--json"],
            vec!["--json", "--action", "settings"],
            vec!["settings", "help", "tail"],
        ];
        for args in cases {
            let (operations, calls, _) = sentinel();

            let execution = execute(operations, &args);

            assert_eq!(execution.exit_code, EXIT_USAGE, "{args:?}");
            assert_no_operations(&calls, &args);
        }
    }
}
