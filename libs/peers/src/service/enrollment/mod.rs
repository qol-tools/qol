mod exchange;
mod invitation;
pub(crate) mod wire;

pub use crate::enrollment::ExportedInvitation;
pub use invitation::Invitation;

use crate::enrollment::{EnrollmentReceipt, EnrollmentRejection, TransactionId};
use crate::AuthorityError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EnrollmentError {
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error("invalid enrollment invitation")]
    InvalidInvitation,
    #[error("enrollment randomness unavailable")]
    Randomness,
    #[error("enrollment TLS failed")]
    Transport,
    #[error("enrollment frame failed; discard the connection")]
    Framing,
    #[error("enrollment response does not match the authenticated transaction")]
    Protocol,
    #[error(
        "outbound enrollment abandoned locally; remote outcome is unchanged and may be unknown"
    )]
    Abandoned,
    #[error("enrollment rejected: {0:?}")]
    Rejected(EnrollmentRejection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnrollmentOutcome {
    Completed(EnrollmentReceipt),
    Pending(TransactionId),
    Rejected(EnrollmentRejection),
    Unknown {
        transaction: TransactionId,
        reason: EnrollmentError,
    },
}

pub(crate) fn random<const N: usize>() -> Result<[u8; N], EnrollmentError> {
    let mut bytes = [0; N];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| EnrollmentError::Randomness)?;
    Ok(bytes)
}

impl From<EnrollmentError> for crate::admin::Error {
    fn from(error: EnrollmentError) -> Self {
        use crate::admin::EnrollmentFailure;
        let error = match error {
            EnrollmentError::Authority(error) => return Self::Authority { error },
            EnrollmentError::InvalidInvitation => EnrollmentFailure::InvalidInvitation,
            EnrollmentError::Randomness => EnrollmentFailure::Unavailable,
            EnrollmentError::Transport | EnrollmentError::Framing => EnrollmentFailure::Transport,
            EnrollmentError::Protocol => EnrollmentFailure::Protocol,
            EnrollmentError::Abandoned => EnrollmentFailure::Abandoned,
            EnrollmentError::Rejected(EnrollmentRejection::UnknownTransaction) => {
                EnrollmentFailure::UnknownTransaction
            }
            EnrollmentError::Rejected(reason) => EnrollmentFailure::Rejected(reason),
        };
        Self::Enrollment { error }
    }
}
