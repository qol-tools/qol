use std::collections::BTreeMap;

use crate::platform;
use crate::AudioError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Idle,
    Suspended,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub properties: BTreeMap<String, String>,
    pub active_port: Option<String>,
    pub monitor_of_sink: Option<u32>,
    pub state: State,
}

pub fn list(direction: Direction) -> Result<Vec<Device>, AudioError> {
    platform::list_devices(direction)
}

pub fn default_name(direction: Direction) -> Result<Option<String>, AudioError> {
    platform::default_name(direction)
}

/// What sort of thing an output is, as the system reports it, so no plugin
/// ever reads a sink name looking for a word like bluez.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bluetooth,
    BuiltIn,
    Usb,
    Hdmi,
    Unknown,
}

/// A backend-issued stable identity for an output, distinct from the id the
/// server happens to be using now.
///
/// It is resolved from device, profile and port identity together, so two
/// outputs on the same card never collapse into one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Identity(String);

impl Identity {
    pub fn from_raw(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Identity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One output or microphone as a settings page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub identity: Identity,
    pub value: String,
    pub label: String,
    pub picture: String,
    pub kind: Kind,
}

/// What resolving a name or label against the live system concluded.
///
/// Ambiguity is refused rather than resolved by picking the first output on a
/// card, because the caller is about to change where every sound goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Resolved(AudioDevice),
    NotFound,
    Ambiguous(Vec<AudioDevice>),
}

pub fn list_outputs() -> Result<Vec<AudioDevice>, AudioError> {
    platform::list_audio_devices(Direction::Output)
}

/// Resolves an exact identity or an unambiguous label match.
pub fn resolve(direction: Direction, requested: &str) -> Result<Resolution, AudioError> {
    platform::resolve_audio_device(direction, requested)
}
