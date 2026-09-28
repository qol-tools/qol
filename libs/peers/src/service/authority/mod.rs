mod connection;
mod enrollment;
mod error;
pub mod operations;
mod pointz;
mod snapshot;
mod state;
mod storage;

#[cfg(test)]
mod tests;

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::SystemTime,
};

use qol_conventions::operations::OperationKey;

use super::{Identity, NormalServerConfig, PeerPin, TrustPolicy};
use crate::{AuthorityProjection, AuthorityStatus, PeerId, PeerProjection, StoreRevision};
use state::{validate_grant, validate_grants, validate_name, State, MAX_TOMBSTONES};
use storage::Storage;

#[cfg(test)]
use state::{LinkedPeer, MAX_PEERS};

pub use error::AuthorityError;

#[derive(Clone)]
pub struct PeerAuthority {
    inner: Arc<Mutex<Inner>>,
    changes: tokio::sync::watch::Sender<()>,
}

#[derive(Clone)]
pub struct WeakPeerAuthority {
    inner: std::sync::Weak<Mutex<Inner>>,
    changes: tokio::sync::watch::Sender<()>,
}

impl WeakPeerAuthority {
    pub fn upgrade(&self) -> Option<PeerAuthority> {
        Some(PeerAuthority {
            inner: self.inner.upgrade()?,
            changes: self.changes.clone(),
        })
    }
}

struct Inner {
    state: State,
    storage: Storage,
    status: AuthorityStatus,
    invitations: Vec<enrollment::PendingInvitation>,
    operation_sessions: std::collections::BTreeMap<PeerId, crate::session::AuthenticatedSession>,
    operation_deadlines: std::collections::BTreeMap<
        PeerId,
        (crate::operations::RequestHandle, tokio::time::Instant),
    >,
}

impl PeerAuthority {
    pub fn session(name: String, now: SystemTime) -> Result<Self, AuthorityError> {
        Self::fresh(name, now, Storage::Session)
    }

    pub fn create_persistent(
        root: &Path,
        name: String,
        now: SystemTime,
    ) -> Result<Self, AuthorityError> {
        validate_name(&name)?;
        Self::fresh(name, now, Storage::create(root)?)
    }

    pub fn open_persistent(root: &Path, now: SystemTime) -> Result<Self, AuthorityError> {
        Self::open_with_storage(Storage::open(root)?, now)
    }

    fn open_with_storage(mut storage: Storage, now: SystemTime) -> Result<Self, AuthorityError> {
        let bytes = storage.read()?;
        let (mut state, renewed) = snapshot::decode(&bytes, now)?;
        let recovered = state.recover_operations();
        state.synchronize_operations()?;
        if renewed || recovered {
            state.revision = next_revision(state.revision)?;
            storage.commit(&state)?;
        }
        Ok(Self::from_state(state, storage))
    }

    fn fresh(name: String, now: SystemTime, mut storage: Storage) -> Result<Self, AuthorityError> {
        validate_name(&name)?;
        let state = State {
            identity: Arc::new(Identity::generate(now).map_err(|_| AuthorityError::Identity)?),
            name,
            revision: StoreRevision::INITIAL,
            peers: Vec::new(),
            tombstones: Vec::new(),
            receipts: Vec::new(),
            outbound: Vec::new(),
            operations: Vec::new(),
            pointz: None,
        };
        state.validate()?;
        storage.commit(&state)?;
        Ok(Self::from_state(state, storage))
    }

    fn from_state(state: State, storage: Storage) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state,
                storage,
                status: AuthorityStatus::Ready,
                invitations: Vec::new(),
                operation_sessions: Default::default(),
                operation_deadlines: Default::default(),
            })),
            changes: tokio::sync::watch::channel(()).0,
        }
    }

    pub fn downgrade(&self) -> WeakPeerAuthority {
        WeakPeerAuthority {
            inner: Arc::downgrade(&self.inner),
            changes: self.changes.clone(),
        }
    }

    pub fn projection(&self) -> Result<AuthorityProjection, AuthorityError> {
        let inner = self.lock()?;
        let state = &inner.state;
        Ok(AuthorityProjection {
            peer_id: state.identity.pin().peer_id(),
            name: state.name.clone(),
            lifetime: inner.storage.lifetime(),
            revision: state.revision,
            status: inner.status,
            peers: state
                .peers
                .iter()
                .map(|peer| PeerProjection {
                    peer_id: peer.pin.peer_id(),
                    name: peer.name.clone(),
                    grants: peer.grants.clone(),
                })
                .collect(),
            tombstones: state.tombstones.iter().map(PeerPin::peer_id).collect(),
        })
    }

    pub fn local_pin(&self) -> Result<PeerPin, AuthorityError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        Ok(inner.state.identity.pin().clone())
    }

    pub fn server_config(&self) -> Result<NormalServerConfig, AuthorityError> {
        let inner = self.lock()?;
        inner.ensure_ready()?;
        let identity = inner.state.identity.clone();
        drop(inner);
        NormalServerConfig::new(&identity, Arc::new(self.clone()))
            .map_err(|_| AuthorityError::Transport)
    }

    pub fn has_grant(&self, peer: PeerId, grant: &OperationKey) -> bool {
        let Ok(inner) = self.lock() else {
            return false;
        };
        inner.status == AuthorityStatus::Ready
            && validate_grant(grant).is_ok()
            && !inner.state.is_revoked(peer)
            && inner
                .state
                .peer(peer)
                .is_some_and(|linked| linked.grants.contains(grant))
    }

    pub fn rename(
        &self,
        expected: StoreRevision,
        name: String,
    ) -> Result<StoreRevision, AuthorityError> {
        self.mutate(expected, |state| {
            validate_name(&name)?;
            state.name = name;
            Ok(())
        })
    }

    pub fn set_grants(
        &self,
        expected: StoreRevision,
        peer: PeerId,
        grants: Vec<OperationKey>,
    ) -> Result<StoreRevision, AuthorityError> {
        self.mutate(expected, |state| {
            validate_grants(&grants)?;
            if state.is_revoked(peer) {
                return Err(AuthorityError::Revoked);
            }
            let linked = state
                .peers
                .iter_mut()
                .find(|entry| entry.pin.peer_id() == peer)
                .ok_or(AuthorityError::UnknownPeer)?;
            linked.grants = grants;
            Ok(())
        })
    }

    pub fn revoke(
        &self,
        expected: StoreRevision,
        peer: PeerId,
    ) -> Result<StoreRevision, AuthorityError> {
        self.mutate(expected, |state| {
            if state.is_revoked(peer) {
                return Err(AuthorityError::Revoked);
            }
            if state.tombstones.len() == MAX_TOMBSTONES {
                return Err(AuthorityError::Capacity);
            }
            let index = state
                .peers
                .iter()
                .position(|entry| entry.pin.peer_id() == peer)
                .ok_or(AuthorityError::UnknownPeer)?;
            let removed = state.peers.remove(index);
            state.tombstones.push(removed.pin);
            state
                .receipts
                .retain(|receipt| receipt.pin.peer_id() != peer);
            state.outbound.retain(|join| join.pin.peer_id() != peer);
            Ok(())
        })
    }

    #[cfg(test)]
    pub(in crate::service) fn insert_link(
        &self,
        expected: StoreRevision,
        pin: PeerPin,
        name: String,
    ) -> Result<StoreRevision, AuthorityError> {
        self.mutate(expected, |state| {
            validate_name(&name)?;
            let peer = pin.peer_id();
            if peer == state.identity.pin().peer_id() {
                return Err(AuthorityError::LocalPeer);
            }
            if state.is_revoked(peer) {
                return Err(AuthorityError::Revoked);
            }
            if state.peer(peer).is_some() {
                return Err(AuthorityError::DuplicatePeer);
            }
            if state.peers.len() == MAX_PEERS {
                return Err(AuthorityError::Capacity);
            }
            state.peers.push(LinkedPeer {
                pin,
                name,
                grants: Vec::new(),
            });
            Ok(())
        })
    }

    fn mutate(
        &self,
        expected: StoreRevision,
        edit: impl FnOnce(&mut State) -> Result<(), AuthorityError>,
    ) -> Result<StoreRevision, AuthorityError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        if inner.state.revision != expected {
            return Err(AuthorityError::StaleRevision {
                expected,
                current: inner.state.revision,
            });
        }
        next_revision(expected)?;
        let mut candidate = inner.state.clone();
        edit(&mut candidate)?;
        self.publish(&mut inner, candidate)
    }

    fn mutate_current(
        &self,
        edit: impl FnOnce(&mut State) -> Result<(), AuthorityError>,
    ) -> Result<StoreRevision, AuthorityError> {
        let mut inner = self.lock()?;
        inner.ensure_ready()?;
        let mut candidate = inner.state.clone();
        edit(&mut candidate)?;
        self.publish(&mut inner, candidate)
    }

    fn publish(
        &self,
        inner: &mut Inner,
        candidate: State,
    ) -> Result<StoreRevision, AuthorityError> {
        self.publish_checked(inner, candidate, || Ok(()))
    }

    fn publish_checked<E: From<AuthorityError>>(
        &self,
        inner: &mut Inner,
        mut candidate: State,
        before_replace: impl FnOnce() -> Result<(), E>,
    ) -> Result<StoreRevision, E> {
        inner.ensure_ready()?;
        let revision = next_revision(inner.state.revision)?;
        candidate.revision = revision;
        candidate.synchronize_operations()?;
        candidate.validate()?;
        let encoded = inner.storage.encode(&candidate)?;
        before_replace()?;
        if let Err(error) = inner
            .storage
            .publish(encoded.as_ref().map(|bytes| bytes.as_slice()))
        {
            inner.status = AuthorityStatus::Faulted;
            inner.invitations.clear();
            self.changes.send_replace(());
            return Err(error.into());
        }
        inner.state = candidate;
        self.changes.send_replace(());
        Ok(revision)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, AuthorityError> {
        self.inner.lock().map_err(|_| AuthorityError::Faulted)
    }
}

impl TrustPolicy for PeerAuthority {
    fn is_trusted(&self, pin: &PeerPin) -> bool {
        let Ok(inner) = self.lock() else {
            return false;
        };
        inner.status == AuthorityStatus::Ready
            && !inner.state.is_revoked(pin.peer_id())
            && inner
                .state
                .peer(pin.peer_id())
                .is_some_and(|peer| peer.pin == *pin)
    }
}

impl Inner {
    fn ensure_ready(&self) -> Result<(), AuthorityError> {
        match self.status {
            AuthorityStatus::Ready => Ok(()),
            AuthorityStatus::Faulted => Err(AuthorityError::Faulted),
        }
    }
}

fn next_revision(revision: StoreRevision) -> Result<StoreRevision, AuthorityError> {
    revision
        .value()
        .checked_add(1)
        .map(StoreRevision::new)
        .ok_or(AuthorityError::RevisionExhausted)
}
