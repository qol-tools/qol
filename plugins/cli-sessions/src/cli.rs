use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use qol_headless::{Command, CommandResult, DoctorCheck, HeadlessApp, PlainTextOutput};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};
use qol_terminal_sessions::{BackendId, SessionId};

use crate::daemon::actions::CONFIG;
use crate::storage::paths::PLUGIN_ID;

const OPEN: &str = "open";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args: Vec<String> = args.into_iter().collect();
    if let Some(result) = qol_terminal_sessions::console::helper(&args) {
        return result.emit();
    }
    if let Some((command, rest)) = args.split_first() {
        if command == crate::park::PARK {
            return park(rest).emit();
        }
        if command == crate::park::LANE_EXEC {
            return lane_exec(rest).emit();
        }
    }
    app().run(args)
}

fn lane_exec(args: &[String]) -> CommandResult {
    let args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
    match qol_agent_launch::lane_exec::run(&args) {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::runtime_error(format!("{PLUGIN_ID} lane-exec: {error:#}")),
    }
}

fn park(args: &[String]) -> CommandResult {
    match crate::park::run(args) {
        Ok(crate::park::Invocation::Help) => CommandResult::success(crate::park::HELP),
        Ok(crate::park::Invocation::Ran) => CommandResult::success(""),
        Ok(crate::park::Invocation::Parked(outcome)) => {
            match serde_json::to_string_pretty(&outcome) {
                Ok(json) => CommandResult::success(json),
                Err(error) => CommandResult::runtime_error(format!("{PLUGIN_ID} park: {error}")),
            }
        }
        Err(error) => CommandResult::runtime_error(format!("{PLUGIN_ID} park: {error:#}")),
    }
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

    HeadlessApp::new(PLUGIN_ID, PLUGIN_ID)
        .about("Track live terminal sessions and summon the retained CLI Sessions panel.")
        .default_command(["run"])
        .command(
            Command::new("run")
                .alias("daemon")
                .about("Run the resident session monitor and retained GPUI panel host.")
                .usage(format!("{PLUGIN_ID} run"))
                .detail("The legacy `daemon` command is an alias.")
                .detail(
                    "The panel starts hidden and reconciles terminal sessions in the background.",
                )
                .output("Lifecycle diagnostics are written to stderr.")
                .exit_behavior("Runs until stopped; exits non-zero if daemon startup fails.")
                .run_result(move |_| Ok(daemon_run(false))),
        )
        .command(
            Command::new(OPEN)
                .about("Show the retained CLI Sessions panel.")
                .usage(format!("{PLUGIN_ID} {OPEN}"))
                .detail("Signals the resident daemon, or starts it with the panel visible.")
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero only if fallback daemon startup fails.")
                .run_result(move |_| {
                    if open_send(OPEN) {
                        return Ok(CommandResult::success(""));
                    }
                    Ok(open_run(true))
                }),
        )
        .command(
            Command::new("next")
                .about("Focus the next terminal session that needs attention.")
                .usage(format!("{PLUGIN_ID} next"))
                .detail("Sends the request to the resident daemon without starting a new one.")
                .output("No stdout; daemon availability is intentionally best-effort.")
                .exit_behavior("Exits zero after attempting delivery.")
                .run_result(move |_| {
                    next_send("next");
                    Ok(CommandResult::success(""))
                }),
        )
        .command(
            Command::new("focus")
                .about("Focus the terminal of one tracked session.")
                .usage(format!("{PLUGIN_ID} focus <backend>:<session>"))
                .detail("A click on a CLI Sessions notification runs this for its session.")
                .output("No stdout on success.")
                .exit_behavior(
                    "Exits non-zero if the session id is invalid or the daemon does not answer.",
                )
                .run_plain_text(|context| {
                    let session = parse_session(context.args())?;
                    ask_daemon(crate::ui::notify::focus_request(&session))?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .command(
            Command::new("reopen")
                .about("Reopen the conversation of a finished parked session.")
                .usage(format!("{PLUGIN_ID} reopen <park-id>"))
                .detail(
                    "A click on a finished parked session's notification runs this for its park.",
                )
                .output("No stdout on success.")
                .exit_behavior(
                    "Exits non-zero without exactly one park id or if the daemon does not answer.",
                )
                .run_plain_text(|context| {
                    let [park] = context.args() else {
                        anyhow::bail!("expected one park id like park-1791472381-2374635");
                    };
                    ask_daemon(crate::ui::notify::reopen_request(park))?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .command(
            Command::new(crate::park::PARK)
                .about("Close this harness and resume it when a long command exits.")
                .usage(format!(
                    "{PLUGIN_ID} park [--model MODEL] [--effort LEVEL] [--title TITLE] [--session SESSION] -- <command> [args...]"
                ))
                .detail("Parks the calling harness session on a long wait such as a pull request watcher.")
                .detail("A detached CLI Sessions process runs the command, closes the calling terminal once its turn ends, resumes the conversation in a new tab when the command exits, and closes that tab once its turn ends.")
                .detail("Everything after `--` is the command, passed through verbatim.")
                .output("Park JSON on stdout; diagnostics on stderr.")
                .exit_behavior("Exits non-zero when the calling session cannot be resolved or resumed, or the runner cannot start.")
                .run_result(|context| Ok(park(context.args()))),
        )
        .command(
            Command::new("parked")
                .about("List parked sessions and what became of them.")
                .usage(format!("{PLUGIN_ID} parked"))
                .detail("One row per park: id, state, tool, cwd, the command it waits on, and the session it resumed in.")
                .output("Park rows on stdout; diagnostics on stderr.")
                .exit_behavior("Exits non-zero when the parked directory cannot be read.")
                .run_plain_text(|context| {
                    reject_args(context.args())?;
                    let rows = crate::park::list()?;
                    if rows.is_empty() {
                        return Ok(PlainTextOutput::text("no parked sessions recorded"));
                    }
                    Ok(PlainTextOutput::text(rows.join("\n")))
                }),
        )
        .command(
            Command::new("unpark")
                .about("Stop a parked wait and resume the conversation now, or reopen a finished one.")
                .usage(format!("{PLUGIN_ID} unpark <park-id>"))
                .detail("Refuses a park that is no longer waiting or whose terminal is still open.")
                .output("A confirmation line on stdout; diagnostics on stderr.")
                .exit_behavior("Exits non-zero when the id is unknown, no longer waiting, still open, or the resume fails.")
                .run_plain_text(|context| {
                    let [park] = context.args() else {
                        anyhow::bail!("expected one park id like park-1791472381-2374635");
                    };
                    Ok(PlainTextOutput::text(crate::park::unpark(park)?))
                }),
        )
        .command(
            Command::new("snapshot")
                .about("Ask the resident daemon to snapshot all observed sessions.")
                .usage(format!("{PLUGIN_ID} snapshot"))
                .detail("The daemon owns terminal reads and snapshot persistence.")
                .output("No stdout; snapshot diagnostics are emitted by the daemon.")
                .exit_behavior("Exits zero after attempting delivery.")
                .run_result(move |_| {
                    snapshot_send("snapshot");
                    Ok(CommandResult::success(""))
                }),
        )
        .command(
            Command::new("settings")
                .about("Open the CLI Sessions settings in the qol settings surface.")
                .usage(format!("{PLUGIN_ID} settings"))
                .detail("Opens the unified qol settings surface.")
                .output("No stdout on success.")
                .exit_behavior("Exits non-zero if the settings surface cannot be opened.")
                .run_plain_text(|context| {
                    reject_args(context.args())?;
                    crate::show_settings()?;
                    Ok(PlainTextOutput::empty())
                }),
        )
        .doctor_checks(doctor_checks)
}

fn run_daemon(show_on_start: bool) -> CommandResult {
    match crate::daemon::run(show_on_start) {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::runtime_error(format!("{PLUGIN_ID}: {error:#}")),
    }
}

fn reject_args(args: &[String]) -> anyhow::Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    anyhow::bail!("unexpected arguments: {}", args.join(" "))
}

fn parse_session(args: &[String]) -> anyhow::Result<SessionId> {
    let [session] = args else {
        anyhow::bail!("expected one session id like kitty:42");
    };
    let (backend, native) = session
        .split_once(':')
        .context("a session id is <backend>:<session>, like kitty:42")?;
    Ok(SessionId::new(BackendId::new(backend)?, native)?)
}

fn ask_daemon(request: DaemonRequest) -> anyhow::Result<()> {
    let action = request.action.clone();
    let response = qol_plugin_daemon::daemon::send_request(
        &CONFIG,
        &request.action,
        request.input,
        Duration::from_secs(2),
    )
    .context("the CLI Sessions daemon is not running")?;
    match response {
        DaemonResponse::Handled { .. } => Ok(()),
        DaemonResponse::Error { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("the daemon did not handle {action}: {other:?}"),
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
    fn focus_rejects_malformed_session_ids() {
        let cases: [&[&str]; 4] = [&[], &["kitty"], &["kitty:"], &["kitty:1", "kitty:2"]];
        for args in cases {
            let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            assert!(super::parse_session(&args).is_err(), "{args:?}");
        }
        let parsed = super::parse_session(&["kitty:42".to_string()]).unwrap();
        assert_eq!(parsed.to_string(), "kitty:42");
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
