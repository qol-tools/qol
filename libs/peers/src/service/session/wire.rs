use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{session::SessionNonce, PeerId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Version;

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u8::deserialize(deserializer)? {
            1 => Ok(Self),
            _ => Err(serde::de::Error::custom(
                "unsupported normal session version",
            )),
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Message {
    Hello {
        version: Version,
        sender: PeerId,
        recipient: PeerId,
        nonce: SessionNonce,
    },
    OperationEpoch {
        version: Version,
        sender_nonce: SessionNonce,
        recipient_nonce: SessionNonce,
        epoch: crate::operations::OperationEpoch,
    },
    Operation {
        version: Version,
        sender_nonce: SessionNonce,
        recipient_nonce: SessionNonce,
        message: crate::service::network::operations::OperationMessage,
    },
    Heartbeat {
        version: Version,
        sender_nonce: SessionNonce,
        recipient_nonce: SessionNonce,
    },
}

impl std::fmt::Debug for Message {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Hello { .. } => "Hello",
            Self::Heartbeat { .. } => "Heartbeat",
            Self::OperationEpoch { .. } => "OperationEpoch",
            Self::Operation { .. } => "Operation(redacted)",
        })
    }
}
