use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult};

const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-launcher.sock",
    use_tmpdir_env: false,
    support_replace_existing: true,
};

pub enum Command {
    Show,
    Reload,
    Kill,
}

pub fn send_show() -> bool {
    core_daemon::send_action(&CONFIG, "show", true)
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
        "show" | "open" => ReadResult::Command(Command::Show),
        "reload" => ReadResult::Command(Command::Reload),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}
