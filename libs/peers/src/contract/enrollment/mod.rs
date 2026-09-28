mod document;
mod identifier;
pub use document::{ExportedInvitation, InvitationDocumentError, MAX_INVITATION_BYTES};

pub use identifier::{EnrollmentIdError, InvitationId, TransactionId};

use serde::{Deserialize, Serialize};

use crate::{AuthorityLifetime, PeerId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnrollmentVersion;

impl EnrollmentVersion {
    pub const V1: Self = Self;
}

impl Serialize for EnrollmentVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}

impl<'de> Deserialize<'de> for EnrollmentVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if u8::deserialize(deserializer)? != 1 {
            return Err(serde::de::Error::custom("unsupported enrollment version"));
        }
        Ok(Self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentReceipt {
    pub invitation: InvitationId,
    pub transaction: TransactionId,
    pub inviter: PeerId,
    pub joiner: PeerId,
    #[serde(deserialize_with = "deserialize_name")]
    pub inviter_name: String,
    #[serde(deserialize_with = "deserialize_name")]
    pub joiner_name: String,
    pub inviter_lifetime: AuthorityLifetime,
    pub joiner_lifetime: AuthorityLifetime,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRequestKey {
    pub invitation: InvitationId,
    pub transaction: TransactionId,
    pub peer: PeerId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingEnrollment {
    pub key: EnrollmentRequestKey,
    pub name: String,
    pub local_lifetime: AuthorityLifetime,
    pub remote_lifetime: AuthorityLifetime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundEnrollment {
    pub key: EnrollmentRequestKey,
    pub state: OutboundEnrollmentState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutboundEnrollmentState {
    Pending {},
    Abandoned {},
    Committed { receipt: EnrollmentReceipt },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentRejection {
    InvalidInvitation,
    Expired,
    Cancelled,
    Conflict,
    Revoked,
    Capacity,
    Unavailable,
    UnknownTransaction,
}

pub(crate) fn deserialize_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let name = String::deserialize(deserializer)?;
    if name.is_empty()
        || name.len() > 256
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(serde::de::Error::custom("invalid enrollment name"));
    }
    Ok(name)
}
