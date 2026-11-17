use anyhow::{bail, Result};
use qol_headless::DoctorCheckResult;
use qol_runtime::protocol::DaemonRequest;

use crate::bluetooth::{
    AdapterHealth, BackendCapabilities, DaemonAction, DaemonHooks, DaemonReply, DeviceInfo,
    ReconnectReport, ReconnectSelection,
};
use crate::config::ReconnectConfig;

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
    audio_reclaim: crate::audio_claim::RECLAIM_SUPPORTED,
    settings_pairing: None,
    settings_pane: None,
};

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "required_binaries",
        format!(
            "{} has no supported backend on this platform",
            crate::PLUGIN_ID
        ),
    )
    .with_fix(format!(
        "Run {} on Linux, macOS or Windows",
        crate::PLUGIN_ID
    ))
    .with_details(serde_json::json!({
        "platform": qol_platform::os::current_name(),
        "executed": false,
    }))
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn set_adapter_powered(_powered: bool) -> Result<AdapterHealth> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn connect_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn pair_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn set_device_trusted(_address: &str, _trusted: bool) -> Result<DeviceInfo> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn disconnect_device(_address: &str) -> Result<DeviceInfo> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn remove_device(_address: &str) -> Result<()> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn reconnect_devices(
    _config: &ReconnectConfig,
    _selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn adapter_health() -> Result<AdapterHealth> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn search_devices(_config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn open_settings() -> Result<()> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn devices_snapshot(_managed_devices: &[String]) -> Result<serde_json::Value> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn search_status_snapshot() -> Result<serde_json::Value> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn set_adapter_source(_source: fn() -> String) {}

pub enum DaemonCommand {}

pub fn dispatch_request(
    _action: DaemonAction,
    _request: &DaemonRequest,
    _config: impl FnOnce() -> ReconnectConfig,
) -> DaemonReply<DaemonCommand> {
    DaemonReply::Error(format!(
        "{} is not implemented on this platform",
        crate::PLUGIN_ID
    ))
}

pub fn run_daemon(
    _config: ReconnectConfig,
    _hooks: DaemonHooks,
    _listen: impl FnOnce(std::sync::mpsc::Sender<DaemonCommand>) -> bool,
) -> Result<()> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}
