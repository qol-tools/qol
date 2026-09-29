use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;

use crate::config::ServerConfig;
use crate::network;

const CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: false,
};

const APP_DOWNLOAD_URL: &str = "https://github.com/qol-tools/pointz/releases/latest";

pub enum Command {
    Settings,
    BeginPairing,
    Input(serde_json::Value),
    Kill,
}

pub fn send_action(action: &str) -> bool {
    core_daemon::send_action(&CONFIG, action, false)
}

pub fn send_kill() -> bool {
    core_daemon::send_kill(&CONFIG)
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_request_listener(&CONFIG, tx, parse_request)
}

pub fn cleanup() {
    core_daemon::cleanup(&CONFIG);
}

fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "input" => ReadResult::Command(Command::Input(request.input.clone())),
        "ping" => ReadResult::Handled,
        "settings" => ReadResult::Command(Command::Settings),
        "begin_pairing" => ReadResult::Command(Command::BeginPairing),
        "kill" => ReadResult::Command(Command::Kill),
        "connection_status" => ReadResult::HandledWithData(serde_json::json!({ "state": "ok" })),
        "connection_info" => ReadResult::HandledWithData(serde_json::json!({
            "hostname": network::get_hostname(),
            "ip": network::get_local_ip().map(|ip| ip.to_string()),
            "discovery_port": ServerConfig::DISCOVERY_PORT,
            "command_port": ServerConfig::COMMAND_PORT,
            "app_download_url": APP_DOWNLOAD_URL,
        })),
        "pairing_status" => ReadResult::HandledWithData(super::pairing::status_json()),
        _ => ReadResult::Fallback,
    }
}
