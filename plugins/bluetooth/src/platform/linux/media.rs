use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::blocking::stdintf::org_freedesktop_dbus::ObjectManager;
use dbus::blocking::Connection;

use crate::bluetooth::{is_audio_device, DeviceInfo};

const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) async fn resolve_audio_connections(
    adapter: &str,
    devices: &mut [DeviceInfo],
) -> Result<()> {
    let needs_transports = devices
        .iter()
        .any(|device| device.connected && is_audio_device(device));
    let transports = if needs_transports {
        tokio::task::spawn_blocking(connected_devices)
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

fn connected_devices() -> Result<HashSet<String>> {
    let connection = Connection::new_system().context("failed to reach BlueZ")?;
    let objects = connection
        .with_proxy("org.bluez", "/", QUERY_TIMEOUT)
        .get_managed_objects()
        .context("failed to inspect Bluetooth audio transports")?;
    Ok(objects
        .values()
        .filter_map(|interfaces| interfaces.get("org.bluez.MediaTransport1"))
        .filter_map(|properties| properties.get("Device"))
        .filter_map(|device| device.0.as_str())
        .map(str::to_owned)
        .collect())
}
