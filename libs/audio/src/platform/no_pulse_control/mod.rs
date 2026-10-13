use crate::control::{Card, ServerFacts, Sink, Source, SourceOutput};
use crate::AudioError;

pub(crate) fn list_cards() -> Result<Vec<Card>, AudioError> {
    Err(AudioError::Unsupported)
}

pub(crate) fn set_card_profile(card: &str, profile: &str) -> Result<(), AudioError> {
    let _ = (card, profile);
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

pub(crate) fn suspend_sink(name: &str, suspended: bool) -> Result<(), AudioError> {
    let _ = (name, suspended);
    Err(AudioError::Unsupported)
}

pub(crate) fn set_default_sink(name: &str) -> Result<(), AudioError> {
    let _ = name;
    Err(AudioError::Unsupported)
}

pub(crate) fn server_facts() -> Result<ServerFacts, AudioError> {
    Err(AudioError::Unsupported)
}
