use std::collections::HashSet;

use tokio::time::Instant;

use super::super::{
    state::{validate_name, State},
    AuthorityError,
};
use crate::enrollment::{EnrollmentReceipt, InvitationId, OutboundEnrollmentState, TransactionId};
use crate::service::{enrollment::wire::Token, PeerPin};
use crate::AuthorityLifetime;

pub(crate) const MAX_INVITATIONS: usize = 8;
pub(crate) const MAX_RECEIPTS: usize = 256;
pub(crate) const MAX_OUTBOUND: usize = 256;
pub(crate) const MAX_PENDING_OUTBOUND: usize = 32;

pub(in crate::service::authority) struct PendingInvitation {
    pub id: InvitationId,
    pub secret: Token,
    pub deadline: Instant,
    pub reservation: Option<Reservation>,
    pub nearby: Option<NearbyBinding>,
}

pub(in crate::service::authority) struct NearbyBinding {
    pub pin: PeerPin,
    pub name: String,
    pub lifetime: AuthorityLifetime,
    pub code: Option<crate::admin::LinkCode>,
    pub approval: Option<Vec<qol_conventions::operations::OperationKey>>,
}

#[derive(Clone)]
pub(in crate::service::authority) struct Reservation {
    pub pin: PeerPin,
    pub transaction: TransactionId,
    pub name: String,
    pub lifetime: AuthorityLifetime,
}

#[derive(Clone)]
pub(in crate::service::authority) struct InboundReceipt {
    pub pin: PeerPin,
    pub receipt: EnrollmentReceipt,
}

#[derive(Clone)]
pub(in crate::service) struct OutboundJoin {
    pub invitation: InvitationId,
    pub transaction: TransactionId,
    pub pin: PeerPin,
    pub name: String,
    pub local_lifetime: AuthorityLifetime,
    pub remote_lifetime: AuthorityLifetime,
    pub state: OutboundEnrollmentState,
}

impl State {
    pub(in crate::service::authority) fn validate_enrollment(&self) -> Result<(), AuthorityError> {
        if self.receipts.len() > MAX_RECEIPTS
            || self.outbound.len() > MAX_OUTBOUND
            || self
                .outbound
                .iter()
                .filter(|entry| matches!(entry.state, OutboundEnrollmentState::Pending {}))
                .count()
                > MAX_PENDING_OUTBOUND
        {
            return Err(AuthorityError::Capacity);
        }
        let local = self.identity.pin().peer_id();
        let mut transactions = HashSet::new();
        let mut invitations = HashSet::new();
        for inbound in &self.receipts {
            let receipt = &inbound.receipt;
            validate_receipt(receipt)?;
            if receipt.inviter != local
                || receipt.joiner != inbound.pin.peer_id()
                || !self
                    .peer(receipt.joiner)
                    .is_some_and(|peer| peer.pin == inbound.pin)
                || self.is_revoked(receipt.joiner)
                || !transactions.insert(receipt.transaction)
                || !invitations.insert(receipt.invitation)
            {
                return Err(AuthorityError::InvalidSnapshot);
            }
        }
        let mut outbound_peers = HashSet::new();
        for join in &self.outbound {
            validate_name(&join.name)?;
            if join.pin.peer_id() == local
                || self.is_revoked(join.pin.peer_id())
                || !transactions.insert(join.transaction)
                || !invitations.insert(join.invitation)
                || (!matches!(join.state, OutboundEnrollmentState::Abandoned {})
                    && !outbound_peers.insert(join.pin.peer_id()))
            {
                return Err(AuthorityError::InvalidSnapshot);
            }
            if let OutboundEnrollmentState::Committed { receipt } = &join.state {
                validate_receipt(receipt)?;
                if receipt.invitation != join.invitation
                    || receipt.transaction != join.transaction
                    || receipt.inviter != join.pin.peer_id()
                    || receipt.joiner != local
                    || receipt.joiner_name != join.name
                    || receipt.joiner_lifetime != join.local_lifetime
                    || receipt.inviter_lifetime != join.remote_lifetime
                    || !self
                        .peer(receipt.inviter)
                        .is_some_and(|peer| peer.pin == join.pin)
                {
                    return Err(AuthorityError::InvalidSnapshot);
                }
            } else if matches!(join.state, OutboundEnrollmentState::Pending {})
                && self.peer(join.pin.peer_id()).is_some()
            {
                return Err(AuthorityError::InvalidSnapshot);
            }
        }
        Ok(())
    }
}

pub(super) fn validate_receipt(receipt: &EnrollmentReceipt) -> Result<(), AuthorityError> {
    validate_name(&receipt.inviter_name)?;
    validate_name(&receipt.joiner_name)?;
    if receipt.inviter == receipt.joiner {
        return Err(AuthorityError::InvalidSnapshot);
    }
    Ok(())
}
