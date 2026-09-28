use std::{
    collections::{BTreeMap, BTreeSet},
    net::{SocketAddr, SocketAddrV4},
};

use super::{discovery::NearbyAdvertisement, routing::valid};
use crate::{admin::MAX_NEARBY, PeerId};

const MAX_ENDPOINTS: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NearbyClaim {
    pub peer_id: PeerId,
    pub name: String,
    pub endpoints: Vec<SocketAddr>,
}

#[derive(Default)]
pub(super) struct Nearby {
    sources: BTreeMap<String, NearbyClaim>,
}

impl Nearby {
    pub fn resolved(
        &mut self,
        source: String,
        peer: PeerId,
        endpoints: &[SocketAddrV4],
        claim: Option<NearbyAdvertisement>,
        ignored: &BTreeSet<PeerId>,
        loopback: bool,
    ) -> bool {
        let removed = self.sources.remove(&source).is_some();
        let Some(claim) = claim else {
            return removed;
        };
        if ignored.contains(&peer)
            || source.len() > 255
            || (!self.sources.values().any(|entry| entry.peer_id == peer)
                && self.peers().len() >= MAX_NEARBY)
        {
            return removed;
        }
        let endpoints: Vec<_> = endpoints
            .iter()
            .map(|address| SocketAddrV4::new(*address.ip(), claim.link))
            .filter(|address| valid(*address, loopback))
            .take(MAX_ENDPOINTS)
            .map(SocketAddr::V4)
            .collect();
        if endpoints.is_empty() {
            return removed;
        }
        self.sources.insert(
            source,
            NearbyClaim {
                peer_id: peer,
                name: claim.name,
                endpoints,
            },
        );
        true
    }

    pub fn removed(&mut self, source: &str) -> bool {
        self.sources.remove(source).is_some()
    }

    pub fn clear(&mut self) -> bool {
        let changed = !self.sources.is_empty();
        self.sources.clear();
        changed
    }

    pub fn retain(&mut self, ignored: &BTreeSet<PeerId>) -> bool {
        let before = self.sources.len();
        self.sources
            .retain(|_, claim| !ignored.contains(&claim.peer_id));
        before != self.sources.len()
    }

    pub fn claims(&self) -> Vec<NearbyClaim> {
        let mut claims: Vec<NearbyClaim> = Vec::new();
        for claim in self.sources.values() {
            match claims
                .iter_mut()
                .find(|entry| entry.peer_id == claim.peer_id)
            {
                Some(entry) => {
                    for endpoint in &claim.endpoints {
                        if entry.endpoints.len() < MAX_ENDPOINTS
                            && !entry.endpoints.contains(endpoint)
                        {
                            entry.endpoints.push(*endpoint);
                        }
                    }
                }
                None => claims.push(claim.clone()),
            }
        }
        claims
    }

    fn peers(&self) -> BTreeSet<PeerId> {
        self.sources.values().map(|claim| claim.peer_id).collect()
    }
}

#[cfg(test)]
#[path = "tests/nearby.rs"]
mod tests;
