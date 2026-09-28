use crate::{
    service::{NormalClientConfig, PeerPin},
    PeerId,
};

use super::{AuthorityError, Inner, PeerAuthority};

impl PeerAuthority {
    pub fn normal_client_config(&self, peer: PeerId) -> Result<NormalClientConfig, AuthorityError> {
        let (identity, pin) = {
            let inner = self.lock()?;
            let pin = linked_pin(&inner, peer)?.clone();
            (inner.state.identity.clone(), pin)
        };
        NormalClientConfig::new(&identity, pin).map_err(|_| AuthorityError::Transport)
    }

    pub fn watch_changes(&self) -> tokio::sync::watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub(in crate::service) fn normal_session_identity(
        &self,
        remote: &PeerPin,
    ) -> Result<PeerId, AuthorityError> {
        let inner = self.lock()?;
        if linked_pin(&inner, remote.peer_id())? != remote {
            return Err(AuthorityError::UnknownPeer);
        }
        Ok(inner.state.identity.pin().peer_id())
    }

    #[cfg(test)]
    pub(in crate::service) fn fault_for_session_test(&self) {
        self.inner.lock().unwrap().status = crate::AuthorityStatus::Faulted;
        self.changes.send_replace(());
    }
}

fn linked_pin(inner: &Inner, peer: PeerId) -> Result<&PeerPin, AuthorityError> {
    inner.ensure_ready()?;
    if peer == inner.state.identity.pin().peer_id() {
        return Err(AuthorityError::LocalPeer);
    }
    inner
        .state
        .peer(peer)
        .map(|linked| &linked.pin)
        .ok_or(AuthorityError::UnknownPeer)
}
