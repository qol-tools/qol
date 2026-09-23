mod connection;
mod control;
mod default_output;
mod devices;
mod environment;
mod identity;
#[cfg(test)]
mod tests;
mod translation;
mod volume;

use crate::control::{Card, ServerFacts, Sink, Source, SourceOutput};
use crate::devices::{Device, Direction};
use crate::AudioError;

pub(crate) use default_output::set_default_output;
pub(crate) use identity::{identity_for_node, list_audio_devices, resolve_audio_device};
pub(crate) use volume::{output_volume_percent, set_output_volume_percent};

pub(crate) fn list_devices(direction: Direction) -> Result<Vec<Device>, AudioError> {
    with_connection(|connection| devices::list_devices(connection, direction))
}

pub(crate) fn effective_default(direction: Direction) -> Result<Option<String>, AudioError> {
    with_connection(|connection| default_output::effective_default(connection, direction))
}

pub(crate) fn list_cards() -> Result<Vec<Card>, AudioError> {
    with_connection(control::list_cards)
}

pub(crate) fn set_card_profile(card: &str, profile: &str) -> Result<(), AudioError> {
    with_connection(|connection| control::set_card_profile(connection, card, profile))
}

pub(crate) fn list_sinks() -> Result<Vec<Sink>, AudioError> {
    with_connection(control::list_sinks)
}

pub(crate) fn list_sources() -> Result<Vec<Source>, AudioError> {
    with_connection(control::list_sources)
}

pub(crate) fn list_source_outputs() -> Result<Vec<SourceOutput>, AudioError> {
    with_connection(control::list_source_outputs)
}

pub(crate) fn suspend_sink(name: &str, suspended: bool) -> Result<(), AudioError> {
    with_connection(|connection| control::suspend_sink(connection, name, suspended))
}

pub(crate) fn set_default_sink(name: &str) -> Result<(), AudioError> {
    with_connection(|connection| control::set_default_sink(connection, name))
}

pub(crate) fn server_facts() -> Result<ServerFacts, AudioError> {
    with_connection(control::server_facts)
}

fn with_connection<T>(
    operation: impl FnOnce(&mut connection::Connection) -> Result<T, AudioError>,
) -> Result<T, AudioError> {
    let mut connection = connection::Connection::connect()?;
    operation(&mut connection)
}
