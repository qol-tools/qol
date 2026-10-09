use std::fmt::{Display, Formatter};

use crate::devices::{Direction, Identity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    Unsupported,
    ServerUnavailable(String),
    Authentication(String),
    Protocol(String),
    Timeout,
    Operation(String),
}

impl AudioError {
    pub fn no_default(direction: Direction) -> Self {
        Self::Operation(
            match direction {
                Direction::Output => "there is no default sound output",
                Direction::Input => "there is no default microphone",
            }
            .to_owned(),
        )
    }

    pub fn not_present(direction: Direction, device: &Identity) -> Self {
        let noun = match direction {
            Direction::Output => "output",
            Direction::Input => "input",
        };
        Self::Operation(format!("the {noun} '{device}' is not present"))
    }
}

impl Display for AudioError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => {
                write!(formatter, "audio control is not supported on this platform")
            }
            Self::ServerUnavailable(reason) => {
                write!(formatter, "the audio server is unavailable: {reason}")
            }
            Self::Authentication(reason) => {
                write!(formatter, "audio server authentication failed: {reason}")
            }
            Self::Protocol(reason) => {
                write!(formatter, "invalid audio protocol exchange: {reason}")
            }
            Self::Timeout => write!(
                formatter,
                "the audio server did not answer within the deadline"
            ),
            Self::Operation(reason) => write!(formatter, "audio operation failed: {reason}"),
        }
    }
}

impl std::error::Error for AudioError {}
