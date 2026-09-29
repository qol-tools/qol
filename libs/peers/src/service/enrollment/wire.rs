use serde::{Deserialize, Serialize};

pub(crate) use super::invitation::Token;
use crate::enrollment::{
    EnrollmentReceipt, EnrollmentRejection, EnrollmentVersion, InvitationId, TransactionId,
};
use crate::AuthorityLifetime;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub version: EnrollmentVersion,
    pub invitation: InvitationId,
    pub transaction: TransactionId,
    pub operation: RequestOperation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RequestOperation {
    Redeem {
        secret: Token,
        #[serde(deserialize_with = "crate::enrollment::deserialize_name")]
        name: String,
        lifetime: AuthorityLifetime,
    },
    Recover {},
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub version: EnrollmentVersion,
    pub invitation: InvitationId,
    pub transaction: TransactionId,
    pub outcome: ResponseOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ResponseOutcome {
    Pending {},
    Committed { receipt: EnrollmentReceipt },
    Confirmed { receipt: EnrollmentReceipt },
    Rejected { reason: EnrollmentRejection },
    Unknown {},
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Confirmation {
    pub version: EnrollmentVersion,
    pub receipt: EnrollmentReceipt,
}
