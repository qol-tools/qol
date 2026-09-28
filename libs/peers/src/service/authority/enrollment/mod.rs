mod admin;
mod inbound;
mod outbound;
mod state;

#[cfg(test)]
mod tests;

pub(super) use state::{InboundReceipt, OutboundJoin, PendingInvitation};
pub(super) use state::{MAX_OUTBOUND, MAX_RECEIPTS};

use std::{net::SocketAddr, time::Duration};

use tokio::{sync::watch, time::Instant};

use super::{Inner, PeerAuthority};
use crate::enrollment::{EnrollmentRejection, InvitationId};
use crate::service::enrollment::{EnrollmentError, Invitation};
use crate::service::{EnrollmentClientConfig, EnrollmentServerConfig, PeerPin};
use state::MAX_INVITATIONS;

impl PeerAuthority {
    pub fn watch_enrollment(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub fn enrollment_server_config(&self) -> Result<EnrollmentServerConfig, EnrollmentError> {
        let identity = {
            let inner = self.lock()?;
            inner.ensure_ready()?;
            inner.state.identity.clone()
        };
        EnrollmentServerConfig::new(&identity).map_err(|_| EnrollmentError::Transport)
    }

    pub fn enrollment_client_config(
        &self,
        pin: PeerPin,
    ) -> Result<EnrollmentClientConfig, EnrollmentError> {
        let identity = {
            let inner = self.lock()?;
            inner.ensure_ready()?;
            check_remote(&inner, &pin)?;
            inner.state.identity.clone()
        };
        EnrollmentClientConfig::new(&identity, pin).map_err(|_| EnrollmentError::Transport)
    }

    pub fn create_invitation(
        &self,
        endpoints: Vec<SocketAddr>,
    ) -> Result<Invitation, EnrollmentError> {
        self.create_invitation_inner(None, endpoints)
    }

    pub fn create_invitation_checked(
        &self,
        expected: crate::StoreRevision,
        endpoints: Vec<SocketAddr>,
    ) -> Result<Invitation, EnrollmentError> {
        self.create_invitation_inner(Some(expected), endpoints)
    }

    fn create_invitation_inner(
        &self,
        expected: Option<crate::StoreRevision>,
        endpoints: Vec<SocketAddr>,
    ) -> Result<Invitation, EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if let Some(expected) = expected {
            check_revision(&inner, expected)?;
        }
        let invitation = Invitation::create(
            inner.state.identity.pin().clone(),
            inner.storage.lifetime(),
            endpoints,
        )?;
        let now = Instant::now();
        inner.invitations.retain(|entry| entry.deadline > now);
        if inner.invitations.len() >= MAX_INVITATIONS {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Capacity));
        }
        if inner
            .invitations
            .iter()
            .any(|entry| entry.id == invitation.id())
            || inner
                .state
                .receipts
                .iter()
                .any(|entry| entry.receipt.invitation == invitation.id())
            || inner
                .state
                .outbound
                .iter()
                .any(|entry| entry.invitation == invitation.id())
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        inner.invitations.push(PendingInvitation {
            id: invitation.id(),
            secret: invitation.document.secret.clone(),
            deadline: now + Duration::from_secs(120),
            reservation: None,
        });
        self.changes.send_replace(());
        Ok(invitation)
    }

    pub fn cancel_invitation(&self, id: InvitationId) -> Result<(), EnrollmentError> {
        self.cancel_invitation_inner(None, id)
    }

    pub fn cancel_invitation_checked(
        &self,
        expected: crate::StoreRevision,
        id: InvitationId,
    ) -> Result<(), EnrollmentError> {
        self.cancel_invitation_inner(Some(expected), id)
    }

    fn cancel_invitation_inner(
        &self,
        expected: Option<crate::StoreRevision>,
        id: InvitationId,
    ) -> Result<(), EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if let Some(expected) = expected {
            check_revision(&inner, expected)?;
        }
        if inner
            .state
            .receipts
            .iter()
            .any(|entry| entry.receipt.invitation == id)
        {
            return Err(EnrollmentError::Rejected(EnrollmentRejection::Conflict));
        }
        inner.invitations.retain(|entry| entry.id != id);
        self.changes.send_replace(());
        Ok(())
    }
}

fn check_remote(inner: &Inner, pin: &PeerPin) -> Result<(), EnrollmentError> {
    if pin == inner.state.identity.pin() {
        return Err(super::AuthorityError::LocalPeer.into());
    }
    if inner.state.is_revoked(pin.peer_id()) {
        return Err(EnrollmentError::Rejected(EnrollmentRejection::Revoked));
    }
    Ok(())
}

fn check_revision(inner: &Inner, expected: crate::StoreRevision) -> Result<(), EnrollmentError> {
    inner.ensure_ready()?;
    if inner.state.revision != expected {
        return Err(super::AuthorityError::StaleRevision {
            expected,
            current: inner.state.revision,
        }
        .into());
    }
    Ok(())
}
