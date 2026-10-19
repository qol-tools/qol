mod default_output;
mod hal;
mod identity;
mod volume;

pub(crate) use default_output::{effective_default, set_default_output};
pub(crate) use identity::{
    companion_input, identity_for_node, list_audio_devices, list_devices, resolve_audio_device,
};
pub(crate) use volume::{is_muted, set_muted, set_volume_percent, volume_percent};
