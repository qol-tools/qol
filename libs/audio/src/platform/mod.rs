#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    companion_input, effective_default, identity_for_node, is_muted, list_audio_devices,
    list_cards, list_devices, list_sinks, list_source_outputs, list_sources, resolve_audio_device,
    server_facts, set_card_profile, set_default_output, set_default_sink, set_muted,
    set_volume_percent, suspend_sink, volume_percent, Meter,
};
#[cfg(not(target_os = "linux"))]
pub(crate) use unsupported::{
    companion_input, effective_default, identity_for_node, is_muted, list_audio_devices,
    list_cards, list_devices, list_sinks, list_source_outputs, list_sources, resolve_audio_device,
    server_facts, set_card_profile, set_default_output, set_default_sink, set_muted,
    set_volume_percent, suspend_sink, volume_percent, Meter,
};
