use super::{check_remote, check_revision};
use crate::admin::{Error, Page, PageCursor, PEERS_PER_PAGE};
use crate::enrollment::{
    EnrollmentRejection, EnrollmentRequestKey, OutboundEnrollment, OutboundEnrollmentState,
    TransactionId,
};
use crate::service::enrollment::{EnrollmentError, Invitation};
use crate::service::{PeerAuthority, PeerPin};
use crate::StoreRevision;

impl PeerAuthority {
    pub(in crate::service) fn check_enrollment_ready(&self) -> Result<(), EnrollmentError> {
        self.lock()?.ensure_ready()?;
        Ok(())
    }

    pub fn reject_enrollment(
        &self,
        expected: StoreRevision,
        key: EnrollmentRequestKey,
    ) -> Result<(), EnrollmentError> {
        let mut inner = self.lock()?;
        check_revision(&inner, expected)?;
        let index = inner
            .invitations
            .iter()
            .position(|entry| {
                entry.id == key.invitation
                    && entry.reservation.as_ref().is_some_and(|reservation| {
                        reservation.transaction == key.transaction
                            && reservation.pin.peer_id() == key.peer
                    })
            })
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        inner.invitations.remove(index);
        self.changes.send_replace(());
        Ok(())
    }

    pub fn admit_enrollment<T>(
        &self,
        expected: StoreRevision,
        transaction: TransactionId,
        invitation: Option<&Invitation>,
        admit: impl FnOnce(PeerPin) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let inner = self.lock()?;
        check_revision(&inner, expected)?;
        let join = inner
            .state
            .outbound
            .iter()
            .find(|entry| entry.transaction == transaction)
            .ok_or(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ))?;
        check_remote(&inner, &join.pin)?;
        if matches!(join.state, OutboundEnrollmentState::Abandoned {}) {
            return Err(EnrollmentError::Abandoned.into());
        }
        if invitation.is_some_and(|invitation| {
            invitation.inviter_pin() != &join.pin
                || invitation.id() != join.invitation
                || invitation.lifetime() != join.remote_lifetime
        }) {
            return Err(EnrollmentError::Protocol.into());
        }
        admit(join.pin.clone())
    }

    pub fn outbound_enrollment_page(
        &self,
        cursor: PageCursor,
    ) -> Result<Page<OutboundEnrollment>, Error> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        if inner.state.identity.pin().peer_id() != cursor.authority_id
            || inner.state.revision != cursor.revision
        {
            return Err(Error::StaleCursor);
        }
        let total = inner.state.outbound.len();
        let start = cursor.offset as usize;
        if start > total {
            return Err(Error::InvalidCursor);
        }
        let end = start.saturating_add(PEERS_PER_PAGE).min(total);
        let items = inner
            .state
            .outbound
            .iter()
            .skip(start)
            .take(PEERS_PER_PAGE)
            .map(|join| OutboundEnrollment {
                key: EnrollmentRequestKey {
                    invitation: join.invitation,
                    transaction: join.transaction,
                    peer: join.pin.peer_id(),
                },
                state: join.state.clone(),
            })
            .collect();
        Ok(Page {
            cursor,
            total: total as u32,
            items,
            next: (end < total).then_some(PageCursor {
                offset: end as u32,
                ..cursor
            }),
        })
    }

    pub fn check_enrollment_transaction(
        &self,
        transaction: TransactionId,
    ) -> Result<(), EnrollmentError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        if !inner
            .state
            .outbound
            .iter()
            .any(|entry| entry.transaction == transaction)
        {
            return Err(EnrollmentError::Rejected(
                EnrollmentRejection::UnknownTransaction,
            ));
        }
        Ok(())
    }
}
