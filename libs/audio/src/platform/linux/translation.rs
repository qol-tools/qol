use std::collections::BTreeMap;
use std::ffi::CString;

use pulseaudio::protocol::{self, port_info::PortInfo};

use crate::devices::State;
use crate::AudioError;

pub(super) fn description_or_name(description: Option<&CString>, name: &str) -> String {
    let text = description
        .map(|description| description.to_string_lossy().into_owned())
        .unwrap_or_default();
    if text.is_empty() {
        name.to_owned()
    } else {
        text
    }
}

pub(super) fn property_map(props: &protocol::Props) -> BTreeMap<String, String> {
    props
        .iter()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                String::from_utf8_lossy(value)
                    .trim_end_matches('\0')
                    .to_owned(),
            )
        })
        .collect()
}

pub(super) fn property_text(props: &protocol::Props, key: &str) -> Option<String> {
    props
        .iter()
        .find_map(|(name, value)| {
            (name.to_bytes() == key.as_bytes()).then(|| {
                String::from_utf8_lossy(value)
                    .trim_end_matches('\0')
                    .to_owned()
            })
        })
        .filter(|value| !value.is_empty())
}

pub(super) fn optional_cstring(value: Option<&CString>) -> Option<String> {
    value
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn cstring(value: &str) -> Result<CString, AudioError> {
    CString::new(value)
        .map_err(|_| AudioError::Operation("audio names cannot contain nul bytes".to_owned()))
}

pub(super) fn active_port_name(ports: &[PortInfo], active: usize) -> Option<String> {
    ports
        .get(active)
        .map(|port| port.name.to_string_lossy().into_owned())
}

pub(super) fn monitor_of_sink(index: Option<u32>) -> Option<u32> {
    index.filter(|index| *index != u32::MAX)
}

pub(super) fn state_of_sink(state: protocol::SinkState) -> State {
    match state {
        protocol::SinkState::Running => State::Running,
        protocol::SinkState::Idle => State::Idle,
        protocol::SinkState::Suspended => State::Suspended,
    }
}

pub(super) fn state_of_source(state: protocol::SourceState) -> State {
    match state {
        protocol::SourceState::Running => State::Running,
        protocol::SourceState::Idle => State::Idle,
        protocol::SourceState::Suspended => State::Suspended,
    }
}
