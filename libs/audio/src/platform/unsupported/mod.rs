mod control;
mod default_output;
mod identity;
mod volume;

pub(crate) use control::{
    list_cards, list_sinks, list_source_outputs, list_sources, server_facts, set_card_profile,
    set_default_sink, suspend_sink,
};
pub(crate) use default_output::{effective_default, set_default_output};
pub(crate) use identity::{identity_for_node, list_audio_devices, resolve_audio_device};
pub(crate) use volume::{output_volume_percent, set_output_volume_percent};

use crate::devices::{Device, Direction};
use crate::AudioError;

pub(crate) fn list_devices(_direction: Direction) -> Result<Vec<Device>, AudioError> {
    Err(AudioError::Unsupported)
}
