use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use super::{ExpectedAuthority, PageCursor};
use crate::enrollment::{
    EnrollmentReceipt, EnrollmentRejection, EnrollmentRequestKey, ExportedInvitation, InvitationId,
    TransactionId,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnrollmentRequest {
    CreateInvitation {
        expected: ExpectedAuthority,
        addresses: Vec<IpAddr>,
    },
    CancelInvitation {
        expected: ExpectedAuthority,
        invitation: InvitationId,
    },
    Pending {},
    Approve {
        expected: ExpectedAuthority,
        key: EnrollmentRequestKey,
    },
    Reject {
        expected: ExpectedAuthority,
        key: EnrollmentRequestKey,
    },
    Prepare {
        expected: ExpectedAuthority,
        document: ExportedInvitation,
    },
    Redeem {
        expected: ExpectedAuthority,
        transaction: TransactionId,
        document: ExportedInvitation,
    },
    Recover {
        expected: ExpectedAuthority,
        transaction: TransactionId,
        endpoints: Vec<SocketAddr>,
    },
    Abandon {
        expected: ExpectedAuthority,
        transaction: TransactionId,
    },
    Resume {
        expected: ExpectedAuthority,
        transaction: TransactionId,
    },
    Outbound {
        cursor: PageCursor,
    },
    Attempt {
        expected: ExpectedAuthority,
        transaction: TransactionId,
    },
}

impl EnrollmentRequest {
    pub fn is_mutation(&self) -> bool {
        !matches!(
            self,
            Self::Pending {} | Self::Outbound { .. } | Self::Attempt { .. }
        )
    }

    pub fn action_name(&self) -> &'static str {
        match self {
            Self::CreateInvitation { .. } => "create_invitation",
            Self::CancelInvitation { .. } => "cancel_invitation",
            Self::Pending {} => "pending_enrollments",
            Self::Approve { .. } => "approve_enrollment",
            Self::Reject { .. } => "reject_enrollment",
            Self::Prepare { .. } => "prepare_join",
            Self::Redeem { .. } => "redeem_join",
            Self::Recover { .. } => "recover_join",
            Self::Abandon { .. } => "abandon_join",
            Self::Resume { .. } => "resume_join",
            Self::Outbound { .. } => "outbound_enrollments",
            Self::Attempt { .. } => "enrollment_attempt",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentFailure {
    InvalidInvitation,
    InvalidEndpoint,
    UnsupportedAddressFamily,
    Unavailable,
    Capacity,
    AlreadyRunning,
    UnknownTransaction,
    Abandoned,
    Transport,
    Protocol,
    Rejected(EnrollmentRejection),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptState {
    Unavailable {},
    Queued {},
    Running {},
    Completed { receipt: EnrollmentReceipt },
    Rejected { reason: EnrollmentRejection },
    Unknown { reason: EnrollmentFailure },
}

impl AttemptState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Queued {} | Self::Running {})
    }
}
