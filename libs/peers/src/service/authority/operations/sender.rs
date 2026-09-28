use super::super::PeerAuthority;
use super::{body_digest, decode_body, random_nonce};
use crate::{
    operations::{
        Failure, Invocation, OperationEpoch, Outcome, RequestHandle, RequestId, RequestStatus,
    },
    session::AuthenticatedSession,
    PeerId, StoreRevision,
};

impl PeerAuthority {
    pub fn observe_operation_epoch(
        &self,
        session: AuthenticatedSession,
        epoch: OperationEpoch,
    ) -> Result<(), Failure> {
        let mut inner = self.lock()?;
        inner.check_operation_session(session)?;
        let current = inner
            .state
            .operation_link(session.remote_peer)?
            .target_epoch;
        if current == Some(epoch) {
            return Ok(());
        }
        if current.is_some() {
            return Err(Failure::OldEpoch);
        }
        let mut candidate = inner.state.clone();
        candidate
            .operation_link_mut(session.remote_peer)?
            .target_epoch = Some(epoch);
        self.publish(&mut inner, candidate)?;
        Ok(())
    }

    pub fn prepare_operation(&self, peer: PeerId, body: String) -> Result<Invocation, Failure> {
        decode_body(&body)?;
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if !inner.operation_sessions.contains_key(&peer) {
            return Err(Failure::Unavailable);
        }
        let mut candidate = inner.state.clone();
        if candidate
            .operations
            .iter()
            .filter(|link| link.pending.is_some())
            .count()
            >= super::state::MAX_SENDERS
        {
            return Err(Failure::Capacity);
        }
        let link = candidate.operation_link_mut(peer)?;
        if link.pending.is_some() {
            return Err(Failure::Busy);
        }
        let epoch = link.target_epoch.ok_or(Failure::Unavailable)?;
        let next = link.next.value().checked_add(1).ok_or(Failure::Exhausted)?;
        let invocation = Invocation {
            handle: RequestHandle {
                recipient: peer,
                epoch,
                sequence: link.next,
                id: RequestId(random_nonce()?),
                body_digest: body_digest(&body),
            },
            body,
        };
        link.next = StoreRevision::new(next);
        link.pending = Some(invocation.clone());
        self.publish(&mut inner, candidate)?;
        Ok(invocation)
    }

    pub fn record_operation_reply(
        &self,
        session: AuthenticatedSession,
        handle: &RequestHandle,
        outcome: Outcome,
    ) -> Result<(), Failure> {
        let mut inner = self.lock()?;
        inner.check_operation_session(session)?;
        if handle.recipient != session.remote_peer {
            return Err(Failure::Untrusted);
        }
        let mut candidate = inner.state.clone();
        let link = candidate.operation_link_mut(session.remote_peer)?;
        if !link
            .pending
            .as_ref()
            .is_some_and(|entry| entry.handle == *handle)
        {
            if link
                .history
                .iter()
                .any(|entry| entry.handle == *handle && entry.outcome == outcome)
            {
                return Ok(());
            }
            return Err(Failure::Conflict);
        }
        if !outcome.terminal() {
            return Ok(());
        }
        if let Outcome::Result { document } = &outcome {
            if document.len() > crate::operations::MAX_RESULT_BYTES
                || serde_json::from_str::<serde_json::Value>(document).is_err()
            {
                return Err(Failure::InvalidBody);
            }
        }
        link.pending = None;
        link.history.push(RequestStatus {
            handle: handle.clone(),
            outcome,
        });
        if link.history.len() > super::state::MAX_PEER_RECORDS {
            link.history.remove(0);
        }
        while candidate.validate_operations().is_err() {
            let Some(link) = candidate
                .operations
                .iter_mut()
                .find(|link| !link.history.is_empty())
            else {
                return Err(Failure::Capacity);
            };
            link.history.remove(0);
        }
        self.publish(&mut inner, candidate)?;
        Ok(())
    }

    pub fn operation_requests(&self, peer: PeerId) -> Result<Vec<RequestStatus>, Failure> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        let link = inner.state.operation_link(peer)?;
        let mut requests: Vec<_> = link.history.iter().rev().take(4).cloned().collect();
        requests.reverse();
        if let Some(pending) = &link.pending {
            requests.push(RequestStatus {
                handle: pending.handle.clone(),
                outcome: Outcome::Unknown,
            });
        }
        Ok(requests)
    }

    pub fn check_operation_handle(&self, handle: &RequestHandle) -> Result<(), Failure> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        let link = inner.state.operation_link(handle.recipient)?;
        if !link.history.iter().any(|entry| entry.handle == *handle)
            && !link
                .pending
                .as_ref()
                .is_some_and(|entry| entry.handle == *handle)
        {
            return Err(Failure::Expired);
        }
        Ok(())
    }
}
