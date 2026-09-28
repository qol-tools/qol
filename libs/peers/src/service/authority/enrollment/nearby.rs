use std::net::SocketAddr;

use qol_conventions::operations::OperationKey;
use tokio::time::Instant;

use super::super::{
    state::{validate_grants, validate_name},
    AuthorityError, PeerAuthority,
};
use super::{check_remote, check_revision, NearbyBinding};
use crate::admin::LinkCode;
use crate::enrollment::{EnrollmentRejection, EnrollmentRequestKey, InvitationId};
use crate::service::{
    enrollment::{EnrollmentError, Invitation},
    NearbyClientConfig, PeerPin,
};
use crate::{AuthorityLifetime, PeerId, StoreRevision};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundNearby {
    pub peer_id: PeerId,
    pub name: String,
    pub code: LinkCode,
    pub confirmed: bool,
    pub redeemed: bool,
}

impl PeerAuthority {
    pub fn nearby_client_config(
        &self,
        peer: PeerId,
    ) -> Result<NearbyClientConfig, EnrollmentError> {
        let identity = {
            let inner = self.lock()?;
            inner.ensure_ready()?;
            if peer == inner.state.identity.pin().peer_id() {
                return Err(AuthorityError::LocalPeer.into());
            }
            inner.state.identity.clone()
        };
        NearbyClientConfig::new(&identity, peer).map_err(|_| EnrollmentError::Transport)
    }

    pub(in crate::service) fn nearby_identity(
        &self,
    ) -> Result<(PeerPin, String, AuthorityLifetime), EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        Ok((
            inner.state.identity.pin().clone(),
            inner.state.name.clone(),
            inner.storage.lifetime(),
        ))
    }

    pub(in crate::service) fn offer_nearby(
        &self,
        pin: &PeerPin,
        name: String,
        lifetime: AuthorityLifetime,
        endpoint: SocketAddr,
    ) -> Result<Invitation, EnrollmentError> {
        validate_name(&name)?;
        self.create_invitation_inner(
            None,
            vec![endpoint],
            Some(NearbyBinding {
                pin: pin.clone(),
                name,
                lifetime,
                code: None,
                approval: None,
            }),
        )
    }

    pub(in crate::service) fn bind_nearby(
        &self,
        invitation: InvitationId,
        pin: &PeerPin,
        code: LinkCode,
    ) -> Result<(), EnrollmentError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        let bound = inner
            .invitations
            .iter_mut()
            .find(|entry| entry.id == invitation && entry.deadline > Instant::now())
            .and_then(|entry| entry.nearby.as_mut())
            .filter(|bound| bound.pin == *pin && bound.code.is_none())
            .ok_or(EnrollmentError::Rejected(EnrollmentRejection::Cancelled))?;
        bound.code = Some(code);
        self.changes.send_replace(());
        Ok(())
    }

    pub(in crate::service) fn drop_nearby(&self, invitation: InvitationId) {
        let Ok(mut inner) = self.lock() else {
            return;
        };
        let before = inner.invitations.len();
        inner
            .invitations
            .retain(|entry| entry.id != invitation || entry.reservation.is_some());
        if inner.invitations.len() != before {
            self.changes.send_replace(());
        }
    }

    pub fn inbound_nearby(&self) -> Result<Vec<InboundNearby>, EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        let now = Instant::now();
        Ok(inner
            .invitations
            .iter()
            .filter(|entry| entry.deadline > now)
            .filter_map(|entry| {
                let bound = entry.nearby.as_ref()?;
                Some(InboundNearby {
                    peer_id: bound.pin.peer_id(),
                    name: bound.name.clone(),
                    code: bound.code?,
                    confirmed: bound.approval.is_some(),
                    redeemed: entry.reservation.is_some(),
                })
            })
            .collect())
    }

    pub fn confirm_nearby(
        &self,
        expected: StoreRevision,
        peer: PeerId,
        grants: Vec<OperationKey>,
    ) -> Result<StoreRevision, EnrollmentError> {
        validate_grants(&grants)?;
        let mut inner = self.lock()?;
        check_revision(&inner, expected)?;
        let now = Instant::now();
        let entry = inner
            .invitations
            .iter_mut()
            .find(|entry| {
                entry.deadline > now
                    && entry
                        .nearby
                        .as_ref()
                        .is_some_and(|bound| bound.pin.peer_id() == peer && bound.code.is_some())
            })
            .ok_or(EnrollmentError::Rejected(EnrollmentRejection::Cancelled))?;
        let Some(reserved) = &entry.reservation else {
            if let Some(bound) = entry.nearby.as_mut() {
                bound.approval = Some(grants);
            }
            self.changes.send_replace(());
            return Ok(inner.state.revision);
        };
        let key = EnrollmentRequestKey {
            invitation: entry.id,
            transaction: reserved.transaction,
            peer,
        };
        self.approve_locked(&mut inner, key, grants)?;
        Ok(inner.state.revision)
    }

    pub fn decline_nearby(
        &self,
        expected: StoreRevision,
        peer: PeerId,
    ) -> Result<(), EnrollmentError> {
        let mut inner = self.lock()?;
        check_revision(&inner, expected)?;
        inner.invitations.retain(|entry| {
            entry
                .nearby
                .as_ref()
                .is_none_or(|bound| bound.pin.peer_id() != peer)
        });
        self.changes.send_replace(());
        Ok(())
    }

    pub fn grant_linked(
        &self,
        peer: PeerId,
        grants: Vec<OperationKey>,
    ) -> Result<StoreRevision, AuthorityError> {
        self.mutate_current(|state| {
            let linked = state
                .peers
                .iter_mut()
                .find(|entry| entry.pin.peer_id() == peer)
                .ok_or(AuthorityError::UnknownPeer)?;
            for grant in grants {
                if !linked.grants.contains(&grant) {
                    linked.grants.push(grant);
                }
            }
            validate_grants(&linked.grants)
        })
    }

    pub(in crate::service) fn check_nearby_remote(
        &self,
        pin: &PeerPin,
    ) -> Result<(), EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        check_remote(&inner, pin)
    }
}
