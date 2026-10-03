use std::collections::{HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::arg::PropMap;
use dbus::blocking::{BlockingSender, Connection};
use dbus::{Message, Path};

use crate::bluetooth::AdapterInfo;

const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
type Objects = HashMap<Path<'static>, HashMap<String, PropMap>>;

fn objects() -> Result<Objects> {
    let connection = Connection::new_system().context("failed to reach the system bus")?;
    let mut request = Message::new_method_call(
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
    )
    .map_err(anyhow::Error::msg)?;
    request.set_auto_start(false);
    connection
        .send_with_reply_and_block(request, QUERY_TIMEOUT)
        .context("Bluetooth service is unavailable")?
        .read1()
        .context("failed to read Bluetooth inventory")
}

pub(super) fn adapters() -> Result<Vec<AdapterInfo>> {
    let objects = objects()?;
    Ok(objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let properties = interfaces.get("org.bluez.Adapter1")?;
            let address = properties.get("Address")?.0.as_str()?;
            Some(AdapterInfo {
                name: path.rsplit('/').next()?.to_string(),
                address: address.to_ascii_uppercase(),
                paired_count: paired_device_count(&objects, path),
            })
        })
        .collect())
}

fn paired_device_count(objects: &Objects, adapter: &str) -> usize {
    objects
        .values()
        .filter_map(|interfaces| interfaces.get("org.bluez.Device1"))
        .filter(|properties| {
            properties.get("Adapter").and_then(|value| value.0.as_str()) == Some(adapter)
                && properties.get("Paired").and_then(|value| value.0.as_u64()) == Some(1)
        })
        .count()
}

pub(super) fn connected_audio_devices() -> Result<HashSet<String>> {
    Ok(objects()?
        .values()
        .filter_map(|interfaces| interfaces.get("org.bluez.MediaTransport1"))
        .filter_map(|properties| properties.get("Device"))
        .filter_map(|device| device.0.as_str())
        .map(str::to_owned)
        .collect())
}
