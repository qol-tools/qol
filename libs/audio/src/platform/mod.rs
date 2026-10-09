#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod pulse_only;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    bluetooth_endpoints, companion_input, disconnect_bluetooth, effective_default,
    identity_for_node, is_muted, list_audio_devices, list_cards, list_devices, list_sinks,
    list_source_outputs, list_sources, reconnect_bluetooth, resolve_audio_device, server_facts,
    set_card_profile, set_default_output, set_default_sink, set_muted, set_volume_percent,
    suspend_sink, volume_percent, Meter,
};
#[cfg(not(target_os = "linux"))]
pub(crate) use pulse_only::{
    list_cards, list_sinks, list_source_outputs, list_sources, server_facts, set_card_profile,
    set_default_sink, suspend_sink,
};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::{
    bluetooth_endpoints, companion_input, disconnect_bluetooth, effective_default,
    identity_for_node, is_muted, list_audio_devices, list_devices, reconnect_bluetooth,
    resolve_audio_device, set_default_output, set_muted, set_volume_percent, volume_percent, Meter,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    bluetooth_endpoints, companion_input, disconnect_bluetooth, effective_default,
    identity_for_node, is_muted, list_audio_devices, list_devices, reconnect_bluetooth,
    resolve_audio_device, set_default_output, set_muted, set_volume_percent, volume_percent, Meter,
};
