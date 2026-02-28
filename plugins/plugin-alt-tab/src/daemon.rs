use std::sync::mpsc::Sender;

use qol_plugin_api::daemon::{self as core_daemon, DaemonConfig, ReadResult};

const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-alt-tab.sock",
    use_tmpdir_env: true,
    support_replace_existing: false,
};

pub enum Command {
    Show,
    ShowReverse,
    Kill,
}

pub fn send_show() -> bool {
    core_daemon::send_action(&CONFIG, "show", false)
}

pub fn send_show_reverse() -> bool {
    core_daemon::send_action(&CONFIG, "show-reverse", false)
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
        "show-reverse" | "open-reverse" => ReadResult::Command(Command::ShowReverse),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}
