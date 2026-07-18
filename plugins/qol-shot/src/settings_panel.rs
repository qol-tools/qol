use gpui::App;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::settings_panel::SettingsPanel;

pub(crate) fn open(tracker: &MonitorTracker, cx: &mut App) -> anyhow::Result<()> {
    qol_gpui::settings_panel::open(
        SettingsPanel {
            plugin_id: crate::PLUGIN_ID,
            contract: crate::config::contract(),
            heading: "QoL Shot Settings",
        },
        tracker,
        &|query| match query {
            "audio_sources" => device_options(crate::platform::list_audio_sources()),
            "audio_sinks" => device_options(crate::platform::list_audio_sinks()),
            _ => Vec::new(),
        },
        cx,
    )
}

fn device_options(devices: Vec<crate::platform::AudioDevice>) -> Vec<(String, String)> {
    devices
        .into_iter()
        .map(|device| (device.value, device.label))
        .collect()
}
