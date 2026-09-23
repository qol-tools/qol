use anyhow::{anyhow, Context, Result};
use qol_audio::control::Sink;
use qol_audio::AudioError;

pub(crate) const BLUETOOTH_SINK_PREFIX: &str = "bluez_output.";
pub(crate) const BLUETOOTH_SOURCE_PREFIX: &str = "bluez_input.";
pub(crate) const BLUETOOTH_CARD_PREFIX: &str = "bluez_card.";

pub(crate) fn device_id(address: &str) -> String {
    address.replace(':', "_")
}

pub(crate) fn sink_prefix(address: &str) -> String {
    format!("{BLUETOOTH_SINK_PREFIX}{}", device_id(address))
}

pub(crate) fn source_prefix(address: &str) -> String {
    format!("{BLUETOOTH_SOURCE_PREFIX}{}", device_id(address))
}

pub(crate) fn card_name(address: &str) -> String {
    format!("{BLUETOOTH_CARD_PREFIX}{}", device_id(address))
}

pub(crate) fn sink_matching<'a>(sinks: &'a [Sink], prefix: &str) -> Option<&'a Sink> {
    sinks.iter().find(|sink| sink.name.starts_with(prefix))
}

pub(crate) async fn request<T, F>(label: &'static str, call: F) -> Result<T>
where
    F: FnOnce() -> std::result::Result<T, AudioError> + Send + 'static,
    T: Send + 'static,
{
    let joined = tokio::task::spawn_blocking(call).await;
    let outcome = joined.map_err(|error| {
        anyhow!("the bundled audio client task for {label} did not complete: {error}")
    })?;
    outcome.with_context(|| format!("the bundled audio client could not {label}"))
}
