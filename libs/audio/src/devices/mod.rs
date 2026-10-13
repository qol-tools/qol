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

pub fn list_inputs() -> Result<Vec<AudioDevice>, AudioError> {
    platform::list_audio_devices(Direction::Input)
}

pub fn companion_input(output: &Identity) -> Result<Option<Identity>, AudioError> {
    platform::companion_input(output)
}

/// Resolves an exact identity or an unambiguous label match.
pub fn resolve(direction: Direction, requested: &str) -> Result<Resolution, AudioError> {
    platform::resolve_audio_device(direction, requested)
}

impl Kind {
    pub fn picture(self, direction: Direction) -> &'static str {
        match (direction, self) {
            (Direction::Input, Kind::Bluetooth) => "headset",
            (Direction::Input, Kind::Usb) => "usb-mic",
            (Direction::Input, _) => "mic-default",
            (Direction::Output, Kind::Bluetooth | Kind::Usb) => "headphones",
            (Direction::Output, Kind::Hdmi) => "hdmi",
            (Direction::Output, _) => "speaker-default",
        }
    }
}

impl Resolution {
    pub fn among(devices: &[AudioDevice], requested: &str) -> Self {
        let exact = devices
            .iter()
            .filter(|device| device.identity.as_str() == requested)
            .cloned()
            .collect::<Vec<_>>();
        if !exact.is_empty() {
            return Self::narrow(exact);
        }
        let labelled = devices
            .iter()
            .filter(|device| device.label.eq_ignore_ascii_case(requested))
            .cloned()
            .collect::<Vec<_>>();
        Self::narrow(labelled)
    }

    fn narrow(mut matches: Vec<AudioDevice>) -> Self {
        if matches.len() > 1 {
            return Self::Ambiguous(matches);
        }
        match matches.pop() {
            Some(device) => Self::Resolved(device),
            None => Self::NotFound,
        }
    }
}
