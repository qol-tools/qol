use crate::StoreRevision;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
#[serde(from = "AuthorityErrorWire")]
pub enum AuthorityError {
    #[error("authority is faulted; close all handles and reopen the persistent store")]
    Faulted,
    #[error("store revision changed; expected {expected}, current {current}")]
    StaleRevision {
        expected: StoreRevision,
        current: StoreRevision,
    },
    #[error("store revision exhausted")]
    RevisionExhausted,
    #[error("authority capacity exceeded")]
    Capacity,
    #[error("invalid peer name")]
    InvalidName,
    #[error("peer grants require an exact canonical stable operation identity")]
    InvalidGrant,
    #[error("duplicate operation grant")]
    DuplicateGrant,
    #[error("peer is already linked")]
    DuplicatePeer,
    #[error("peer is not linked")]
    UnknownPeer,
    #[error("revoked identity requires a new locally approved relinking transition")]
    Revoked,
    #[error("local identity cannot be linked as a remote peer")]
    LocalPeer,
    #[error("authority root already exists; opening and creation are separate operations")]
    AlreadyExists,
    #[error("authority snapshot or lock is missing")]
    MissingStore,
    #[error("authority store is already locked by another writer")]
    WriterBusy,
    #[error("authority path, ownership, permissions or file identity is unsafe")]
    UnsafeStore,
    #[error("authority snapshot is invalid")]
    InvalidSnapshot,
    #[error("authority snapshot version is unsupported")]
    UnsupportedVersion,
    #[error("authority identity is invalid or cannot be generated")]
    Identity,
    #[error("private durable authority storage is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("authority storage failed; persistent state must be reconciled by reopening")]
    Storage,
    #[error("authority TLS configuration failed")]
    Transport,
    #[error("PointZ trust has not been set up on this computer")]
    PointzAbsent,
    #[error("PointZ trust was already imported")]
    PointzMigrated,
    #[error("PointZ device is not paired")]
    UnknownDevice,
}

#[derive(Deserialize)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
enum AuthorityErrorWire {
    Faulted {},
    StaleRevision {
        expected: StoreRevision,
        current: StoreRevision,
    },
    RevisionExhausted {},
    Capacity {},
    InvalidName {},
    InvalidGrant {},
    DuplicateGrant {},
    DuplicatePeer {},
    UnknownPeer {},
    Revoked {},
    LocalPeer {},
    AlreadyExists {},
    MissingStore {},
    WriterBusy {},
    UnsafeStore {},
    InvalidSnapshot {},
    UnsupportedVersion {},
    Identity {},
    UnsupportedPlatform {},
    Storage {},
    Transport {},
    PointzAbsent {},
    PointzMigrated {},
    UnknownDevice {},
}

impl From<AuthorityErrorWire> for AuthorityError {
    fn from(value: AuthorityErrorWire) -> Self {
        match value {
            AuthorityErrorWire::Faulted {} => Self::Faulted,
            AuthorityErrorWire::StaleRevision { expected, current } => {
                Self::StaleRevision { expected, current }
            }
            AuthorityErrorWire::RevisionExhausted {} => Self::RevisionExhausted,
            AuthorityErrorWire::Capacity {} => Self::Capacity,
            AuthorityErrorWire::InvalidName {} => Self::InvalidName,
            AuthorityErrorWire::InvalidGrant {} => Self::InvalidGrant,
            AuthorityErrorWire::DuplicateGrant {} => Self::DuplicateGrant,
            AuthorityErrorWire::DuplicatePeer {} => Self::DuplicatePeer,
            AuthorityErrorWire::UnknownPeer {} => Self::UnknownPeer,
            AuthorityErrorWire::Revoked {} => Self::Revoked,
            AuthorityErrorWire::LocalPeer {} => Self::LocalPeer,
            AuthorityErrorWire::AlreadyExists {} => Self::AlreadyExists,
            AuthorityErrorWire::MissingStore {} => Self::MissingStore,
            AuthorityErrorWire::WriterBusy {} => Self::WriterBusy,
            AuthorityErrorWire::UnsafeStore {} => Self::UnsafeStore,
            AuthorityErrorWire::InvalidSnapshot {} => Self::InvalidSnapshot,
            AuthorityErrorWire::UnsupportedVersion {} => Self::UnsupportedVersion,
            AuthorityErrorWire::Identity {} => Self::Identity,
            AuthorityErrorWire::UnsupportedPlatform {} => Self::UnsupportedPlatform,
            AuthorityErrorWire::Storage {} => Self::Storage,
            AuthorityErrorWire::Transport {} => Self::Transport,
            AuthorityErrorWire::PointzAbsent {} => Self::PointzAbsent,
            AuthorityErrorWire::PointzMigrated {} => Self::PointzMigrated,
            AuthorityErrorWire::UnknownDevice {} => Self::UnknownDevice,
        }
    }
}
