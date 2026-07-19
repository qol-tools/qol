use gpui::AsyncApp;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::settings_panel::{SettingsPanel, SettingsRuntime};

pub(crate) fn open_from_async(tracker: MonitorTracker, cx: &AsyncApp) -> anyhow::Result<()> {
    qol_gpui::settings_panel::open_from_async(
        SettingsPanel {
            plugin_id: crate::PLUGIN_ID,
            contract: crate::config::contract(),
            heading: "QoL Shot Settings",
        },
        tracker,
        SettingsRuntime::new(query_options),
        cx,
    )
}

fn query_options(query: &str) -> Result<serde_json::Value, String> {
    let options = match query {
        "audio_sources" => device_options(crate::platform::list_audio_sources()),
        "audio_sinks" => device_options(crate::platform::list_audio_sinks()),
        _ => Vec::new(),
    };
    Ok(serde_json::json!(options
        .into_iter()
        .map(|(value, label)| serde_json::json!({ "value": value, "label": label }))
        .collect::<Vec<_>>()))
}

fn device_options(devices: Vec<crate::platform::AudioDevice>) -> Vec<(String, String)> {
    devices
        .into_iter()
        .map(|device| (device.value, device.label))
        .collect()
}
