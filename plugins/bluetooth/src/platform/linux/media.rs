use std::collections::HashSet;

use anyhow::{Context, Result};

use crate::bluetooth::{is_audio_device, DeviceInfo};

pub(super) async fn resolve_audio_connections(
    adapter: &str,
    devices: &mut [DeviceInfo],
) -> Result<()> {
    let needs_transports = devices
        .iter()
        .any(|device| device.connected && is_audio_device(device));
    let transports = if needs_transports {
        tokio::task::spawn_blocking(super::inventory::connected_audio_devices)
            .await
            .context("failed to read Bluetooth audio transports")??
    } else {
        HashSet::new()
    };
    for device in devices.iter_mut().filter(|device| is_audio_device(device)) {
        let path = format!(
            "/org/bluez/{adapter}/dev_{}",
            device.address.replace(':', "_")
        );
        device.audio_connected = Some(device.connected && transports.contains(&path));
    }
    Ok(())
}
