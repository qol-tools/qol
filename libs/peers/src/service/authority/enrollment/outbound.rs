use tokio::time::Instant;

use super::super::{
    state::{LinkedPeer, MAX_PEERS},
    AuthorityError, PeerAuthority,
};
use super::{
    check_remote,
    state::{validate_receipt, MAX_OUTBOUND, MAX_PENDING_OUTBOUND},
    OutboundJoin,
};
use crate::enrollment::{
    EnrollmentReceipt, EnrollmentRejection, EnrollmentRequestKey, OutboundEnrollment,
    OutboundEnrollmentState, TransactionId,
};
use crate::service::{
    enrollment::{random, EnrollmentError, Invitation},
    PeerPin,
};
use crate::StoreRevision;

impl PeerAuthority {
    pub fn prepare_join(
        &self,
        expected: StoreRevision,
        invitation: &Invitation,
    ) -> Result<TransactionId, EnrollmentError> {
        let transaction = TransactionId::from_random(random()?);
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if inner.state.revision != expected {
            return Err(AuthorityError::StaleRevision {
                expected,
                current: inner.state.revision,
            }
            .into());
        }
        check_remote(&inner, invitation.inviter_pin())?;
        let now = Instant::now();
        if inner.state.outbound.iter().any(|entry| {
            (entry.pin == *invitation.inviter_pin()
                && !matches!(entry.state, OutboundEnrollmentState::Abandoned {}))
                || entry.transaction == transaction
                || entry.invitation == invitation.id()
        }) || inner
            .state
            .peer(invitation.inviter_pin().peer_id())
            .is_some()
            || inner.state.receipts.iter().any(|entry| {
                entry.receipt.transaction == transaction
                    || entry.receipt.invitation == invitation.id()
            })
            || inner.invitations.iter().any(|entry| {
                entry.id == invitation.id()
                    || (entry.deadline > now
                        && entry.reservation.as_ref().is_some_and(|reserved| {
                            reserved.transaction == transaction
                                || reserved.pin == *invitation.inviter_pin()
                        }))
            })
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        if inner.state.outbound.len() >= MAX_OUTBOUND
            || inner
                .state
                .outbound
                .iter()
                .filter(|entry| matches!(entry.state, OutboundEnrollmentState::Pending {}))
                .count()
                >= MAX_PENDING_OUTBOUND
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity));
        }
        let mut candidate = inner.state.clone();
        candidate.outbound.push(OutboundJoin {
            invitation: invitation.id(),
            transaction,
            pin: invitation.inviter_pin().clone(),
            name: inner.state.name.clone(),
            local_lifetime: inner.storage.lifetime(),
            remote_lifetime: invitation.lifetime(),
            state: OutboundEnrollmentState::Pending {},
        });
        self.publish(&mut inner, candidate)?;
        Ok(transaction)
    }

    pub fn outbound_enrollments(&self) -> Result<Vec<OutboundEnrollment>, EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        Ok(inner
            .state
            .outbound
            .iter()
            .map(|join| OutboundEnrollment {
                key: EnrollmentRequestKey {
                    invitation: join.invitation,
                    transaction: join.transaction,
                    peer: join.pin.peer_id(),
                },
                state: join.state.clone(),
            })
            .collect())
    }

    pub fn abandon_join(
        &self,
        expected: StoreRevision,
        transaction: TransactionId,
    ) -> Result<StoreRevision, EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if inner.state.revision != expected {
            return Err(AuthorityError::StaleRevision {
                expected,
                current: inner.state.revision,
            }
            .into());
        }
        let index = inner
            .state
            .outbound
            .iter()
            .position(|entry| entry.transaction == transaction)
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        match &inner.state.outbound[index].state {
            OutboundEnrollmentState::Pending {} => {}
            OutboundEnrollmentState::Abandoned {} => return Ok(inner.state.revision),
            OutboundEnrollmentState::Committed { .. } => {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
            }
        }
        let mut candidate = inner.state.clone();
        candidate.outbound[index].state = OutboundEnrollmentState::Abandoned {};
        Ok(self.publish(&mut inner, candidate)?)
    }

    pub fn resume_join(
        &self,
        expected: StoreRevision,
        transaction: TransactionId,
    ) -> Result<StoreRevision, EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if inner.state.revision != expected {
            return Err(AuthorityError::StaleRevision {
                expected,
                current: inner.state.revision,
            }
            .into());
        }
        let index = inner
            .state
            .outbound
            .iter()
            .position(|entry| entry.transaction == transaction)
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        let join = &inner.state.outbound[index];
        check_remote(&inner, &join.pin)?;
        match &join.state {
            OutboundEnrollmentState::Pending {} => return Ok(inner.state.revision),
            OutboundEnrollmentState::Abandoned {} => {}
            OutboundEnrollmentState::Committed { .. } => {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
            }
        }
        let now = Instant::now();
        if inner.state.peer(join.pin.peer_id()).is_some()
            || inner.state.outbound.iter().any(|entry| {
                entry.pin == join.pin
                    && !matches!(entry.state, OutboundEnrollmentState::Abandoned {})
            })
            || inner.invitations.iter().any(|entry| {
                entry.deadline > now
                    && entry.reservation.as_ref().is_some_and(|reserved| {
                        reserved.pin == join.pin || reserved.transaction == join.transaction
                    })
            })
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        if inner
            .state
            .outbound
            .iter()
            .filter(|entry| matches!(entry.state, OutboundEnrollmentState::Pending {}))
            .count()
            >= MAX_PENDING_OUTBOUND
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity));
        }
        let mut candidate = inner.state.clone();
        candidate.outbound[index].state = OutboundEnrollmentState::Pending {};
        Ok(self.publish(&mut inner, candidate)?)
    }

    pub(in crate::service) fn outbound_join(
        &self,
        transaction: TransactionId,
        pin: &PeerPin,
    ) -> Result<OutboundJoin, EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        check_remote(&inner, pin)?;
        let join = inner
            .state
            .outbound
            .iter()
            .find(|entry| entry.transaction == transaction)
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        if join.pin != *pin {
            return Err(EnrollmentError::Protocol);
        }
        if matches!(join.state, OutboundEnrollmentState::Abandoned {}) {
            return Err(EnrollmentError::Abandoned);
        }
        Ok(join.clone())
    }

    pub(in crate::service) fn commit_join(
        &self,
        pin: &PeerPin,
        receipt: &EnrollmentReceipt,
    ) -> Result<(), EnrollmentError> {
        validate_receipt(receipt).map_err(|_| EnrollmentError::Protocol)?;
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        check_remote(&inner, pin)?;
        let index = inner
            .state
            .outbound
            .iter()
            .position(|entry| entry.transaction == receipt.transaction)
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        let join = &inner.state.outbound[index];
        if join.pin != *pin
            || receipt.inviter != pin.peer_id()
            || receipt.joiner != inner.state.identity.pin().peer_id()
            || receipt.invitation != join.invitation
            || receipt.joiner_name != join.name
            || receipt.joiner_lifetime != join.local_lifetime
            || receipt.inviter_lifetime != join.remote_lifetime
        {
            return Err(EnrollmentError::Protocol);
        }
        if matches!(join.state, OutboundEnrollmentState::Abandoned {}) {
            return Err(EnrollmentError::Abandoned);
        }
        if let OutboundEnrollmentState::Committed { receipt: stored } = &join.state {
            if stored != receipt
                || !inner
                    .state
                    .peer(pin.peer_id())
                    .is_some_and(|peer| peer.pin == *pin)
            {
                return Err(EnrollmentError::Protocol);
            }
            return Ok(());
        }
        if inner.state.peer(pin.peer_id()).is_some() {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        if inner.state.peers.len() >= MAX_PEERS {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity));
        }
        let mut candidate = inner.state.clone();
        candidate.peers.push(LinkedPeer {
            pin: pin.clone(),
            name: receipt.inviter_name.clone(),
            grants: Vec::new(),
        });
        candidate.outbound[index].state = OutboundEnrollmentState::Committed {
            receipt: receipt.clone(),
        };
        self.publish(&mut inner, candidate)?;
        Ok(())
    }
}
