use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

pub const MAX_INVITATION_BYTES: usize = 4096;

#[derive(Clone, Eq, PartialEq)]
pub struct ExportedInvitation(Zeroizing<String>);

impl ExportedInvitation {
    pub fn from_owned(value: Zeroizing<String>) -> Result<Self, InvitationDocumentError> {
        if value.len() > MAX_INVITATION_BYTES || !value.starts_with("qol-link:") {
            return Err(InvitationDocumentError);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ExportedInvitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExportedInvitation([REDACTED])")
    }
}

impl Serialize for ExportedInvitation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for ExportedInvitation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = ExportedInvitation;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a bounded invitation document")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.len() > MAX_INVITATION_BYTES || !value.starts_with("qol-link:") {
                    return Err(E::custom("invalid invitation document"));
                }
                Ok(ExportedInvitation(Zeroizing::new(value.to_owned())))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                ExportedInvitation::from_owned(Zeroizing::new(value))
                    .map_err(|_| E::custom("invalid invitation document"))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid invitation document")]
pub struct InvitationDocumentError;
