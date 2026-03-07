use std::sync::mpsc::Sender;

use qol_plugin_api::daemon::{self as core_daemon, DaemonConfig, ReadResult};

const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-os-themes.sock",
    use_tmpdir_env: true,
    support_replace_existing: false,
};

pub enum Command {
    Kill,
}

pub fn send_ping() -> bool {
    core_daemon::send_ping(&CONFIG)
}

pub fn send_kill() -> bool {
    core_daemon::send_kill(&CONFIG)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&CONFIG, tx, parse_command)
}

pub fn cleanup() {
    core_daemon::cleanup(&CONFIG);
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    match cmd {
        "ping" => ReadResult::Handled,
        "run" | "open" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}
