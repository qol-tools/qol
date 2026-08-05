use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: false,
};

pub enum Command {
    Reload,
    Kill,
    Settings,
}

pub fn send_reload() -> bool {
    core_daemon::send_action(&CONFIG, "reload", false)
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
        "reload" => ReadResult::Command(Command::Reload),
        "kill" => ReadResult::Command(Command::Kill),
        "settings" => ReadResult::Command(Command::Settings),
        _ => ReadResult::Fallback,
    }
}
