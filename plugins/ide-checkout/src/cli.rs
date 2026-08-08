use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::ExitCode;
use std::time::Duration;

use qol_headless::{Command, CommandResult, HeadlessApp};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const BINARY_NAME: &str = "task-runner";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    app_with_handlers(crate::daemon::run, run_status)
}

fn app_with_handlers<Daemon, Status>(daemon: Daemon, status: Status) -> HeadlessApp
where
    Daemon: Fn() -> u8 + Send + Sync + 'static,
    Status: Fn() -> u8 + Send + Sync + 'static,
{
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Run the IDE Checkout checkout daemon and inspect its local runtime contract.")
        .default_command(["daemon"])
        .command(
            Command::new("daemon")
                .about("Run the loopback IDE Checkout checkout daemon.")
                .usage(format!("{BINARY_NAME} daemon"))
                .detail("Loads the typed plugin config and serves checkout requests until stopped.")
                .output("Runtime diagnostics are written to stderr.")
                .exit_behavior("Exits non-zero if the daemon cannot bind or serve requests.")
                .run_result(move |_| Ok(CommandResult::new("", "", daemon()))),
        )
        .command(
            Command::new("status")
                .about("Probe the daemon health endpoint and show a desktop notification.")
                .usage(format!("{BINARY_NAME} status"))
                .detail("This preserves the plugin action used by qol-tray.")
                .output("No stdout; the result is delivered as a desktop notification.")
                .exit_behavior("Exits zero after reporting whether the daemon answered.")
                .run_result(move |_| Ok(CommandResult::new("", "", status()))),
        )
        .command(settings_command())
        .doctor_checks(crate::doctor::checks())
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the IDE Checkout settings page in qol-tray.")
        .usage(format!("{BINARY_NAME} settings"))
        .output("No stdout on success; opens the settings URL through the platform launcher.")
        .exit_behavior("Exits non-zero if the settings URL cannot be launched.")
        .run_result(move |_| Ok(result_for(crate::daemon::open_settings())))
}

fn result_for(result: std::io::Result<()>) -> CommandResult {
    match result {
        Ok(()) => CommandResult::success(""),
        Err(error) => CommandResult::new("", format!("{error}\n"), 1),
    }
}

fn run_status() -> u8 {
    let message = if daemon_is_running() {
        format!(
            "IDE Checkout daemon is running on port {}",
            crate::daemon::daemon_port()
        )
    } else {
        "IDE Checkout daemon is NOT running".to_string()
    };

    qol_plugin_daemon::notification::send_notification("IDE Checkout", &message);
    0
}

fn daemon_is_running() -> bool {
    let mut stream = match TcpStream::connect(("127.0.0.1", crate::daemon::daemon_port())) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    if stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .is_err()
    {
        return false;
    }
    if stream
        .set_write_timeout(Some(Duration::from_millis(300)))
        .is_err()
    {
        return false;
    }

    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }

    let mut buffer = [0_u8; 256];
    let size = match stream.read(&mut buffer) {
        Ok(size) if size > 0 => size,
        _ => return false,
    };

    let response = match std::str::from_utf8(&buffer[..size]) {
        Ok(response) => response,
        Err(_) => return false,
    };

    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use qol_headless::{CommandResult, DoctorReport, EXIT_RUNTIME_ERROR, EXIT_SUCCESS, EXIT_USAGE};
    use qol_plugin_api::manifest::PluginManifest;

    use super::*;

    #[derive(Default)]
    struct OperationCalls {
        daemon: AtomicUsize,
        status: AtomicUsize,
    }

    fn sentinel_app(calls: Arc<OperationCalls>) -> HeadlessApp {
        let daemon_calls = Arc::clone(&calls);
        let status_calls = Arc::clone(&calls);
        app_with_handlers(
            move || {
                daemon_calls.daemon.fetch_add(1, Ordering::SeqCst);
                0
            },
            move || {
                status_calls.status.fetch_add(1, Ordering::SeqCst);
                0
            },
        )
    }

    #[test]
    fn default_daemon_and_status_preserve_the_operational_routes() {
        let cases = [
            (Vec::<String>::new(), 1, 0),
            (vec!["daemon".to_string()], 1, 0),
            (vec!["status".to_string()], 0, 1),
        ];

        for (args, expected_daemon, expected_status) in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution = sentinel_app(Arc::clone(&calls)).execute(args);

            assert_eq!(execution.exit_code, EXIT_SUCCESS);
            assert_eq!(calls.daemon.load(Ordering::SeqCst), expected_daemon);
            assert_eq!(calls.status.load(Ordering::SeqCst), expected_status);
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
    fn command_help_is_equivalent_in_both_positions() {
        for command in ["daemon", "status", "settings", "doctor"] {
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
                "required_binaries",
                "config_readable",
                "runtime_assets",
                "configured_apps",
                "temp_root",
                "daemon_endpoint",
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
    fn doctor_and_help_never_invoke_operational_handlers() {
        let cases = [
            vec!["doctor"],
            vec!["--json", "doctor"],
            vec!["doctor", "--json"],
            vec!["help"],
            vec!["help", "doctor"],
            vec!["doctor", "help"],
            vec!["help", "daemon"],
            vec!["status", "help"],
        ];

        for args in cases {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.iter().map(|arg| (*arg).to_string()));

            assert_eq!(execution.exit_code, EXIT_SUCCESS, "args={args:?}");
            assert_eq!(calls.daemon.load(Ordering::SeqCst), 0, "args={args:?}");
            assert_eq!(calls.status.load(Ordering::SeqCst), 0, "args={args:?}");
        }
    }

    #[test]
    fn unsupported_json_is_rejected_before_status_runs() {
        for args in [["status", "--json"], ["--json", "status"]] {
            let calls = Arc::new(OperationCalls::default());
            let execution =
                sentinel_app(Arc::clone(&calls)).execute(args.into_iter().map(str::to_string));

            assert_eq!(execution.exit_code, EXIT_USAGE, "args={args:?}");
            assert!(
                execution.stderr.contains("does not support --json"),
                "args={args:?}"
            );
            assert_eq!(calls.status.load(Ordering::SeqCst), 0, "args={args:?}");
        }
    }
}
