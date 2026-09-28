use tokio::time::Instant;

use super::super::{
    state::{validate_name, LinkedPeer, MAX_PEERS},
    AuthorityError, PeerAuthority,
};
use super::{
    check_remote,
    state::{Reservation, MAX_RECEIPTS},
    InboundReceipt,
};
use crate::enrollment::{
    EnrollmentReceipt, EnrollmentRejection, EnrollmentRequestKey, OutboundEnrollmentState,
    PendingEnrollment,
};
use crate::service::{
    enrollment::{
        wire::{Request, RequestOperation},
        EnrollmentError,
    },
    PeerPin,
};
use crate::StoreRevision;

impl PeerAuthority {
    pub fn pending_enrollments(&self) -> Result<Vec<PendingEnrollment>, EnrollmentError> {
        self.pending_enrollments_snapshot().map(|(_, items)| items)
    }

    pub fn pending_enrollments_snapshot(
        &self,
    ) -> Result<(StoreRevision, Vec<PendingEnrollment>), EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        let now = Instant::now();
        Ok((
            inner.state.revision,
            inner
                .invitations
                .iter()
                .filter(|entry| entry.deadline > now && entry.nearby.is_none())
                .filter_map(|entry| {
                    let reserved = entry.reservation.as_ref()?;
                    if inner.state.is_revoked(reserved.pin.peer_id()) {
                        return None;
                    }
                    Some(PendingEnrollment {
                        key: EnrollmentRequestKey {
                            invitation: entry.id,
                            transaction: reserved.transaction,
                            peer: reserved.pin.peer_id(),
                        },
                        name: reserved.name.clone(),
                        local_lifetime: inner.storage.lifetime(),
                        remote_lifetime: reserved.lifetime,
                    })
                })
                .collect(),
        ))
    }

    pub fn approve_enrollment(
        &self,
        expected: StoreRevision,
        key: EnrollmentRequestKey,
    ) -> Result<EnrollmentReceipt, EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if inner.state.revision != expected {
            return Err(AuthorityError::StaleRevision {
                expected,
                current: inner.state.revision,
            }
            .into());
        }
        self.approve_locked(&mut inner, key, Vec::new())
    }

    pub(super) fn approve_locked(
        &self,
        inner: &mut super::super::Inner,
        key: EnrollmentRequestKey,
        grants: Vec<qol_conventions::operations::OperationKey>,
    ) -> Result<EnrollmentReceipt, EnrollmentError> {
        if inner.state.is_revoked(key.peer) {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Revoked));
        }
        if let Some(stored) = inner
            .state
            .receipts
            .iter()
            .find(|entry| entry.receipt.invitation == key.invitation)
        {
            if stored.receipt.transaction == key.transaction && stored.pin.peer_id() == key.peer {
                return Ok(stored.receipt.clone());
            }
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        let invitation = inner
            .invitations
            .iter()
            .find(|entry| entry.id == key.invitation)
            .ok_or(EnrollmentError::Rejected(EnrollmentRejection::Cancelled))?;
        if invitation.deadline <= Instant::now() {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Expired));
        }
        let deadline = invitation.deadline;
        let reserved = invitation
            .reservation
            .as_ref()
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        if reserved.transaction != key.transaction || reserved.pin.peer_id() != key.peer {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        check_remote(inner, &reserved.pin)?;
        if inner.state.peer(key.peer).is_some()
            || inner.state.outbound.iter().any(|entry| {
                entry.pin == reserved.pin
                    && !matches!(entry.state, OutboundEnrollmentState::Abandoned {})
            })
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        if inner.state.receipts.len() >= MAX_RECEIPTS || inner.state.peers.len() >= MAX_PEERS {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity));
        }
        let receipt = EnrollmentReceipt {
            invitation: key.invitation,
            transaction: key.transaction,
            inviter: inner.state.identity.pin().peer_id(),
            joiner: key.peer,
            inviter_name: inner.state.name.clone(),
            joiner_name: reserved.name.clone(),
            inviter_lifetime: inner.storage.lifetime(),
            joiner_lifetime: reserved.lifetime,
        };
        let mut candidate = inner.state.clone();
        candidate.peers.push(LinkedPeer {
            pin: reserved.pin.clone(),
            name: reserved.name.clone(),
            grants,
        });
        candidate.receipts.push(InboundReceipt {
            pin: reserved.pin.clone(),
            receipt: receipt.clone(),
        });
        self.publish_checked(inner, candidate, || {
            if deadline <= Instant::now() {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Expired));
            }
            Ok(())
        })?;
        inner.invitations.retain(|entry| entry.id != key.invitation);
        Ok(receipt)
    }

    pub(in crate::service) fn reserve_enrollment(
        &self,
        pin: &PeerPin,
        request: &Request,
    ) -> Result<(), EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        check_remote(&inner, pin)?;
        if let Some(stored) = inner
            .state
            .receipts
            .iter()
            .find(|entry| entry.receipt.invitation == request.invitation)
        {
            if stored.pin != *pin || stored.receipt.transaction != request.transaction {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
            }
            if let RequestOperation::Redeem { name, lifetime, .. } = &request.operation {
                if stored.receipt.joiner_name != *name
                    || stored.receipt.joiner_lifetime != *lifetime
                {
                    return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
                }
            }
            return Ok(());
        }
        let RequestOperation::Redeem {
            secret,
            name,
            lifetime,
        } = &request.operation
        else {
            return Err(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ));
        };
        validate_name(name)?;
        if inner.state.receipts.len() >= MAX_RECEIPTS || inner.state.peers.len() >= MAX_PEERS {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity));
        }
        let now = Instant::now();
        if inner.state.peer(pin.peer_id()).is_some()
            || inner
                .state
                .receipts
                .iter()
                .any(|entry| entry.receipt.transaction == request.transaction)
            || inner.state.outbound.iter().any(|entry| {
                entry.transaction == request.transaction
                    || entry.invitation == request.invitation
                    || (entry.pin == *pin
                        && !matches!(entry.state, OutboundEnrollmentState::Abandoned {}))
            })
            || inner.invitations.iter().any(|entry| {
                entry.id != request.invitation
                    && entry.deadline > now
                    && entry.reservation.as_ref().is_some_and(|reserved| {
                        reserved.transaction == request.transaction || reserved.pin == *pin
                    })
            })
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        let invitation = inner
            .invitations
            .iter_mut()
            .find(|entry| entry.id == request.invitation)
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::InvalidInvitation,
            ))?;
        if invitation.deadline <= Instant::now() {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Expired));
        }
        if !invitation.secret.matches(secret) {
            return Err(EnrollmentError::Rejected(
                EnrollmentRejection::InvalidInvitation,
            ));
        }
        if let Some(bound) = &invitation.nearby {
            if bound.code.is_none()
                || bound.pin != *pin
                || bound.name != *name
                || bound.lifetime != *lifetime
            {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
            }
        }
        if let Some(reserved) = &invitation.reservation {
            if reserved.pin != *pin
                || reserved.transaction != request.transaction
                || reserved.name != *name
                || reserved.lifetime != *lifetime
            {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
            }
            return Ok(());
        }
        invitation.reservation = Some(Reservation {
            pin: pin.clone(),
            transaction: request.transaction,
            name: name.clone(),
            lifetime: *lifetime,
        });
        let approval = invitation
            .nearby
            .as_ref()
            .and_then(|bound| bound.approval.clone());
        self.changes.send_replace(());
        if let Some(grants) = approval {
            let key = EnrollmentRequestKey {
                invitation: request.invitation,
                transaction: request.transaction,
                peer: pin.peer_id(),
            };
            self.approve_locked(&mut inner, key, grants)?;
        }
        Ok(())
    }

    pub(in crate::service) async fn wait_enrollment(
        &self,
        pin: &PeerPin,
        key: EnrollmentRequestKey,
    ) -> Result<EnrollmentReceipt, EnrollmentError> {
        let mut changes = self.watch_enrollment();
        loop {
            let deadline = {
                let inner = self.lock()?;
                inner.ensure_ready()?;
                check_remote(&inner, pin)?;
                if let Some(receipt) = matching_receipt(&inner, pin, key)? {
                    return Ok(receipt);
                }
                let invitation = inner
                    .invitations
                    .iter()
                    .find(|entry| entry.id == key.invitation)
                    .ok_or(EnrollmentError::Rejected(EnrollmentRejection::Cancelled))?;
                let reserved = invitation
                    .reservation
                    .as_ref()
                    .ok_or(EnrollmentError::Rejected(
                        EnrollmentRejection::UnknownTransaction,
                    ))?;
                if reserved.pin != *pin || reserved.transaction != key.transaction {
                    return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
                }
                invitation.deadline
            };
            if deadline <= Instant::now() {
                return Err(EnrollmentError::Rejected(EnrollmentRejection::Expired));
            }
            let _ = tokio::time::timeout_at(deadline, changes.changed()).await;
        }
    }

    pub(in crate::service) fn checked_inbound_receipt(
        &self,
        pin: &PeerPin,
        key: EnrollmentRequestKey,
    ) -> Result<EnrollmentReceipt, EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        check_remote(&inner, pin)?;
        matching_receipt(&inner, pin, key)?.ok_or(EnrollmentError::Rejected(
            EnrollmentRejection::UnknownTransaction,
        ))
    }
}

fn matching_receipt(
    inner: &super::super::Inner,
    pin: &PeerPin,
    key: EnrollmentRequestKey,
) -> Result<Option<EnrollmentReceipt>, EnrollmentError> {
    if pin.peer_id() != key.peer {
        return Err(EnrollmentError::Protocol);
    }
    let Some(stored) = inner
        .state
        .receipts
        .iter()
        .find(|entry| entry.receipt.invitation == key.invitation)
    else {
        return Ok(None);
    };
    if stored.pin != *pin
        || stored.receipt.transaction != key.transaction
        || !inner
            .state
            .peer(key.peer)
            .is_some_and(|peer| peer.pin == *pin)
    {
        return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
    }
    Ok(Some(stored.receipt.clone()))
}
