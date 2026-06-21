use anyhow::Result;

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-screen-recorder/";

pub(crate) fn open_qol_settings() -> Result<()> {
    crate::platform::open_url(SETTINGS_URL)
}
