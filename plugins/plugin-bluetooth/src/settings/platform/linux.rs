use std::time::Duration;

use anyhow::Result;

pub(crate) fn run_panel() -> Result<()> {
    let runtime = qol_gpui::settings_panel::SettingsRuntime::new(crate::platform::settings_query)
        .with_input_action(crate::platform::settings_action)
        .poll_every(Duration::from_millis(500));
    qol_gpui::settings_panel::run_plugin_settings(
        crate::PLUGIN_ID,
        "Bluetooth Settings",
        crate::config::contract(),
        runtime,
    )
}
