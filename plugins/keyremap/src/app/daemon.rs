use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};

use crate::control::Command;

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: false,
};

pub(super) fn send_reload() -> bool {
    core_daemon::send_action(&CONFIG, "reload", false)
}

pub(super) fn send_kill() -> bool {
    core_daemon::send_kill(&CONFIG)
}

pub(super) fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&CONFIG, tx, parse_command)
}

pub(super) fn cleanup() {
    core_daemon::cleanup(&CONFIG);
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    match cmd {
        "ping" => ReadResult::Handled,
        "reload" => ReadResult::Command(Command::Reload),
        "toggle" => ReadResult::Command(Command::Toggle),
        "kill" => ReadResult::Command(Command::Kill),
        "settings" => ReadResult::Command(Command::Settings),
        _ => ReadResult::Fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_is_a_daemon_command() {
        assert!(matches!(
            parse_command("toggle"),
            ReadResult::Command(Command::Toggle)
        ));
    }
}
