use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddrV4,
    time::Duration,
};

use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
    task::JoinSet,
    time::{timeout, timeout_at, Instant},
};

use super::{
    discovery::{cancelled, Advertisement, DiscoveryEvent, CLOSE_TIMEOUT},
    routing::{rank, Routes, MAX_HANDSHAKES, MAX_PEERS},
    NetworkOptions, NetworkSnapshot,
};
use crate::{
    admin::NetworkRevision,
    network::{DiscoveryStatus, ListenerStatus, NetworkFailure},
    service::{
        session::{NormalSession, SessionError},
        PeerAuthority,
    },
    session::AuthenticatedSession,
    AuthorityStatus, PeerId, StoreRevision,
};

type Handshake = (
    Option<PeerId>,
    bool,
    Result<NormalSession<TcpStream>, SessionError>,
);

struct Live {
    authenticated: AuthenticatedSession,
    outgoing: bool,
    stop: watch::Sender<bool>,
}

struct Supervisor {
    authority: PeerAuthority,
    trusted: BTreeSet<PeerId>,
    routes: Routes,
    live: BTreeMap<PeerId, Live>,
    handshakes: JoinSet<Handshake>,
    sessions: JoinSet<AuthenticatedSession>,
    view: watch::Sender<NetworkSnapshot>,
    loopback: bool,
    operations: std::sync::Arc<super::operations::Hub>,
    workers: mpsc::Receiver<super::operations::Worker>,
    dispatches: JoinSet<super::operations::Completion>,
}

pub(super) async fn run(
    authority: PeerAuthority,
    options: NetworkOptions,
    mut stop: watch::Receiver<bool>,
    view: watch::Sender<NetworkSnapshot>,
    operations: std::sync::Arc<super::operations::Hub>,
    workers: mpsc::Receiver<super::operations::Worker>,
) -> Result<(), NetworkFailure> {
    let mut owner = Supervisor {
        authority,
        trusted: BTreeSet::new(),
        routes: Routes::default(),
        live: BTreeMap::new(),
        handshakes: JoinSet::new(),
        sessions: JoinSet::new(),
        view,
        loopback: options.allow_loopback_hints,
        operations,
        workers,
        dispatches: JoinSet::new(),
    };
    let result = owner.serve(options, &mut stop).await;
    owner.view.send_modify(|view| {
        view.status.stopping = true;
        view.status.listener = ListenerStatus::Closed {};
    });
    owner.operations.stop();
    owner.handshakes.abort_all();
    while owner.handshakes.join_next().await.is_some() {}
    for live in owner.live.values() {
        owner.operations.close(live.authenticated);
        live.stop.send_replace(true);
    }
    while owner.sessions.join_next().await.is_some() {}
    owner.workers.close();
    while let Some(worker) = owner.workers.recv().await {
        let hub = owner.operations.clone();
        owner.dispatches.spawn_blocking(move || hub.execute(worker));
    }
    let mut dispatch_failed = false;
    while let Some(result) = owner.dispatches.join_next().await {
        match result {
            Ok(completion) => {
                dispatch_failed |= owner.operations.completed(completion).is_err();
            }
            Err(_) => dispatch_failed = true,
        }
    }
    if dispatch_failed {
        return Err(NetworkFailure::Cleanup);
    }
    owner.live.clear();
    let projection = owner.publish_sessions();
    owner.view.send_modify(|view| {
        view.status.stopping = false;
        view.status.failure = result.as_ref().err().copied().or(projection.err());
        if view.status.failure == Some(NetworkFailure::Listener) {
            view.status.listener = ListenerStatus::Failed {
                error: NetworkFailure::Listener,
            };
        }
        if matches!(view.status.discovery, DiscoveryStatus::Starting {}) {
            view.status.discovery = DiscoveryStatus::Closed {};
        }
    });
    result
}

impl Supervisor {
    async fn serve(
        &mut self,
        options: NetworkOptions,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(), NetworkFailure> {
        let mut changes = self.authority.watch_changes();
        self.refresh_authority()?;
        if *stop.borrow() {
            return Ok(());
        }
        let listener = TcpListener::bind(SocketAddrV4::new(options.bind, 0))
            .await
            .map_err(|_| NetworkFailure::Listener)?;
        let port = listener
            .local_addr()
            .map_err(|_| NetworkFailure::Listener)?
            .port();
        self.view
            .send_modify(|view| view.status.listener = ListenerStatus::Listening { port });
        let peer = self
            .authority
            .local_pin()
            .map_err(|_| NetworkFailure::Authority)?
            .peer_id();
        let (events, mut discovery_events) = mpsc::channel(64);
        let (discovery_stop, stopping) = watch::channel(false);
        let mut discovery = options.discovery.run(
            Advertisement {
                peer,
                port,
                bind: options.bind,
            },
            events,
            stopping,
        );
        let mut discovery_done = None;
        let mut discovery_events_open = true;
        let mut discovery_deadline = None;
        let result = loop {
            self.dial_ready();
            let retry = self.next_retry();
            tokio::select! {
                biased;
                () = cancelled(stop) => break Ok(()),
                change = changes.changed() => {
                    if change.is_err() { break Err(NetworkFailure::Authority); }
                    if let Err(error) = self.refresh_authority() { break Err(error); }
                },
                completion = self.dispatches.join_next(), if !self.dispatches.is_empty() => {
                    match completion {
                        Some(Ok(completion)) => if let Err(error) = self.operations.completed(completion) { break Err(error); },
                        Some(Err(_)) => break Err(NetworkFailure::Cleanup),
                        None => {},
                    }
                },
                worker = self.workers.recv() => {
                    if let Some(worker) = worker {
                        let hub = self.operations.clone();
                        self.dispatches.spawn_blocking(move || hub.execute(worker));
                    }
                },
                result = self.sessions.join_next(), if !self.sessions.is_empty() => {
                    match result {
                        Some(Ok(session)) => if let Err(error) = self.exited(session) { break Err(error); },
                        Some(Err(_)) => break Err(NetworkFailure::Task),
                        None => {},
                    }
                },
                result = self.handshakes.join_next(), if !self.handshakes.is_empty() => {
                    match result {
                        Some(Ok(result)) => if let Err(error) = self.handshake_finished(result) { break Err(error); },
                        Some(Err(_)) => break Err(NetworkFailure::Task),
                        None => {},
                    }
                },
                result = &mut discovery, if discovery_done.is_none() => {
                    self.routes.clear_hints();
                    self.view.send_modify(|view| {
                        view.status.discovery = DiscoveryStatus::Failed { error: result.err().unwrap_or(NetworkFailure::Discovery) };
                        view.status.failure = result.err();
                    });
                    discovery_done = Some(result);
                },
                () = retry_at(discovery_deadline), if discovery_done.is_none() => {
                    discovery_done = Some(Err(NetworkFailure::Cleanup));
                    self.view.send_modify(|view| {
                        view.status.discovery = DiscoveryStatus::Failed { error: NetworkFailure::Cleanup };
                        view.status.failure = Some(NetworkFailure::Cleanup);
                    });
                },
                event = discovery_events.recv(), if discovery_events_open && discovery_done.is_none() => {
                    match event {
                        Some(event) => self.discovery_event(event),
                        None => {
                            discovery_events_open = false;
                            self.discovery_event(DiscoveryEvent::Failed);
                            discovery_stop.send_replace(true);
                            discovery_deadline = Some(Instant::now() + CLOSE_TIMEOUT);
                        },
                    }
                },
                accepted = listener.accept(), if self.can_accept() => {
                    match accepted {
                        Ok((stream, _)) => self.accept(stream),
                        Err(_) => break Err(NetworkFailure::Listener),
                    }
                },
                () = retry_at(retry) => {},
            }
        };
        drop(listener);
        self.view.send_modify(|view| {
            view.status.stopping = true;
            view.status.listener = ListenerStatus::Closed {};
        });
        self.handshakes.abort_all();
        for live in self.live.values() {
            self.operations.close(live.authenticated);
            live.stop.send_replace(true);
        }
        self.live.clear();
        let cleared = self.publish_sessions();
        discovery_stop.send_replace(true);
        let closed = match discovery_done {
            Some(result) => result,
            None => timeout_at(
                discovery_deadline.unwrap_or_else(|| Instant::now() + CLOSE_TIMEOUT),
                &mut discovery,
            )
            .await
            .unwrap_or(Err(NetworkFailure::Cleanup)),
        };
        self.view.send_modify(|view| {
            view.status.discovery = match closed {
                Ok(()) => DiscoveryStatus::Closed {},
                Err(error) => DiscoveryStatus::Failed { error },
            }
        });
        closed.and(result).and(cleared)
    }

    fn refresh_authority(&mut self) -> Result<(), NetworkFailure> {
        let projection = self
            .authority
            .projection()
            .map_err(|_| NetworkFailure::Authority)?;
        if projection.status != AuthorityStatus::Ready {
            return Err(NetworkFailure::Authority);
        }
        self.trusted = projection
            .peers
            .into_iter()
            .map(|peer| peer.peer_id)
            .collect();
        self.routes.retain(&self.trusted);
        self.live.retain(|peer, live| {
            if self.trusted.contains(peer) {
                return true;
            }
            self.operations.close(live.authenticated);
            live.stop.send_replace(true);
            false
        });
        self.publish_sessions()
    }

    fn discovery_event(&mut self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::Ready => self
                .view
                .send_modify(|view| view.status.discovery = DiscoveryStatus::Ready {}),
            DiscoveryEvent::Failed => {
                self.routes.clear_hints();
                self.view.send_modify(|view| {
                    view.status.discovery = DiscoveryStatus::Failed {
                        error: NetworkFailure::Discovery,
                    }
                });
            }
            DiscoveryEvent::Resolved {
                source,
                peer,
                endpoints,
            } => self
                .routes
                .resolved(source, peer, endpoints, &self.trusted, self.loopback),
            DiscoveryEvent::Removed { source } => self.routes.removed(&source),
        }
    }

    fn can_accept(&self) -> bool {
        self.handshakes.len() < MAX_HANDSHAKES && self.sessions.len() < MAX_PEERS + MAX_HANDSHAKES
    }

    fn next_retry(&self) -> Option<Instant> {
        if !self.can_accept() {
            return None;
        }
        self.routes
            .peers
            .iter()
            .filter(|(peer, route)| !self.live.contains_key(peer) && route.endpoint().is_some())
            .map(|(_, route)| route.next)
            .min()
    }

    fn dial_ready(&mut self) {
        for (peer, route) in &mut self.routes.peers {
            if self.handshakes.len() >= MAX_HANDSHAKES
                || self.sessions.len() >= MAX_PEERS + MAX_HANDSHAKES
            {
                break;
            }
            if self.live.contains_key(peer) || route.next > Instant::now() {
                continue;
            }
            let Some(endpoint) = route.endpoint() else {
                continue;
            };
            route.dialing = true;
            let authority = self.authority.clone();
            let peer = *peer;
            self.handshakes.spawn(async move {
                let work = async {
                    let io = timeout(Duration::from_secs(5), TcpStream::connect(endpoint))
                        .await
                        .map_err(|_| SessionError::Transport)?
                        .map_err(|_| SessionError::Transport)?;
                    NormalSession::connect(io, authority.clone(), peer).await
                };
                let result = tokio::select! {
                    biased;
                    () = invalidated(&authority, Some(peer)) => Err(SessionError::Untrusted),
                    result = work => result,
                };
                (Some(peer), true, result)
            });
        }
    }

    fn accept(&mut self, stream: TcpStream) {
        let authority = self.authority.clone();
        self.handshakes.spawn(async move {
            let result = tokio::select! {
                biased;
                () = invalidated(&authority, None) => Err(SessionError::Untrusted),
                result = NormalSession::accept(stream, authority.clone()) => result,
            };
            (None, false, result)
        });
    }

    fn handshake_finished(
        &mut self,
        (peer, outgoing, result): Handshake,
    ) -> Result<(), NetworkFailure> {
        if let Some(route) = peer.and_then(|peer| self.routes.peers.get_mut(&peer)) {
            route.failed();
        }
        let Ok(session) = result else {
            return Ok(());
        };
        let Ok(authenticated) = session.authenticated() else {
            return Ok(());
        };
        let peer = authenticated.remote_peer;
        if !self.trusted.contains(&peer) || self.sessions.len() >= MAX_PEERS + MAX_HANDSHAKES {
            return Ok(());
        }
        if let Some(current) = self.live.get(&peer) {
            if rank(current.authenticated, current.outgoing) <= rank(authenticated, outgoing) {
                return Ok(());
            }
            current.stop.send_replace(true);
        }
        let (stop, mut stopping) = watch::channel(false);
        let io = self
            .operations
            .elect(authenticated)
            .map_err(|_| NetworkFailure::Authority)?;
        let hub = self.operations.clone();
        self.sessions.spawn(async move {
            session
                .run_operations(cancelled(&mut stopping), hub.clone(), io)
                .await;
            hub.close(authenticated);
            authenticated
        });
        self.live.insert(
            peer,
            Live {
                authenticated,
                outgoing,
                stop,
            },
        );
        self.publish_sessions()
    }

    fn exited(&mut self, session: AuthenticatedSession) -> Result<(), NetworkFailure> {
        let peer = session.remote_peer;
        if !self
            .live
            .get(&peer)
            .is_some_and(|current| current.authenticated == session)
        {
            return Ok(());
        }
        self.live.remove(&peer);
        if let Some(route) = self.routes.peers.get_mut(&peer) {
            route.failed();
        }
        self.publish_sessions()
    }

    fn publish_sessions(&self) -> Result<(), NetworkFailure> {
        let sessions: Vec<_> = self.live.values().map(|live| live.authenticated).collect();
        let old = self.view.borrow();
        if old.sessions == sessions {
            return Ok(());
        }
        let revision = old
            .revision
            .0
            .value()
            .checked_add(1)
            .ok_or(NetworkFailure::RevisionExhausted)?;
        drop(old);
        self.view.send_modify(|view| {
            view.revision = NetworkRevision(StoreRevision::new(revision));
            view.sessions = sessions;
        });
        Ok(())
    }
}

async fn retry_at(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn invalidated(authority: &PeerAuthority, peer: Option<PeerId>) {
    let mut changes = authority.watch_changes();
    loop {
        let ready = authority.projection().is_ok_and(|projection| {
            projection.status == AuthorityStatus::Ready
                && peer
                    .is_none_or(|peer| projection.peers.iter().any(|linked| linked.peer_id == peer))
        });
        if !ready || changes.changed().await.is_err() {
            return;
        }
        if peer.is_none() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "tests/supervisor.rs"]
mod tests;
