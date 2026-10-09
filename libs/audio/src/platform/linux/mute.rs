use pulseaudio::protocol;

use crate::devices::Direction;
use crate::AudioError;

use super::default_output::default_node;

pub(crate) fn is_muted(direction: Direction) -> Result<Option<bool>, AudioError> {
    super::with_connection(|connection| {
        Ok(default_node(connection, direction)?.map(|node| node.muted))
    })
}

pub(crate) fn set_muted(direction: Direction, muted: bool) -> Result<(), AudioError> {
    super::with_connection(|connection| {
        let node = default_node(connection, direction)?.ok_or_else(|| no_default(direction))?;
        let params = protocol::SetDeviceMuteParams {
            device_index: Some(node.index),
            device_name: None,
            mute: muted,
        };
        connection.request_ack(&match direction {
            Direction::Output => protocol::Command::SetSinkMute(params),
            Direction::Input => protocol::Command::SetSourceMute(params),
        })
    })
}

pub(super) fn no_default(direction: Direction) -> AudioError {
    AudioError::Operation(
        match direction {
            Direction::Output => "there is no default sound output",
            Direction::Input => "there is no default microphone",
        }
        .to_owned(),
    )
}
