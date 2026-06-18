use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult};

pub const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-cli-sessions.sock",
    use_tmpdir_env: false,
    support_replace_existing: true,
};

#[derive(Debug)]
pub enum Command {
    Open,
    Kill,
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    match cmd {
        "ping" => ReadResult::Handled,
        "open" | "show" => ReadResult::Command(Command::Open),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&CONFIG, tx, parse_command)
}
