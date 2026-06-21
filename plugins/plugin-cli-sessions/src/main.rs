use std::env;
use std::process::ExitCode;

use plugin_cli_sessions::daemon::actions::CONFIG;
use qol_plugin_daemon::daemon as core_daemon;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("daemon") => run_daemon(false),
        Some("open") | Some("run") => open_or_show(),
        Some("next") => {
            core_daemon::send_action(&CONFIG, "next", false);
            ExitCode::SUCCESS
        }
        Some("snapshot") => {
            core_daemon::send_action(&CONFIG, "snapshot", false);
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("plugin-cli-sessions: unknown subcommand {other:?}");
            ExitCode::from(2)
        }
    }
}

fn open_or_show() -> ExitCode {
    if core_daemon::send_action(&CONFIG, "open", false) {
        return ExitCode::SUCCESS;
    }
    run_daemon(true)
}

fn run_daemon(visible: bool) -> ExitCode {
    match plugin_cli_sessions::daemon::run(visible) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("plugin-cli-sessions: {e:#}");
            ExitCode::from(1)
        }
    }
}
