mod activation;
mod enrollment;
pub use enrollment::{AttemptState, EnrollmentFailure, EnrollmentRequest};
mod nearby;
pub use nearby::{LinkCode, NearbyComputer, NearbyLink, NearbyRequest, NearbyState, MAX_NEARBY};
mod network;
mod pointz;
pub use network::{NetworkRevision, NetworkSummary, SessionCursor, SessionPage, SessionSummary};
pub use pointz::{PointzRequest, PointzStatus};
mod wire;

pub use activation::{ActivationId, ActivationIdError};

use qol_conventions::operations::OperationKey;
use serde::{Deserialize, Serialize};

use crate::{AuthorityError, AuthorityLifetime, AuthorityStatus, PeerId, StoreRevision};

#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod tests;

pub const PEERS_PER_PAGE: usize = 16;
pub const GRANTS_PER_PAGE: usize = 16;
pub const TOMBSTONES_PER_PAGE: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
#[serde(from = "wire::Request")]
pub enum Request {
    Enrollment {
        request: EnrollmentRequest,
    },
    Nearby {
        request: NearbyRequest,
    },
    Status,
    Enable,
    Network,
    Sessions {
        cursor: SessionCursor,
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
    OpenPersistent,
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
        request: PointzRequest,
    },
}

impl Request {
    pub fn is_mutation(&self) -> bool {
        match self {
            Self::Enrollment { request } => request.is_mutation(),
            Self::Nearby { request } => request.is_mutation(),
            Self::Pointz { request } => request.is_mutation(),
            Self::Status
            | Self::Network
            | Self::Sessions { .. }
            | Self::Peers { .. }
            | Self::Grants { .. }
            | Self::Tombstones { .. } => false,
            Self::Enable
            | Self::StartSession { .. }
            | Self::CreatePersistent { .. }
            | Self::OpenPersistent
            | Self::Stop { .. }
            | Self::Rename { .. }
            | Self::SetGrants { .. }
            | Self::Revoke { .. } => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PageCursor {
    pub authority_id: PeerId,
    pub activation_id: ActivationId,
    pub revision: StoreRevision,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
#[serde(from = "wire::Lifecycle")]
pub enum Lifecycle {
    Inactive,
    Standby,
    Active,
    Stopping,
    Unavailable { error: Error },
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoritySummary {
    pub peer_id: PeerId,
    pub activation_id: ActivationId,
    pub name: String,
    pub lifetime: AuthorityLifetime,
    pub revision: StoreRevision,
    pub status: AuthorityStatus,
    pub peer_count: u32,
    pub grant_count: u32,
    pub tombstone_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub lifecycle: Lifecycle,
    pub authority: Option<AuthoritySummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerSummary {
    pub peer_id: PeerId,
    pub name: String,
    pub grant_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    pub cursor: PageCursor,
    pub total: u32,
    pub items: Vec<T>,
    pub next: Option<PageCursor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Invitation {
        authority: ExpectedAuthority,
        invitation: crate::enrollment::InvitationId,
        document: crate::enrollment::ExportedInvitation,
    },
    PendingEnrollments {
        authority: ExpectedAuthority,
        items: Vec<crate::enrollment::PendingEnrollment>,
    },
    JoinPrepared {
        authority: ExpectedAuthority,
        transaction: crate::enrollment::TransactionId,
    },
    OutboundEnrollments {
        page: Page<crate::enrollment::OutboundEnrollment>,
    },
    EnrollmentAttempt {
        authority: ExpectedAuthority,
        transaction: crate::enrollment::TransactionId,
        state: AttemptState,
    },
    Nearby {
        authority: ExpectedAuthority,
        computers: Vec<NearbyComputer>,
    },

    Network {
        network: NetworkSummary,
    },
    Sessions {
        page: SessionPage,
    },
    Status {
        status: Status,
    },
    Peers {
        page: Page<PeerSummary>,
    },
    Grants {
        peer_id: PeerId,
        page: Page<OperationKey>,
    },
    Tombstones {
        page: Page<PeerId>,
    },
    PointzStatus {
        status: PointzStatus,
    },
    PointzDevices {
        page: Page<crate::pointz::PointzDevice>,
    },
    PointzPairing {
        pairing: crate::pointz::PointzPairing,
    },
    Changed {
        authority_id: PeerId,
        activation_id: ActivationId,
        revision: StoreRevision,
    },
    Error {
        error: Error,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
#[serde(from = "wire::Error")]
pub enum Error {
    #[error("enrollment request refused: {error:?}")]
    Enrollment { error: EnrollmentFailure },
    #[error("peer authority rejected the request: {error}")]
    Authority { error: AuthorityError },
    #[error("peer authority is not active")]
    Inactive,
    #[error("peer network cleanup is still pending")]
    Stopping,
    #[error("shadow generation is waiting for promotion")]
    Standby,
    #[error("peer authority is already active; stop it before changing lifetime")]
    AlreadyActive,
    #[error("peer host has shut down")]
    Shutdown,
    #[error("peer host data root is unavailable")]
    RootUnavailable,
    #[error("peer host state is unavailable")]
    HostUnavailable,
    #[error("authority identity or activation changed")]
    StaleAuthority,
    #[error("authority activation randomness is unavailable")]
    ActivationUnavailable,
    #[error("page authority identity, activation or revision changed")]
    StaleCursor,
    #[error("page offset is outside the projection")]
    InvalidCursor,
    #[error("operation is absent or is not explicitly exposed to peers")]
    GrantUnavailable,
    #[error("peer administration reply exceeds the local message boundary")]
    ReplyTooLarge,
    #[error("PointZ with core trust support is not installed and enabled")]
    PointzUnavailable,
}

impl From<AuthorityError> for Error {
    fn from(error: AuthorityError) -> Self {
        Self::Authority { error }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedAuthority {
    pub authority_id: PeerId,
    pub activation_id: ActivationId,
    pub revision: StoreRevision,
}

impl AuthoritySummary {
    pub fn expected(&self) -> ExpectedAuthority {
        ExpectedAuthority {
            authority_id: self.peer_id,
            activation_id: self.activation_id,
            revision: self.revision,
        }
    }
}
