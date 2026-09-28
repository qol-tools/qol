use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddrV4,
    time::Duration,
};

use tokio::time::Instant;

use crate::{session::AuthenticatedSession, PeerId};

pub(super) const MAX_HANDSHAKES: usize = 16;
pub(super) const MAX_PEERS: usize = 256;
const MAX_HINTS: usize = 8;

#[derive(Default)]
pub(super) struct Routes {
    pub peers: BTreeMap<PeerId, Route>,
    seen: BTreeMap<String, (PeerId, Vec<SocketAddrV4>)>,
}

pub(super) struct Route {
    hints: Vec<(String, SocketAddrV4)>,
    failures: u32,
    pub next: Instant,
    pub dialing: bool,
}

impl Routes {
    pub fn retain(&mut self, trusted: &BTreeSet<PeerId>) {
        self.peers.retain(|peer, _| trusted.contains(peer));
        for (source, (peer, endpoints)) in &self.seen {
            if trusted.contains(peer) {
                hint(&mut self.peers, source, *peer, endpoints);
            }
        }
    }

    pub fn resolved(
        &mut self,
        source: String,
        peer: PeerId,
        endpoints: Vec<SocketAddrV4>,
        trusted: &BTreeSet<PeerId>,
        loopback: bool,
    ) {
        self.removed(&source);
        if source.len() > 255 {
            return;
        }
        let endpoints: Vec<_> = endpoints
            .into_iter()
            .filter(|address| valid(*address, loopback))
            .take(MAX_HINTS)
            .collect();
        if endpoints.is_empty() {
            return;
        }
        if trusted.contains(&peer) {
            hint(&mut self.peers, &source, peer, &endpoints);
        }
        if self.seen.len() < MAX_PEERS {
            self.seen.insert(source, (peer, endpoints));
        }
    }

    pub fn removed(&mut self, source: &str) {
        self.seen.remove(source);
        for route in self.peers.values_mut() {
            route.hints.retain(|hint| hint.0 != source);
        }
    }

    pub fn clear_hints(&mut self) {
        self.seen.clear();
        for route in self.peers.values_mut() {
            route.hints.clear();
        }
    }
}

fn hint(
    peers: &mut BTreeMap<PeerId, Route>,
    source: &str,
    peer: PeerId,
    endpoints: &[SocketAddrV4],
) {
    if !peers.contains_key(&peer) && peers.len() >= MAX_PEERS {
        return;
    }
    let route = peers.entry(peer).or_insert_with(|| Route {
        hints: Vec::new(),
        failures: 0,
        next: Instant::now(),
        dialing: false,
    });
    for endpoint in endpoints {
        if route.hints.len() >= MAX_HINTS {
            break;
        }
        if !route
            .hints
            .iter()
            .any(|hint| hint.1 == *endpoint && hint.0 == source)
        {
            route.hints.push((source.to_owned(), *endpoint));
        }
    }
}

impl Route {
    pub fn endpoint(&self) -> Option<SocketAddrV4> {
        if self.dialing {
            return None;
        }
        let index = self.failures as usize % self.hints.len().max(1);
        self.hints.get(index).map(|hint| hint.1)
    }

    pub fn failed(&mut self) {
        self.dialing = false;
        self.failures = self.failures.saturating_add(1);
        self.next = Instant::now() + Duration::from_secs((1u64 << self.failures.min(5)).min(30));
    }
}

pub(super) fn valid(address: SocketAddrV4, loopback: bool) -> bool {
    let ip = address.ip();
    address.port() != 0
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && (loopback || !ip.is_loopback())
}

pub(super) fn rank(session: AuthenticatedSession, outgoing: bool) -> (bool, String, String) {
    let local_first = session.local_peer < session.remote_peer;
    let preferred = outgoing == local_first;
    let generation = session.generation;
    let (first, second) = if local_first {
        (generation.local, generation.remote)
    } else {
        (generation.remote, generation.local)
    };
    (!preferred, first.to_string(), second.to_string())
}

#[cfg(test)]
#[path = "tests/routing.rs"]
mod tests;
