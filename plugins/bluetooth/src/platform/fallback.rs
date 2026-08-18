use anyhow::{bail, Result};
use qol_headless::DoctorCheckResult;

use crate::bluetooth::{
    AdapterHealth, BackendCapabilities, DeviceInfo, ReconnectReport, ReconnectSelection,
};
use crate::config::ReconnectConfig;

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
};

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "required_binaries",
        "plugin-bluetooth has no supported backend on this platform",
    )
    .with_fix("Run plugin-bluetooth on Linux or macOS")
    .with_details(serde_json::json!({
        "platform": std::env::consts::OS,
        "executed": false,
    }))
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn set_adapter_powered(_powered: bool) -> Result<AdapterHealth> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn connect_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn pair_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn set_device_trusted(_address: &str, _trusted: bool) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn disconnect_device(_address: &str) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn remove_device(_address: &str) -> Result<()> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn reconnect_devices(
    _config: &ReconnectConfig,
    _selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn adapter_health() -> Result<AdapterHealth> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn search_devices(_config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn stop_search() -> Result<()> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn devices_snapshot() -> Result<serde_json::Value> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn search_status_snapshot() -> Result<serde_json::Value> {
    bail!("plugin-bluetooth is not implemented on this platform")
}

pub fn settings_query(_query: &str) -> std::result::Result<serde_json::Value, String> {
    Err("plugin-bluetooth has no native settings on this platform".into())
}

pub fn settings_action(
    _action: &str,
    _input: serde_json::Value,
) -> std::result::Result<(), String> {
    Err("plugin-bluetooth has no native settings on this platform".into())
}

pub fn run_daemon(_config: ReconnectConfig) -> Result<()> {
    bail!("plugin-bluetooth is not implemented on this platform")
}
