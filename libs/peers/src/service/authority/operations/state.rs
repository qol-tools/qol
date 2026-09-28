use super::super::{state::State, AuthorityError};
use crate::{
    operations::{Failure, Invocation, OperationEpoch, Outcome, RequestHandle, RequestStatus},
    PeerId, StoreRevision,
};
use serde::{Deserialize, Serialize};

pub(super) const MAX_RECORD_BYTES: usize = 8192;
pub(super) const MAX_OPERATION_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_RECEIVER_RECORDS: usize = 512;
pub(super) const MAX_PEER_RECORDS: usize = 16;
pub(super) const MAX_SENDERS: usize = 256;
const CANCEL_RESERVE: usize = 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::service::authority) struct LinkOperations {
    pub peer: PeerId,
    pub epoch: OperationEpoch,
    pub admitted: StoreRevision,
    pub target_epoch: Option<OperationEpoch>,
    pub next: StoreRevision,
    pub pending: Option<Invocation>,
    pub history: Vec<RequestStatus>,
    pub records: Vec<ReceiverRecord>,
    pub cancelled: Option<RequestHandle>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::service::authority) struct ReceiverRecord {
    pub handle: RequestHandle,
    pub declaration: String,
    pub outcome: Outcome,
}

impl LinkOperations {
    pub fn fresh(peer: PeerId) -> Result<Self, AuthorityError> {
        Ok(Self {
            peer,
            epoch: OperationEpoch(super::random_nonce().map_err(|_| AuthorityError::Identity)?),
            admitted: StoreRevision::INITIAL,
            target_epoch: None,
            next: StoreRevision::new(1),
            pending: None,
            history: Vec::new(),
            records: Vec::new(),
            cancelled: None,
        })
    }

    pub fn lookup(&self, handle: &RequestHandle) -> Result<Option<Outcome>, Failure> {
        if handle.epoch != self.epoch {
            return Err(Failure::OldEpoch);
        }
        if let Some(record) = self
            .records
            .iter()
            .find(|entry| entry.handle.sequence == handle.sequence)
        {
            if record.handle != *handle {
                return Err(Failure::Conflict);
            }
            return Ok(Some(record.outcome.clone()));
        }
        if let Some(cancelled) = self
            .cancelled
            .as_ref()
            .filter(|entry| entry.sequence == handle.sequence)
        {
            if cancelled != handle {
                return Err(Failure::Conflict);
            }
            return Ok(Some(Outcome::CancelledBeforeDispatch));
        }
        if handle.sequence <= self.admitted {
            return Err(Failure::Expired);
        }
        if self.admitted.value().checked_add(1) != Some(handle.sequence.value()) {
            return Err(Failure::Gap);
        }
        Ok(None)
    }

    pub fn active(&self) -> bool {
        self.records.iter().any(|entry| !entry.outcome.terminal())
    }
}

impl State {
    pub(in crate::service::authority) fn synchronize_operations(
        &mut self,
    ) -> Result<(), AuthorityError> {
        for peer in &self.peers {
            let id = peer.pin.peer_id();
            if !self.operations.iter().any(|link| link.peer == id) {
                self.operations.push(LinkOperations::fresh(id)?);
            }
        }
        let trusted: Vec<_> = self.peers.iter().map(|peer| peer.pin.peer_id()).collect();
        self.operations
            .retain(|link| trusted.contains(&link.peer) || link.active());
        Ok(())
    }

    pub(in crate::service::authority) fn recover_operations(&mut self) -> bool {
        let mut changed = false;
        for link in &mut self.operations {
            for record in &mut link.records {
                let recovered = match record.outcome {
                    Outcome::Accepted => Some(Outcome::CancelledBeforeDispatch),
                    Outcome::DispatchStarted => Some(Outcome::Unknown),
                    _ => None,
                };
                if let Some(outcome) = recovered {
                    record.outcome = outcome;
                    changed = true;
                }
            }
        }
        changed
    }

    pub(in crate::service::authority) fn validate_operations(
        &self,
    ) -> Result<usize, AuthorityError> {
        let mut peers = std::collections::HashSet::new();
        let mut receivers = 0;
        let mut senders = 0;
        let mut reserved = 0;
        for link in &self.operations {
            if !peers.insert(link.peer)
                || (self.peer(link.peer).is_none() && !link.active())
                || link.records.len() > MAX_PEER_RECORDS
                || link.history.len() > MAX_PEER_RECORDS
                || link.next.value() == 0
                || link
                    .records
                    .iter()
                    .filter(|record| !record.outcome.terminal())
                    .count()
                    > 1
            {
                return Err(AuthorityError::InvalidSnapshot);
            }
            receivers += link.records.len();
            senders += usize::from(link.pending.is_some());
            let cancel_size = encoded_size(&link.cancelled)?;
            if cancel_size > CANCEL_RESERVE {
                return Err(AuthorityError::Capacity);
            }
            reserved += CANCEL_RESERVE - cancel_size;
            let mut sequences = std::collections::BTreeSet::new();
            for record in &link.records {
                if record.handle.recipient != self.identity.pin().peer_id()
                    || record.handle.epoch != link.epoch
                    || record.handle.sequence > link.admitted
                    || record.handle.sequence.value() == 0
                    || record.handle.body_digest.len() != 43
                    || !sequences.insert(record.handle.sequence)
                {
                    return Err(AuthorityError::InvalidSnapshot);
                }
                let size = encoded_size(record)?;
                if size > MAX_RECORD_BYTES {
                    return Err(AuthorityError::Capacity);
                }
                if record.declaration.len() != 43 || !valid_outcome(&record.outcome) {
                    return Err(AuthorityError::InvalidSnapshot);
                }
                reserved += MAX_RECORD_BYTES - size;
            }
            let mut sent = std::collections::BTreeSet::new();
            for status in &link.history {
                if status.handle.recipient != link.peer
                    || Some(status.handle.epoch) != link.target_epoch
                    || status.handle.sequence.value() == 0
                    || status.handle.sequence >= link.next
                    || status.handle.body_digest.len() != 43
                    || !sent.insert(status.handle.sequence)
                    || !status.outcome.terminal()
                    || !valid_outcome(&status.outcome)
                {
                    return Err(AuthorityError::InvalidSnapshot);
                }
                if encoded_size(status)? > MAX_RECORD_BYTES {
                    return Err(AuthorityError::Capacity);
                }
            }
            if let Some(pending) = &link.pending {
                if pending.handle.recipient != link.peer
                    || Some(pending.handle.epoch) != link.target_epoch
                    || pending.handle.sequence.value() == 0
                    || sent.contains(&pending.handle.sequence)
                    || pending.handle.sequence.value().checked_add(1) != Some(link.next.value())
                    || super::body_digest(&pending.body) != pending.handle.body_digest
                    || super::decode_body(&pending.body).is_err()
                {
                    return Err(AuthorityError::InvalidSnapshot);
                }
                let size = encoded_size(pending)?;
                if size > MAX_RECORD_BYTES {
                    return Err(AuthorityError::Capacity);
                }
                reserved += MAX_RECORD_BYTES - size;
            }
            if let Some(cancelled) = &link.cancelled {
                if cancelled.epoch != link.epoch
                    || cancelled.sequence > link.admitted
                    || cancelled.sequence.value() == 0
                    || cancelled.body_digest.len() != 43
                    || sequences.contains(&cancelled.sequence)
                    || cancelled.recipient != self.identity.pin().peer_id()
                {
                    return Err(AuthorityError::InvalidSnapshot);
                }
            }
        }
        if self
            .peers
            .iter()
            .any(|peer| !peers.contains(&peer.pin.peer_id()))
        {
            return Err(AuthorityError::InvalidSnapshot);
        }
        let encoded = encoded_size(&self.operations)?;
        if receivers > MAX_RECEIVER_RECORDS
            || senders > MAX_SENDERS
            || encoded + reserved > MAX_OPERATION_BYTES
        {
            return Err(AuthorityError::Capacity);
        }
        Ok(reserved)
    }

    pub(super) fn operation_link(&self, peer: PeerId) -> Result<&LinkOperations, Failure> {
        if self.peer(peer).is_none() || self.is_revoked(peer) {
            return Err(Failure::Untrusted);
        }
        self.operations
            .iter()
            .find(|link| link.peer == peer)
            .ok_or(Failure::Storage)
    }

    pub(super) fn operation_link_mut(
        &mut self,
        peer: PeerId,
    ) -> Result<&mut LinkOperations, Failure> {
        self.operation_link(peer)?;
        self.operations
            .iter_mut()
            .find(|link| link.peer == peer)
            .ok_or(Failure::Storage)
    }

    pub(super) fn reserve_receiver(&mut self, peer: PeerId) -> Result<(), Failure> {
        if self.operation_link(peer)?.active() {
            return Err(Failure::Busy);
        }
        while self.operation_link(peer)?.records.len() >= MAX_PEER_RECORDS {
            let link = self.operation_link_mut(peer)?;
            let index = link
                .records
                .iter()
                .position(|entry| entry.outcome.terminal())
                .ok_or(Failure::Capacity)?;
            link.records.remove(index);
        }
        while self
            .operations
            .iter()
            .map(|link| link.records.len())
            .sum::<usize>()
            >= MAX_RECEIVER_RECORDS
        {
            let victim = self
                .operations
                .iter()
                .enumerate()
                .flat_map(|(i, link)| {
                    link.records
                        .iter()
                        .enumerate()
                        .filter(|(_, record)| record.outcome.terminal())
                        .map(move |(j, record)| ((record.handle.sequence, link.peer), i, j))
                })
                .min_by_key(|entry| entry.0)
                .ok_or(Failure::Capacity)?;
            self.operations[victim.1].records.remove(victim.2);
        }
        Ok(())
    }
}

fn encoded_size(value: &impl Serialize) -> Result<usize, AuthorityError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| AuthorityError::InvalidSnapshot)
}

fn valid_outcome(outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Result { document } => {
            document.len() <= crate::operations::MAX_RESULT_BYTES
                && serde_json::from_str::<serde_json::Value>(document).is_ok()
        }
        _ => true,
    }
}
