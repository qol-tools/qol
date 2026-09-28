mod transport;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
};

use tokio::sync::{mpsc, watch};

use crate::admin::{AttemptState, EnrollmentFailure, Error};
use crate::enrollment::{ExportedInvitation, TransactionId};
use crate::service::{enrollment::Invitation, PeerAuthority, PeerPin};
use crate::StoreRevision;

pub(super) const MAX_EXCHANGES: usize = 8;
const MAX_RESULTS: usize = 256;

struct Attempt {
    state: AttemptState,
    cancel: Option<watch::Sender<bool>>,
}

type Attempts = Arc<Mutex<HashMap<TransactionId, Attempt>>>;

pub(super) struct Control {
    commands: mpsc::Sender<Job>,
    attempts: Attempts,
    pub(super) loopback: bool,
}

pub(super) struct Owner {
    commands: mpsc::Receiver<Job>,
    attempts: Attempts,
}

struct Job {
    transaction: TransactionId,
    pin: PeerPin,
    endpoints: Vec<SocketAddr>,
    document: Option<ExportedInvitation>,
    cancel: watch::Receiver<bool>,
}

pub(super) fn prepare(loopback: bool) -> (Control, Owner) {
    let (commands, incoming) = mpsc::channel(MAX_EXCHANGES);
    let attempts = Arc::new(Mutex::new(HashMap::new()));
    (
        Control {
            commands,
            attempts: attempts.clone(),
            loopback,
        },
        Owner {
            commands: incoming,
            attempts,
        },
    )
}

impl Control {
    pub(super) fn admit(
        &self,
        authority: &PeerAuthority,
        expected: StoreRevision,
        transaction: TransactionId,
        document: Option<ExportedInvitation>,
        endpoints: Vec<SocketAddr>,
    ) -> Result<AttemptState, Error> {
        let invitation = document
            .as_ref()
            .map(|document| Invitation::import(document.expose()))
            .transpose()?;
        let endpoints = invitation
            .as_ref()
            .map_or(endpoints, |invitation| invitation.endpoints().to_vec());
        validate_endpoints(&endpoints, self.loopback)?;
        authority.admit_enrollment(expected, transaction, invitation.as_ref(), |pin| {
            let mut attempts = self.attempts.lock().map_err(|_| Error::HostUnavailable)?;
            if attempts
                .get(&transaction)
                .is_some_and(|attempt| attempt.cancel.is_some())
            {
                return Err(refusal(EnrollmentFailure::AlreadyRunning));
            }
            if attempts
                .values()
                .filter(|attempt| attempt.cancel.is_some())
                .count()
                >= MAX_EXCHANGES
                || (!attempts.contains_key(&transaction) && attempts.len() >= MAX_RESULTS)
            {
                return Err(refusal(EnrollmentFailure::Capacity));
            }
            let (cancel, cancelled) = watch::channel(false);
            let job = Job {
                transaction,
                pin,
                endpoints,
                document,
                cancel: cancelled,
            };
            self.commands.try_send(job).map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => refusal(EnrollmentFailure::Capacity),
                mpsc::error::TrySendError::Closed(_) => refusal(EnrollmentFailure::Unavailable),
            })?;
            attempts.insert(
                transaction,
                Attempt {
                    state: AttemptState::Queued {},
                    cancel: Some(cancel),
                },
            );
            Ok(AttemptState::Queued {})
        })
    }

    pub(super) fn attempt(&self, transaction: TransactionId) -> Result<AttemptState, Error> {
        let attempts = self.attempts.lock().map_err(|_| Error::HostUnavailable)?;
        Ok(attempts
            .get(&transaction)
            .map_or(AttemptState::Unavailable {}, |attempt| {
                attempt.state.clone()
            }))
    }

    pub(super) fn cancel(&self, transaction: TransactionId) {
        let attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(cancel) = attempts
            .get(&transaction)
            .and_then(|attempt| attempt.cancel.as_ref())
        {
            cancel.send_replace(true);
        }
    }
}

pub(super) fn validate_endpoints(endpoints: &[SocketAddr], loopback: bool) -> Result<(), Error> {
    if endpoints.is_empty() || endpoints.len() > MAX_EXCHANGES {
        return Err(refusal(EnrollmentFailure::InvalidEndpoint));
    }
    for endpoint in endpoints {
        validate_address(endpoint.ip(), loopback)?;
        if endpoint.port() == 0 {
            return Err(refusal(EnrollmentFailure::InvalidEndpoint));
        }
    }
    Ok(())
}

pub(super) fn validate_address(address: IpAddr, loopback: bool) -> Result<(), Error> {
    let IpAddr::V4(address) = address else {
        return Err(refusal(EnrollmentFailure::UnsupportedAddressFamily));
    };
    if address.is_unspecified()
        || address.is_multicast()
        || address.is_broadcast()
        || (address.is_loopback() && !loopback)
    {
        return Err(refusal(EnrollmentFailure::InvalidEndpoint));
    }
    Ok(())
}

fn refusal(error: EnrollmentFailure) -> Error {
    Error::Enrollment { error }
}

impl Owner {
    fn state(&self, transaction: TransactionId, state: AttemptState, complete: bool) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(attempt) = attempts.get_mut(&transaction) {
            attempt.state = state;
            if complete {
                attempt.cancel.take();
            }
        }
    }
}
