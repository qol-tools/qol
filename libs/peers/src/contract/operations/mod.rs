use crate::{admin::ExpectedAuthority, session::SessionNonce, PeerId, StoreRevision};
use qol_conventions::operations::OperationKey;
use serde::{Deserialize, Serialize};

pub const MAX_ARGUMENT_BYTES: usize = 2048;
pub const MAX_BODY_BYTES: usize = 4096;
pub const MAX_RESULT_BYTES: usize = 4096;
pub const MAX_TIMEOUT_MS: u16 = 10_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OperationEpoch(pub SessionNonce);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct RequestId(pub SessionNonce);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestHandle {
    pub recipient: PeerId,
    pub epoch: OperationEpoch,
    pub sequence: StoreRevision,
    pub id: RequestId,
    pub body_digest: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationBody {
    pub version: u8,
    pub key: OperationKey,
    pub arguments: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u16,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub handle: RequestHandle,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Accepted,
    DispatchStarted,
    Acknowledged,
    Result { document: String },
    HandlerError,
    HandlerFallback,
    Refused { reason: Failure },
    CancelledBeforeDispatch,
    Unknown,
}

impl Outcome {
    pub fn terminal(&self) -> bool {
        !matches!(self, Self::Accepted | Self::DispatchStarted)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[error("peer operation refused: {self:?}")]
pub enum Failure {
    InvalidBody,
    Untrusted,
    GrantRequired,
    Unavailable,
    UpdateRequired,
    Unsupported,
    StaleSession,
    StaleAuthority,
    ChangedDeclaration,
    ChangedInstance,
    Artifact,
    OldEpoch,
    Conflict,
    Gap,
    Expired,
    Capacity,
    Busy,
    Exhausted,
    Deadline,
    Storage,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Invoke {
        expected: ExpectedAuthority,
        peer: PeerId,
        body: String,
    },
    Outcome {
        expected: ExpectedAuthority,
        handle: RequestHandle,
    },
    Cancel {
        expected: ExpectedAuthority,
        handle: RequestHandle,
    },
    Requests {
        expected: ExpectedAuthority,
        peer: PeerId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestStatus {
    pub handle: RequestHandle,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Status { status: RequestStatus },
    Requests { requests: Vec<RequestStatus> },
    Error { error: Failure },
    Unknown { handle: Option<RequestHandle> },
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invoke { .. } => "Invoke(redacted)",
            Self::Outcome { .. } => "Outcome",
            Self::Cancel { .. } => "Cancel",
            Self::Requests { .. } => "Requests",
        })
    }
}

fn default_timeout_ms() -> u16 {
    MAX_TIMEOUT_MS
}
