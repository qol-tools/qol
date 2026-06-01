use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult};

use crate::domain::config::ServerConfig;
use crate::utils;

const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-pointz.sock",
    use_tmpdir_env: true,
    support_replace_existing: false,
};

const APP_DOWNLOAD_URL: &str = "https://github.com/qol-tools/pointz/releases/latest";

pub enum Command {
    Settings,
    Kill,
}

pub fn send_action(action: &str) -> bool {
    core_daemon::send_action(&CONFIG, action, false)
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
        "settings" => ReadResult::Command(Command::Settings),
        "kill" => ReadResult::Command(Command::Kill),
        "connection_status" => ReadResult::HandledWithData(serde_json::json!({ "state": "ok" })),
        "connection_info" => ReadResult::HandledWithData(serde_json::json!({
            "hostname": utils::get_hostname(),
            "ip": utils::get_local_ip().map(|ip| ip.to_string()),
            "discovery_port": ServerConfig::DISCOVERY_PORT,
            "command_port": ServerConfig::COMMAND_PORT,
            "app_download_url": APP_DOWNLOAD_URL,
        })),
        _ => ReadResult::Fallback,
    }
}
