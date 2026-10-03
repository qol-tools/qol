use anyhow::Result;

use crate::PLUGIN_ID;

pub(crate) fn open_qol_settings() -> Result<()> {
    Ok(qol_apps::desktop_integration::open_plugin_settings_via_tray(PLUGIN_ID)?)
}
