use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use super::managed::forget_managed;
use super::settings::show_settings;
use crate::bluetooth::{DaemonAction, DaemonHooks, DaemonReply};
use crate::{config, platform};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const HOOKS: DaemonHooks = DaemonHooks {
    open_settings_page: show_settings,
    forget_managed,
};

pub fn run_daemon() -> Result<()> {
    use_configured_adapter();
    platform::run_daemon(config::load(), HOOKS, listen)
}

pub fn use_configured_adapter() {
    platform::set_adapter_source(configured_adapter);
}

fn configured_adapter() -> String {
    config::load().adapter
}

pub fn stop_search() -> Result<()> {
    if core_daemon::send_action(&DAEMON_CONFIG, "stop_search", true) {
        return Ok(());
    }
    bail!("Bluetooth daemon is not reachable")
}

pub fn settings_query(query: &str) -> std::result::Result<serde_json::Value, String> {
    match core_daemon::send_request(
        &DAEMON_CONFIG,
        query,
        serde_json::Value::Null,
        Duration::from_secs(2),
    ) {
        Ok(DaemonResponse::Handled { data: Some(data) }) => Ok(data),
        Ok(DaemonResponse::Handled { data: None }) => Ok(serde_json::Value::Null),
        Ok(DaemonResponse::Error { message }) => Err(message),
        Ok(DaemonResponse::Fallback) => Err("Bluetooth daemon declined the query".into()),
        Ok(DaemonResponse::NotReady { .. }) => Err("Bluetooth daemon is still starting".into()),
        Err(error) => Err(format!("Bluetooth daemon query failed: {error}")),
    }
}

pub fn settings_action(action: &str, input: serde_json::Value) -> std::result::Result<(), String> {
    match core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(2)) {
        Ok(DaemonResponse::Handled { .. }) => Ok(()),
        Ok(DaemonResponse::Error { message }) => Err(message),
        Ok(DaemonResponse::Fallback) => Err("Bluetooth daemon declined the action".into()),
        Ok(DaemonResponse::NotReady { .. }) => Err("Bluetooth daemon is still starting".into()),
        Err(error) => Err(format!("Bluetooth daemon action failed: {error}")),
    }
}

fn listen(commands: mpsc::Sender<platform::DaemonCommand>) -> bool {
    core_daemon::start_request_listener(&DAEMON_CONFIG, commands, handle_request)
}

fn handle_request(request: &DaemonRequest) -> ReadResult<platform::DaemonCommand> {
    match parse_action(&request.action) {
        Ok(action) => read_result(platform::dispatch_request(action, request, config::load)),
        Err(message) => ReadResult::Error(message),
    }
}

fn read_result<C>(reply: DaemonReply<C>) -> ReadResult<C> {
    match reply {
        DaemonReply::Command(command) => ReadResult::Command(command),
        DaemonReply::Handled => ReadResult::Handled,
        DaemonReply::Data(payload) => ReadResult::HandledWithData(payload),
        DaemonReply::Error(message) => ReadResult::Error(message),
    }
}

fn parse_action(id: &str) -> std::result::Result<DaemonAction, String> {
    match id {
        "ping" => Ok(DaemonAction::Ping),
        "kill" => Ok(DaemonAction::Kill),
        "enable_adapter" => Ok(DaemonAction::EnableAdapter),
        "disable_adapter" => Ok(DaemonAction::DisableAdapter),
        "pair_device" => Ok(DaemonAction::PairDevice),
        "trust_device" => Ok(DaemonAction::TrustDevice),
        "untrust_device" => Ok(DaemonAction::UntrustDevice),
        "connect_device" => Ok(DaemonAction::ConnectDevice),
        "disconnect_device" => Ok(DaemonAction::DisconnectDevice),
        "reclaim_device" => Ok(DaemonAction::ReclaimDevice),
        "remove_device" => Ok(DaemonAction::RemoveDevice),
        "start_search" => Ok(DaemonAction::StartSearch),
        "stop_search" => Ok(DaemonAction::StopSearch),
        "open_system_settings" => Ok(DaemonAction::OpenSystemSettings),
        "devices" => Ok(DaemonAction::Devices),
        "nearby_devices" => Ok(DaemonAction::NearbyDevices),
        "managed_device_options" => Ok(DaemonAction::ManagedDeviceOptions),
        "adapter_options" => Ok(DaemonAction::AdapterOptions),
        "search_status" => Ok(DaemonAction::SearchStatus),
        "adapter_status" => Ok(DaemonAction::AdapterStatus),
        "reconnect" => Ok(DaemonAction::Reconnect),
        "reconnect_trusted" => Ok(DaemonAction::ReconnectTrusted),
        "reload" => Ok(DaemonAction::Reload),
        "settings" => Ok(DaemonAction::Settings),
        "host_fixes" => Ok(DaemonAction::HostFixes),
        "host_problems" => Ok(DaemonAction::HostProblems),
        "apply_host_fix" => Ok(DaemonAction::ApplyHostFix),
        "handoff_state" => Ok(DaemonAction::HandoffState),
        "release_for_handoff" => Ok(DaemonAction::ReleaseForHandoff),
        "resume_reconnect" => Ok(DaemonAction::ResumeReconnect),
        unknown => Err(format!("unknown Bluetooth action: {unknown}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn daemon_actions_parse_at_the_protocol_boundary() {
        let cases = [
            ("ping", DaemonAction::Ping),
            ("kill", DaemonAction::Kill),
            ("enable_adapter", DaemonAction::EnableAdapter),
            ("disable_adapter", DaemonAction::DisableAdapter),
            ("pair_device", DaemonAction::PairDevice),
            ("trust_device", DaemonAction::TrustDevice),
            ("untrust_device", DaemonAction::UntrustDevice),
            ("connect_device", DaemonAction::ConnectDevice),
            ("disconnect_device", DaemonAction::DisconnectDevice),
            ("reclaim_device", DaemonAction::ReclaimDevice),
            ("remove_device", DaemonAction::RemoveDevice),
            ("start_search", DaemonAction::StartSearch),
            ("stop_search", DaemonAction::StopSearch),
            ("open_system_settings", DaemonAction::OpenSystemSettings),
            ("devices", DaemonAction::Devices),
            ("nearby_devices", DaemonAction::NearbyDevices),
            ("managed_device_options", DaemonAction::ManagedDeviceOptions),
            ("adapter_options", DaemonAction::AdapterOptions),
            ("search_status", DaemonAction::SearchStatus),
            ("adapter_status", DaemonAction::AdapterStatus),
            ("reconnect", DaemonAction::Reconnect),
            ("reconnect_trusted", DaemonAction::ReconnectTrusted),
            ("reload", DaemonAction::Reload),
            ("settings", DaemonAction::Settings),
            ("host_fixes", DaemonAction::HostFixes),
            ("host_problems", DaemonAction::HostProblems),
            ("apply_host_fix", DaemonAction::ApplyHostFix),
            ("handoff_state", DaemonAction::HandoffState),
            ("release_for_handoff", DaemonAction::ReleaseForHandoff),
            ("resume_reconnect", DaemonAction::ResumeReconnect),
        ];
        for (id, expected) in cases {
            assert_eq!(parse_action(id), Ok(expected), "action={id}");
        }
    }

    #[test]
    fn unknown_daemon_actions_are_rejected_by_name() {
        assert_eq!(
            parse_action("not_a_bluetooth_action"),
            Err("unknown Bluetooth action: not_a_bluetooth_action".into())
        );
    }

    #[test]
    fn every_manifest_action_reaches_the_daemon_parse() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        for action in manifest.executable_actions() {
            assert!(parse_action(&action.id).is_ok(), "action={}", action.id);
        }
    }

    #[test]
    fn replies_map_onto_the_daemon_read_results() {
        assert!(matches!(
            read_result(DaemonReply::Command(7)),
            ReadResult::Command(7)
        ));
        assert!(matches!(
            read_result::<u8>(DaemonReply::Handled),
            ReadResult::Handled
        ));
        assert!(matches!(
            read_result::<u8>(DaemonReply::Data(serde_json::json!({ "count": 0 }))),
            ReadResult::HandledWithData(_)
        ));
        assert!(matches!(
            read_result::<u8>(DaemonReply::Error("no".into())),
            ReadResult::Error(message) if message == "no"
        ));
    }
}
