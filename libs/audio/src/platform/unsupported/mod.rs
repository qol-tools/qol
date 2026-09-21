mod default_output;
mod identity;
mod volume;

pub(crate) use default_output::{
    capture_default, default_release_capability, effective_default, restore_default,
    set_default_output,
};
pub(crate) use identity::{identity_for_node, list_audio_devices, resolve_audio_device};
pub(crate) use volume::{output_volume_percent, set_output_volume_percent};

use crate::control::{Card, ServerFacts, Sink, Source, SourceOutput};
use crate::devices::{Device, Direction};
use crate::AudioError;

pub(crate) fn list_devices(_direction: Direction) -> Result<Vec<Device>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn default_name(_direction: Direction) -> Result<Option<String>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn list_cards() -> Result<Vec<Card>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn set_card_profile(_card: &str, _profile: &str) -> Result<(), AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn list_sinks() -> Result<Vec<Sink>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn list_sources() -> Result<Vec<Source>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn list_source_outputs() -> Result<Vec<SourceOutput>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn suspend_sink(_name: &str, _suspended: bool) -> Result<(), AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn set_default_sink(_name: &str) -> Result<(), AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn server_facts() -> Result<ServerFacts, AudioError> {
    Err(AudioError::Unsupported)
}
