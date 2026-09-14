use gpui::AsyncApp;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::settings_panel::SettingsRuntime;

pub(crate) async fn open_from_async(tracker: MonitorTracker, cx: &AsyncApp) -> anyhow::Result<()> {
    qol_gpui::settings_panel::open_plugin_settings(
        crate::PLUGIN_ID,
        "QoL Shot Settings",
        crate::config::contract(),
        tracker,
        SettingsRuntime::new(query_options),
        cx,
    )
    .await
}

fn query_options(query: &str) -> Result<serde_json::Value, String> {
    let devices = match query {
        "audio_sources" => crate::platform::list_audio_sources(),
        "audio_sinks" => crate::platform::list_audio_sinks(),
        _ => Vec::new(),
    };
    Ok(serde_json::json!(devices))
}
