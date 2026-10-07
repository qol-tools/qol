use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use qol_conventions::operations::OperationKey;
use tokio::{
    net::TcpStream,
    sync::watch,
    time::{timeout, Instant},
};

use super::{refusal, transport::failure, validate_endpoints, Control, MAX_EXCHANGES};
use crate::admin::{AttemptState, EnrollmentFailure, Error, LinkCode, NearbyLink, NearbyState};
use crate::enrollment::{EnrollmentRejection, TransactionId};
use crate::service::enrollment::{EnrollmentError, Invitation, NearbyOffer};
use crate::service::{network::discovery::cancelled, PeerAuthority};
use crate::{PeerId, StoreRevision};

const IO_DEADLINE: Duration = Duration::from_secs(5);
const OFFER_DEADLINE: Duration = Duration::from_secs(30);
const CONFIRM_WINDOW: Duration = Duration::from_secs(110);
const MAX_OFFERS: usize = 8;

pub(super) struct OfferJob {
    pub peer: PeerId,
    endpoints: Vec<SocketAddr>,
    cancel: watch::Receiver<bool>,
}

enum OfferState {
    Connecting {
        cancel: watch::Sender<bool>,
        grants: Vec<OperationKey>,
    },
    Ready {
        invitation: Box<Invitation>,
        code: LinkCode,
        deadline: Instant,
    },
    Confirmed {
        transaction: TransactionId,
        code: LinkCode,
    },
    Failed {
        error: EnrollmentFailure,
    },
}

#[derive(Clone, Default)]
pub(super) struct Offers(Arc<Mutex<BTreeMap<PeerId, (String, OfferState)>>>);

impl Offers {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<PeerId, (String, OfferState)>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(super) fn finished(
        &self,
        peer: PeerId,
        result: Result<NearbyOffer, EnrollmentError>,
    ) -> Option<Vec<OperationKey>> {
        let mut offers = self.lock();
        let (_, state) = offers.get_mut(&peer)?;
        let OfferState::Connecting { grants, .. } = state else {
            return None;
        };
        let grants = std::mem::take(grants);
        match result {
            Ok(offer) => {
                *state = OfferState::Ready {
                    invitation: Box::new(offer.invitation),
                    code: offer.code,
                    deadline: Instant::now() + CONFIRM_WINDOW,
                };
                Some(grants)
            }
            Err(error) => {
                *state = OfferState::Failed {
                    error: failure(error),
                };
                None
            }
        }
    }

    pub(super) fn close(&self) {
        for (_, state) in self.lock().values_mut() {
            if matches!(
                state,
                OfferState::Connecting { .. } | OfferState::Ready { .. }
            ) {
                *state = OfferState::Failed {
                    error: EnrollmentFailure::Unavailable,
                };
            }
        }
    }
}

impl Control {
    pub(in crate::service::network) fn link_nearby(
        &self,
        authority: &PeerAuthority,
        peer: PeerId,
        name: String,
        endpoints: Vec<SocketAddr>,
        grants: Vec<OperationKey>,
    ) -> Result<(), Error> {
        validate_endpoints(&endpoints, self.loopback)?;
        authority.nearby_client_config(peer)?;
        let mut offers = self.offers.lock();
        match offers.get(&peer).map(|(_, state)| state) {
            Some(OfferState::Connecting { .. }) => {
                return Err(refusal(EnrollmentFailure::AlreadyRunning));
            }
            Some(OfferState::Confirmed { transaction, .. })
                if self.attempt(*transaction)?.is_active() =>
            {
                return Err(refusal(EnrollmentFailure::AlreadyRunning));
            }
            Some(_) => {}
            None if offers.len() >= MAX_OFFERS => {
                let Some(stale) = offers
                    .iter()
                    .find(|(_, (_, state))| matches!(state, OfferState::Failed { .. }))
                    .map(|(peer, _)| *peer)
                else {
                    return Err(refusal(EnrollmentFailure::Capacity));
                };
                offers.remove(&stale);
            }
            None => {}
        }
        let (cancel, cancelled) = watch::channel(false);
        self.offer_commands
            .try_send(OfferJob {
                peer,
                endpoints,
                cancel: cancelled,
            })
            .map_err(|error| match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    refusal(EnrollmentFailure::Capacity)
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    refusal(EnrollmentFailure::Unavailable)
                }
            })?;
        offers.insert(peer, (name, OfferState::Connecting { cancel, grants }));
        Ok(())
    }

    pub(in crate::service::network) fn confirm_nearby(
        &self,
        authority: &PeerAuthority,
        expected: StoreRevision,
        peer: PeerId,
        grants: Vec<OperationKey>,
    ) -> Result<AttemptState, Error> {
        let mut offers = self.offers.lock();
        let Some((
            name,
            OfferState::Ready {
                invitation,
                code,
                deadline,
            },
        )) = offers.get(&peer)
        else {
            return Err(refusal(EnrollmentFailure::UnknownTransaction));
        };
        if *deadline <= Instant::now() {
            return Err(refusal(EnrollmentFailure::Rejected(
                EnrollmentRejection::Expired,
            )));
        }
        let code = *code;
        let name = name.clone();
        let document = invitation.export()?;
        let transaction = authority.prepare_nearby_join(expected, invitation)?;
        let revision = authority.projection()?.revision;
        offers.insert(peer, (name, OfferState::Confirmed { transaction, code }));
        drop(offers);
        self.admit(
            authority,
            revision,
            transaction,
            Some(document),
            Vec::new(),
            grants,
        )
    }

    pub(super) fn accept_offer(
        &self,
        authority: &PeerAuthority,
        peer: PeerId,
        grants: Vec<OperationKey>,
    ) {
        let result = match authority.projection() {
            Ok(projection) => self
                .confirm_nearby(authority, projection.revision, peer, grants)
                .map(drop),
            Err(_) => Err(refusal(EnrollmentFailure::Unavailable)),
        };
        let Err(error) = result else {
            return;
        };
        let error = match error {
            Error::Enrollment { error } => error,
            _ => EnrollmentFailure::Unavailable,
        };
        if let Some((_, state @ OfferState::Ready { .. })) = self.offers.lock().get_mut(&peer) {
            *state = OfferState::Failed { error };
        }
    }

    pub(in crate::service::network) fn decline_nearby(
        &self,
        peer: PeerId,
    ) -> Option<TransactionId> {
        match self.offers.lock().remove(&peer)?.1 {
            OfferState::Connecting { cancel, .. } => {
                cancel.send_replace(true);
                None
            }
            OfferState::Confirmed { transaction, .. } => {
                self.cancel(transaction);
                Some(transaction)
            }
            OfferState::Ready { .. } | OfferState::Failed { .. } => None,
        }
    }

    pub(in crate::service::network) fn nearby_links(
        &self,
    ) -> Result<Vec<(PeerId, String, NearbyLink)>, Error> {
        let offers = self.offers.lock();
        let now = Instant::now();
        let mut links = Vec::new();
        for (peer, (name, state)) in offers.iter() {
            let link = match state {
                OfferState::Connecting { .. } => NearbyLink {
                    code: None,
                    state: NearbyState::Connecting {},
                },
                OfferState::Ready { code, deadline, .. } if *deadline <= now => NearbyLink {
                    code: Some(*code),
                    state: NearbyState::Failed {
                        error: EnrollmentFailure::Rejected(EnrollmentRejection::Expired),
                    },
                },
                OfferState::Ready { code, .. } => NearbyLink {
                    code: Some(*code),
                    state: NearbyState::Confirm {},
                },
                OfferState::Confirmed { transaction, code } => {
                    let state = match self.attempt(*transaction)? {
                        AttemptState::Queued {} | AttemptState::Running {} => {
                            NearbyState::WaitingForPeer {}
                        }
                        AttemptState::Completed { .. } => continue,
                        AttemptState::Rejected { reason } => NearbyState::Failed {
                            error: EnrollmentFailure::Rejected(reason),
                        },
                        AttemptState::Unknown { reason } => NearbyState::Failed { error: reason },
                        AttemptState::Unavailable {} => NearbyState::Failed {
                            error: EnrollmentFailure::Unavailable,
                        },
                    };
                    NearbyLink {
                        code: Some(*code),
                        state,
                    }
                }
                OfferState::Failed { error } => NearbyLink {
                    code: None,
                    state: NearbyState::Failed { error: *error },
                },
            };
            links.push((*peer, name.clone(), link));
        }
        Ok(links)
    }
}

pub(super) async fn offer(
    authority: PeerAuthority,
    mut job: OfferJob,
) -> Result<NearbyOffer, EnrollmentError> {
    let work = async {
        let config = authority.nearby_client_config(job.peer)?;
        let mut connected = None;
        for endpoint in &job.endpoints {
            if let Ok(Ok(stream)) = timeout(IO_DEADLINE, TcpStream::connect(endpoint)).await {
                connected = Some(stream);
                break;
            }
        }
        let stream = connected.ok_or(EnrollmentError::Transport)?;
        let connection = timeout(IO_DEADLINE, config.connect(stream))
            .await
            .map_err(|_| EnrollmentError::Transport)?
            .map_err(|_| EnrollmentError::Transport)?;
        authority.request_nearby(connection).await
    };
    tokio::select! {
        biased;
        () = cancelled(&mut job.cancel) => Err(EnrollmentError::Abandoned),
        result = timeout(OFFER_DEADLINE, work) => result.unwrap_or(Err(EnrollmentError::Transport)),
    }
}

const _: () = assert!(MAX_OFFERS <= MAX_EXCHANGES);
