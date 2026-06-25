use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult};

const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-shot.sock",
    use_tmpdir_env: false,
    support_replace_existing: true,
};

pub enum Command {
    Screenshot,
    Preview,
    Cli(String),
    Kill,
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&CONFIG, tx, parse_command)
}

pub fn cleanup() {
    core_daemon::cleanup(&CONFIG);
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    #[cfg(debug_assertions)]
    if cmd != "ping" {
        qol_runtime::probe!("CMD_RECV", "cmd={cmd}");
    }
    match cmd {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Command(Command::Kill),
        "screenshot" => ReadResult::Command(Command::Screenshot),
        "preview" => ReadResult::Command(Command::Preview),
        other => ReadResult::Command(Command::Cli(other.to_string())),
    }
}
