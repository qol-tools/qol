pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() {
    if let Err(error) = qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID) {
        let url = qol_conventions::settings_url(PLUGIN_ID);
        log::warn!("{PLUGIN_ID}: failed to open settings URL {url}: {error}");
    }
}
