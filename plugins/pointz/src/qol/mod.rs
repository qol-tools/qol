pub(crate) mod pages;

use crate::PLUGIN_ID;

pub fn open_settings() {
    if let Err(error) = qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID) {
        log::warn!("{PLUGIN_ID}: failed to open settings: {error}");
    }
}
