use anyhow::{bail, Result};
use qol_headless::DoctorCheckResult;

use crate::bluetooth::{AdapterHealth, DeviceInfo, ReconnectReport, ReconnectSelection};
use crate::config::ReconnectConfig;

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "required_binaries",
        "Bluetooth has no supported macOS backend",
    )
    .with_fix("Run Bluetooth on Linux")
    .with_details(serde_json::json!({
        "platform": "macos",
        "pactl": null,
        "executed": false,
    }))
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn set_adapter_powered(_powered: bool) -> Result<AdapterHealth> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn connect_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn pair_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn set_device_trusted(_address: &str, _trusted: bool) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn disconnect_device(_address: &str) -> Result<DeviceInfo> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn remove_device(_address: &str) -> Result<()> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn reconnect_devices(
    _config: &ReconnectConfig,
    _selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn adapter_health() -> Result<AdapterHealth> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn search_devices(_config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn stop_search() -> Result<()> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn devices_snapshot() -> Result<serde_json::Value> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn search_status_snapshot() -> Result<serde_json::Value> {
    bail!("plugin-bluetooth is not implemented on macOS")
}

pub fn run_daemon(_config: ReconnectConfig) -> Result<()> {
    bail!("plugin-bluetooth is not implemented on macOS")
}
