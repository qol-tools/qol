pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn settings_url() -> String {
    qol_conventions::settings_url(PLUGIN_ID)
}

pub fn open_settings() {
    let url = settings_url();
    if let Err(error) = open::that(&url) {
        log::warn!("{PLUGIN_ID}: failed to open settings URL {url}: {error}");
    }
}
