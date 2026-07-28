use std::process::ExitCode;
use std::sync::Arc;

use qol_headless::{Command, CommandResult, DoctorCheck, HeadlessApp};

use crate::daemon::actions::CONFIG;
use crate::storage::paths::PLUGIN_ID;

const BINARY_NAME: &str = "cli-sessions";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(run_daemon, send_action, crate::doctor::checks())
}

fn app_with_handlers<Run, SendAction>(
    run: Run,
    send: SendAction,
    doctor_checks: Vec<DoctorCheck>,
) -> HeadlessApp
where
    Run: Fn(bool) -> CommandResult + Send + Sync + 'static,
    SendAction: Fn(&str) -> bool + Send + Sync + 'static,
{
    let run = Arc::new(run);
    let send = Arc::new(send);

    let daemon_run = Arc::clone(&run);
    let open_run = Arc::clone(&run);
    let open_send = Arc::clone(&send);
    let next_send = Arc::clone(&send);
    let snapshot_send = send;

    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Track live terminal sessions and summon the retained CLI Sessions panel.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .alias("daemon")
                .about("Run the resident session monitor and retained GPUI panel host.")
                .usage(format!("{BINARY_NAME} run"))
                .detail("The legacy `daemon` command is an alias.")
                .detail(
                    "The panel starts hidden and reconciles terminal sessions in the background.",
                )
                .output("Lifecycle diagnostics are written to stderr.")
                .exit_behavior("Runs until stopped; exits non-zero if daemon startup fails.")
                .run_result(move |_| Ok(daemon_run(false))),
        )
        .command(
            Command::new("open")
                .about("Show the retained CLI Sessions panel.")
                .usage(format!("{BINARY_NAME} open"))
                .detail("Signals the resident daemon, or starts it with the panel visible.")
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero only if fallback daemon startup fails.")
                .run_result(move |_| {
                    if open_send("open") {
                        return Ok(CommandResult::success(""));
                    }
                    Ok(open_run(true))
                }),
        )
        .command(
            Command::new("next")
                .about("Focus the next terminal session that needs attention.")
                .usage(format!("{BINARY_NAME} next"))
                .detail("Sends the request to the resident daemon without starting a new one.")
                .output("No stdout; daemon availability is intentionally best-effort.")
                .exit_behavior("Exits zero after attempting delivery.")
                .run_result(move |_| {
                    next_send("next");
                    Ok(CommandResult::success(""))
                }),
        )
        .command(
            Command::new("snapshot")
                .about("Ask the resident daemon to snapshot all observed sessions.")
                .usage(format!("{BINARY_NAME} snapshot"))
                .detail("The daemon owns terminal reads and snapshot persistence.")
                .output("No stdout; snapshot diagnostics are emitted by the daemon.")
                .exit_behavior("Exits zero after attempting delivery.")
                .run_result(move |_| {
                    snapshot_send("snapshot");
                    Ok(CommandResult::success(""))
                }),
        )
        .doctor_checks(doctor_checks)
}

fn run_daemon(show_on_start: bool) -> CommandResult {
    match crate::daemon::run(show_on_start) {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::runtime_error(format!("plugin-cli-sessions: {error:#}")),
    }
}

fn send_action(action: &str) -> bool {
    qol_plugin_daemon::daemon::send_action(&CONFIG, action, false)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use qol_headless::{DoctorCheckResult, DoctorReport, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        run: Mutex<Vec<bool>>,
        actions: Mutex<Vec<String>>,
        daemon_available: bool,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let run_calls = Arc::clone(&calls);
        let send_calls = Arc::clone(&calls);
        app_with_handlers(
            move |show_on_start| {
                run_calls.run.lock().unwrap().push(show_on_start);
                CommandResult::success("")
            },
            move |action| {
                send_calls.actions.lock().unwrap().push(action.to_string());
                send_calls.daemon_available
            },
            sentinel_doctor_checks(),
        )
    }

    fn sentinel_doctor_checks() -> Vec<DoctorCheck> {
        crate::doctor::check_ids()
            .iter()
            .map(|id| {
                let id = *id;
                DoctorCheck::new(id, format!("Sentinel {id} check."), move || {
                    Ok(DoctorCheckResult::ok(id, format!("{id} is healthy")))
                })
            })
            .collect()
    }

    #[test]
    fn daemon_aliases_preserve_hidden_startup() {
        for args in [
            Vec::<String>::new(),
            vec!["run".to_string()],
            vec!["daemon".to_string()],
        ] {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute(args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS);
            assert_eq!(calls.run.lock().unwrap().as_slice(), [false]);
            assert!(calls.actions.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn open_signals_the_daemon_or_starts_visible_fallback() {
        let available = Arc::new(OperationCalls {
            daemon_available: true,
            ..OperationCalls::default()
        });
        let execution = sentinel_app(Arc::clone(&available)).execute(["open".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(available.actions.lock().unwrap().as_slice(), ["open"]);
        assert!(available.run.lock().unwrap().is_empty());

        let missing = Arc::new(OperationCalls::default());
        let execution = sentinel_app(Arc::clone(&missing)).execute(["open".to_string()]);

        assert_eq!(execution.exit_code, EXIT_SUCCESS);
        assert_eq!(missing.actions.lock().unwrap().as_slice(), ["open"]);
        assert_eq!(missing.run.lock().unwrap().as_slice(), [true]);
    }

    #[test]
    fn one_shot_actions_preserve_best_effort_delivery() {
        let calls = Arc::new(OperationCalls::default());
        let app = sentinel_app(Arc::clone(&calls));

        assert_eq!(app.execute(["next".to_string()]).exit_code, EXIT_SUCCESS);
        assert_eq!(
            app.execute(["snapshot".to_string()]).exit_code,
            EXIT_SUCCESS
        );
        assert_eq!(
            calls.actions.lock().unwrap().as_slice(),
            ["next", "snapshot"]
        );
        assert!(calls.run.lock().unwrap().is_empty());
    }

    #[test]
    fn manifest_actions_have_contextual_help() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        for action in manifest.executable_actions() {
            let args = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let command = args.first().expect("runtime args must name a command");
            let execution = app_with_handlers(
                |_| CommandResult::success(""),
                |_| true,
                sentinel_doctor_checks(),
            )
            .execute(["help".to_string(), command.clone()]);

            assert_eq!(
                execution.exit_code, EXIT_SUCCESS,
                "action={} stderr={}",
                action.id, execution.stderr
            );
        }
    }

    #[test]
    fn contextual_help_is_equivalent_in_both_positions() {
        for command in ["run", "open", "next", "snapshot", "doctor"] {
            let first = sentinel_app(Arc::new(OperationCalls::default()))
                .execute(["help".to_string(), command.to_string()]);
            let final_token = sentinel_app(Arc::new(OperationCalls::default()))
                .execute([command.to_string(), "help".to_string()]);

            assert_eq!(first.exit_code, EXIT_SUCCESS, "command={command}");
            assert_eq!(first.stdout, final_token.stdout, "command={command}");
            assert!(first.stdout.contains("Output:"), "command={command}");
            assert!(first.stdout.contains("Exit:"), "command={command}");
        }
    }

    #[test]
    fn doctor_json_matches_the_shared_contract_in_both_flag_positions() {
        let before = sentinel_app(Arc::new(OperationCalls::default()))
            .execute(["--json".to_string(), "doctor".to_string()]);
        let after = sentinel_app(Arc::new(OperationCalls::default()))
            .execute(["doctor".to_string(), "--json".to_string()]);

        assert_eq!(before.exit_code, EXIT_SUCCESS);
        assert_eq!(before.stdout, after.stdout);
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
    }

    #[test]
    fn doctor_and_help_never_reach_operational_handlers() {
        let cases = [
            vec!["help"],
            vec!["--help"],
            vec!["help", "run"],
            vec!["open", "help"],
            vec!["help", "next"],
            vec!["snapshot", "help"],
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
            assert!(calls.run.lock().unwrap().is_empty(), "args={args:?}");
            assert!(calls.actions.lock().unwrap().is_empty(), "args={args:?}");
        }
    }

    #[test]
    fn unsupported_json_cannot_start_or_signal_the_daemon() {
        for args in [
            vec!["--json"],
            vec!["--json", "run"],
            vec!["run", "--json"],
            vec!["--json", "open"],
        ] {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args={args:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "args={args:?}"
            );
            assert!(calls.run.lock().unwrap().is_empty(), "args={args:?}");
            assert!(calls.actions.lock().unwrap().is_empty(), "args={args:?}");
        }
    }
}
