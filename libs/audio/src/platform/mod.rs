#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "windows"))]
mod no_loopback_capture;
#[cfg(not(target_os = "linux"))]
mod no_pulse_control;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{bluetooth_endpoints, disconnect_bluetooth, reconnect_bluetooth};
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    companion_input, effective_default, identity_for_node, is_muted, list_audio_devices,
    list_cards, list_devices, list_sinks, list_source_outputs, list_sources, resolve_audio_device,
    server_facts, set_card_profile, set_default_output, set_default_sink, set_muted,
    set_volume_percent, suspend_sink, volume_percent, Meter,
};
#[cfg(not(target_os = "windows"))]
pub(crate) use no_loopback_capture::Capture;
#[cfg(not(target_os = "linux"))]
pub(crate) use no_pulse_control::{
    list_cards, list_sinks, list_source_outputs, list_sources, server_facts, set_card_profile,
    set_default_sink, suspend_sink,
};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{bluetooth_endpoints, disconnect_bluetooth, reconnect_bluetooth};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::{
    companion_input, effective_default, identity_for_node, is_muted, list_audio_devices,
    list_devices, resolve_audio_device, set_default_output, set_muted, set_volume_percent,
    volume_percent, Meter,
};
#[cfg(target_os = "windows")]
pub use windows::{bluetooth_endpoints, disconnect_bluetooth, reconnect_bluetooth};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    companion_input, effective_default, identity_for_node, is_muted, list_audio_devices,
    list_devices, resolve_audio_device, set_default_output, set_muted, set_volume_percent,
    volume_percent, Capture, Meter,
};
