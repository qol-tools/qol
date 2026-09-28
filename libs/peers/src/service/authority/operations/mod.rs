mod body;
mod sender;
mod state;
#[cfg(test)]
mod tests;

pub use body::{body_digest, decode_body};
pub(super) use state::LinkOperations;

use super::{AuthorityError, PeerAuthority};
use crate::{
    operations::{Failure, Invocation, OperationEpoch, Outcome, RequestHandle},
    session::{AuthenticatedSession, SessionNonce},
    PeerId,
};
use state::ReceiverRecord;
use std::time::Duration;
use tokio::time::Instant;

impl From<AuthorityError> for Failure {
    fn from(error: AuthorityError) -> Self {
        match error {
            AuthorityError::UnknownPeer | AuthorityError::LocalPeer => Self::Untrusted,
            AuthorityError::Capacity => Self::Capacity,
            _ => Self::Storage,
        }
    }
}

pub(super) fn random_nonce() -> Result<SessionNonce, Failure> {
    let mut bytes = [0; 16];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| Failure::Storage)?;
    Ok(SessionNonce::from_random(bytes))
}

impl PeerAuthority {
    pub fn operation_epoch(&self, peer: PeerId) -> Result<OperationEpoch, Failure> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        Ok(inner.state.operation_link(peer)?.epoch)
    }

    pub fn elect_operation_session(&self, session: AuthenticatedSession) -> Result<(), Failure> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        inner.state.operation_link(session.remote_peer)?;
        if session.local_peer != inner.state.identity.pin().peer_id() {
            return Err(Failure::Untrusted);
        }
        inner
            .operation_sessions
            .insert(session.remote_peer, session);
        Ok(())
    }

    pub fn close_operation_session(&self, session: AuthenticatedSession) {
        let Ok(mut inner) = self.lock() else {
            return;
        };
        if inner.operation_sessions.get(&session.remote_peer) == Some(&session) {
            inner.operation_sessions.remove(&session.remote_peer);
        }
    }

    pub fn admit_operation(
        &self,
        session: AuthenticatedSession,
        invocation: &Invocation,
        declaration: String,
    ) -> Result<(Outcome, bool), Failure> {
        let body = decode_body(&invocation.body)?;
        if body_digest(&invocation.body) != invocation.handle.body_digest {
            return Err(Failure::Conflict);
        }
        let mut inner = self.lock()?;
        inner.check_operation_session(session)?;
        if invocation.handle.recipient != session.local_peer {
            return Err(Failure::Untrusted);
        }
        if let Some(outcome) = inner
            .state
            .operation_link(session.remote_peer)?
            .lookup(&invocation.handle)?
        {
            return Ok((outcome, false));
        }
        if !inner
            .state
            .peer(session.remote_peer)
            .is_some_and(|peer| peer.grants.contains(&body.key))
        {
            return Err(Failure::GrantRequired);
        }
        if declaration.len() != 43 {
            return Err(Failure::ChangedDeclaration);
        }
        let mut candidate = inner.state.clone();
        candidate.reserve_receiver(session.remote_peer)?;
        let link = candidate.operation_link_mut(session.remote_peer)?;
        link.admitted = invocation.handle.sequence;
        link.records.push(ReceiverRecord {
            handle: invocation.handle.clone(),
            declaration,
            outcome: Outcome::Accepted,
        });
        let deadline = Instant::now() + Duration::from_millis(u64::from(body.timeout_ms));
        self.publish(&mut inner, candidate)?;
        inner
            .operation_deadlines
            .insert(session.remote_peer, (invocation.handle.clone(), deadline));
        Ok((Outcome::Accepted, true))
    }

    pub fn start_operation(
        &self,
        session: AuthenticatedSession,
        invocation: &Invocation,
        declaration: &str,
    ) -> Result<Instant, Failure> {
        let body = decode_body(&invocation.body)?;
        if body_digest(&invocation.body) != invocation.handle.body_digest {
            return Err(Failure::Conflict);
        }
        let mut inner = self.lock()?;
        inner.check_operation_session(session)?;
        if !inner
            .state
            .peer(session.remote_peer)
            .is_some_and(|peer| peer.grants.contains(&body.key))
        {
            return Err(Failure::GrantRequired);
        }
        let (handle, deadline) = inner
            .operation_deadlines
            .get(&session.remote_peer)
            .ok_or(Failure::StaleSession)?;
        if handle != &invocation.handle {
            return Err(Failure::Conflict);
        }
        let deadline = *deadline;
        if Instant::now() >= deadline {
            return Err(Failure::Deadline);
        }
        let mut candidate = inner.state.clone();
        let link = candidate.operation_link_mut(session.remote_peer)?;
        let record = link
            .records
            .iter_mut()
            .find(|entry| entry.handle == invocation.handle)
            .ok_or(Failure::Expired)?;
        if record.outcome != Outcome::Accepted {
            return Err(Failure::Busy);
        }
        if record.declaration != declaration {
            return Err(Failure::ChangedDeclaration);
        }
        record.outcome = Outcome::DispatchStarted;
        self.publish(&mut inner, candidate)?;
        Ok(deadline)
    }

    pub fn settle_operation(
        &self,
        peer: PeerId,
        handle: &RequestHandle,
        mut outcome: Outcome,
    ) -> Result<Outcome, Failure> {
        if !outcome.terminal() {
            return Err(Failure::Conflict);
        }
        if let Outcome::Result { document } = &outcome {
            if document.len() > crate::operations::MAX_RESULT_BYTES
                || serde_json::from_str::<serde_json::Value>(document).is_err()
            {
                outcome = Outcome::Unknown;
            }
        }
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        let mut candidate = inner.state.clone();
        let link = candidate
            .operations
            .iter_mut()
            .find(|link| link.peer == peer)
            .ok_or(Failure::Untrusted)?;
        let record = link
            .records
            .iter_mut()
            .find(|entry| entry.handle == *handle)
            .ok_or(Failure::Expired)?;
        if record.outcome.terminal() {
            return Ok(record.outcome.clone());
        }
        if record.outcome == Outcome::Accepted
            && !matches!(
                outcome,
                Outcome::Refused { .. } | Outcome::CancelledBeforeDispatch
            )
        {
            return Err(Failure::Conflict);
        }
        record.outcome = outcome.clone();
        if serde_json::to_vec(&record)
            .map_err(|_| Failure::Storage)?
            .len()
            > state::MAX_RECORD_BYTES
        {
            record.outcome = Outcome::Unknown;
            outcome = Outcome::Unknown;
        }
        self.publish(&mut inner, candidate)?;
        inner.operation_deadlines.remove(&peer);
        Ok(outcome)
    }

    pub fn operation_outcome(
        &self,
        session: AuthenticatedSession,
        handle: &RequestHandle,
    ) -> Result<Outcome, Failure> {
        let inner = self.lock()?;
        inner.check_operation_session(session)?;
        if handle.recipient != session.local_peer {
            return Err(Failure::Untrusted);
        }
        inner
            .state
            .operation_link(session.remote_peer)?
            .lookup(handle)?
            .ok_or(Failure::Gap)
    }

    pub fn cancel_operation(
        &self,
        session: AuthenticatedSession,
        handle: &RequestHandle,
    ) -> Result<Outcome, Failure> {
        let mut inner = self.lock()?;
        inner.check_operation_session(session)?;
        if handle.recipient != session.local_peer || handle.body_digest.len() != 43 {
            return Err(Failure::InvalidBody);
        }
        let mut candidate = inner.state.clone();
        let link = candidate.operation_link_mut(session.remote_peer)?;
        match link.lookup(handle)? {
            Some(Outcome::Accepted) => {
                let record = link
                    .records
                    .iter_mut()
                    .find(|entry| entry.handle == *handle)
                    .ok_or(Failure::Expired)?;
                record.outcome = Outcome::CancelledBeforeDispatch;
            }
            Some(outcome) => return Ok(outcome),
            None => {
                link.admitted = handle.sequence;
                link.cancelled = Some(handle.clone());
            }
        }
        self.publish(&mut inner, candidate)?;
        if inner
            .operation_deadlines
            .get(&session.remote_peer)
            .is_some_and(|(current, _)| current == handle)
        {
            inner.operation_deadlines.remove(&session.remote_peer);
        }
        Ok(Outcome::CancelledBeforeDispatch)
    }
}

impl super::Inner {
    fn check_operation_session(&self, session: AuthenticatedSession) -> Result<(), Failure> {
        self.ensure_ready()?;
        self.state.operation_link(session.remote_peer)?;
        if self.operation_sessions.get(&session.remote_peer) != Some(&session) {
            return Err(Failure::StaleSession);
        }
        Ok(())
    }
}
