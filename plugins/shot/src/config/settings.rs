use anyhow::Result;

use crate::PLUGIN_ID;

pub(crate) fn open_qol_settings() -> Result<()> {
    let url = qol_conventions::settings_url(PLUGIN_ID);
    crate::platform::open_url(&url)
}
