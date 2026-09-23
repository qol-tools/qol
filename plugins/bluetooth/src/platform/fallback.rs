use anyhow::{bail, Result};
use qol_headless::DoctorCheckResult;

use crate::bluetooth::{
    AdapterHealth, BackendCapabilities, DeviceInfo, ReconnectReport, ReconnectSelection,
};
use crate::config::ReconnectConfig;

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
    audio_reclaim: crate::audio_claim::platform::RECLAIM_SUPPORTED,
};

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "required_binaries",
        format!(
            "{} has no supported backend on this platform",
            crate::PLUGIN_ID
        ),
    )
    .with_fix(format!("Run {} on Linux or macOS", crate::PLUGIN_ID))
    .with_details(serde_json::json!({
        "platform": std::env::consts::OS,
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

pub fn stop_search() -> Result<()> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn devices_snapshot() -> Result<serde_json::Value> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn search_status_snapshot() -> Result<serde_json::Value> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}

pub fn settings_query(_query: &str) -> std::result::Result<serde_json::Value, String> {
    Err(format!(
        "{} has no native settings on this platform",
        crate::PLUGIN_ID
    )
    .into())
}

pub fn settings_action(
    _action: &str,
    _input: serde_json::Value,
) -> std::result::Result<(), String> {
    Err(format!(
        "{} has no native settings on this platform",
        crate::PLUGIN_ID
    )
    .into())
}

pub fn run_daemon(_config: ReconnectConfig) -> Result<()> {
    bail!("{} is not implemented on this platform", crate::PLUGIN_ID)
}
