#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    capture_default, default_name, default_release_capability, effective_default,
    identity_for_node, list_audio_devices, list_cards, list_devices, list_sinks,
    list_source_outputs, list_sources, output_volume_percent, resolve_audio_device,
    restore_default, server_facts, set_card_profile, set_default_output, set_default_sink,
    set_output_volume_percent, suspend_sink,
};
#[cfg(not(target_os = "linux"))]
pub(crate) use unsupported::{
    capture_default, default_name, default_release_capability, effective_default,
    identity_for_node, list_audio_devices, list_cards, list_devices, list_sinks,
    list_source_outputs, list_sources, output_volume_percent, resolve_audio_device,
    restore_default, server_facts, set_card_profile, set_default_output, set_default_sink,
    set_output_volume_percent, suspend_sink,
};
