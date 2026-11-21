use qol_sync::Mutex;

use crate::config::{self, ReconnectConfig};

static FORGET_WRITES: Mutex<()> = Mutex::new(());

pub(super) fn forget_managed(current: &ReconnectConfig, address: &str) {
    if config::without_managed(&current.managed_devices, address).is_none() {
        return;
    }
    let address = address.to_string();
    std::mem::drop(std::thread::spawn(move || {
        let _writing = FORGET_WRITES.lock();
        let Some(kept) = config::without_managed(&config::load().managed_devices, &address) else {
            return;
        };
        let id = qol_config::plugin_id_from_env(crate::PLUGIN_ID);
        let keys = serde_json::json!({ "managed_devices": kept });
        if let Err(error) = qol_plugin_api::host_exec::save_plugin_config_keys(&id, &keys) {
            log::warn!("Bluetooth could not drop a removed device from reconnecting: {error}");
        }
    }));
}
