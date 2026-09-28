use super::{AuthorityError, ExpectedAuthority, PageCursor, PeerId};
use qol_conventions::operations::OperationKey;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Enrollment {
        request: super::EnrollmentRequest,
    },
    Status {},
    Network {},
    Sessions {
        cursor: super::SessionCursor,
    },
    Peers {
        cursor: PageCursor,
    },
    Grants {
        peer_id: PeerId,
        cursor: PageCursor,
    },
    Tombstones {
        cursor: PageCursor,
    },
    StartSession {
        name: String,
    },
    CreatePersistent {
        name: String,
    },
    OpenPersistent {},
    Stop {
        expected: ExpectedAuthority,
    },
    Rename {
        expected: ExpectedAuthority,
        name: String,
    },
    SetGrants {
        expected: ExpectedAuthority,
        peer_id: PeerId,
        grants: Vec<OperationKey>,
    },
    Revoke {
        expected: ExpectedAuthority,
        peer_id: PeerId,
    },
    Pointz {
        request: super::PointzRequest,
    },
}

impl From<Request> for super::Request {
    fn from(value: Request) -> Self {
        match value {
            Request::Enrollment { request } => Self::Enrollment { request },
            Request::Status {} => Self::Status,
            Request::Network {} => Self::Network,
            Request::Sessions { cursor } => Self::Sessions { cursor },
            Request::Peers { cursor } => Self::Peers { cursor },
            Request::Grants { peer_id, cursor } => Self::Grants { peer_id, cursor },
            Request::Tombstones { cursor } => Self::Tombstones { cursor },
            Request::StartSession { name } => Self::StartSession { name },
            Request::CreatePersistent { name } => Self::CreatePersistent { name },
            Request::OpenPersistent {} => Self::OpenPersistent,
            Request::Stop { expected } => Self::Stop { expected },
            Request::Rename { expected, name } => Self::Rename { expected, name },
            Request::SetGrants {
                expected,
                peer_id,
                grants,
            } => Self::SetGrants {
                expected,
                peer_id,
                grants,
            },
            Request::Revoke { expected, peer_id } => Self::Revoke { expected, peer_id },
            Request::Pointz { request } => Self::Pointz { request },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Lifecycle {
    Inactive {},
    Standby {},
    Active {},
    Stopping {},
    Unavailable { error: super::Error },
    Shutdown {},
}

impl From<Lifecycle> for super::Lifecycle {
    fn from(value: Lifecycle) -> Self {
        match value {
            Lifecycle::Inactive {} => Self::Inactive,
            Lifecycle::Standby {} => Self::Standby,
            Lifecycle::Active {} => Self::Active,
            Lifecycle::Stopping {} => Self::Stopping,
            Lifecycle::Unavailable { error } => Self::Unavailable { error },
            Lifecycle::Shutdown {} => Self::Shutdown,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Error {
    Enrollment { error: super::EnrollmentFailure },
    Authority { error: AuthorityError },
    Inactive {},
    Standby {},
    AlreadyActive {},
    Stopping {},
    Shutdown {},
    RootUnavailable {},
    HostUnavailable {},
    StaleAuthority {},
    ActivationUnavailable {},
    StaleCursor {},
    InvalidCursor {},
    GrantUnavailable {},
    ReplyTooLarge {},
    PointzUnavailable {},
}

impl From<Error> for super::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Enrollment { error } => Self::Enrollment { error },
            Error::Authority { error } => Self::Authority { error },
            Error::Inactive {} => Self::Inactive,
            Error::Standby {} => Self::Standby,
            Error::AlreadyActive {} => Self::AlreadyActive,
            Error::Stopping {} => Self::Stopping,
            Error::Shutdown {} => Self::Shutdown,
            Error::RootUnavailable {} => Self::RootUnavailable,
            Error::HostUnavailable {} => Self::HostUnavailable,
            Error::StaleAuthority {} => Self::StaleAuthority,
            Error::ActivationUnavailable {} => Self::ActivationUnavailable,
            Error::StaleCursor {} => Self::StaleCursor,
            Error::InvalidCursor {} => Self::InvalidCursor,
            Error::GrantUnavailable {} => Self::GrantUnavailable,
            Error::ReplyTooLarge {} => Self::ReplyTooLarge,
            Error::PointzUnavailable {} => Self::PointzUnavailable,
        }
    }
}
