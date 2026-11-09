use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use tokio::{net::TcpListener, sync::watch, time::Instant};

use super::refusal;
use crate::admin::{EnrollmentFailure, Error};

pub(super) const INVITATION_RESERVATION: Duration = Duration::from_secs(30);
pub(in crate::service::network) const NEARBY_LEASE: Duration = Duration::from_secs(10);

#[derive(Default)]
struct State {
    reserved_until: Option<Instant>,
    nearby_until: Option<Instant>,
    port: Option<u16>,
    unadopted: Option<std::net::TcpListener>,
    closed: bool,
}

impl State {
    fn nearby_open(&self, now: Instant) -> bool {
        self.nearby_until.is_some_and(|deadline| deadline > now)
    }

    fn held(&self, now: Instant) -> bool {
        [self.reserved_until, self.nearby_until]
            .into_iter()
            .flatten()
            .any(|deadline| deadline > now)
    }

    fn bound_port(&self) -> Option<u16> {
        self.port.or_else(|| {
            self.unadopted
                .as_ref()
                .and_then(|socket| socket.local_addr().ok())
                .map(|address| address.port())
        })
    }

    fn nearby_link(&self, now: Instant) -> Option<u16> {
        self.nearby_open(now).then(|| self.bound_port()).flatten()
    }
}

pub(in crate::service::network) struct Gate {
    bind: Ipv4Addr,
    state: Mutex<State>,
    wake: watch::Sender<()>,
    nearby_link: watch::Sender<Option<u16>>,
}

impl Gate {
    pub(super) fn new(bind: Ipv4Addr) -> Self {
        Self {
            bind,
            state: Mutex::new(State::default()),
            wake: watch::channel(()).0,
            nearby_link: watch::channel(None).0,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn publish(&self, state: &State) {
        self.nearby_link
            .send_replace(state.nearby_link(Instant::now()));
    }

    pub(in crate::service::network) fn subscribe(&self) -> watch::Receiver<()> {
        self.wake.subscribe()
    }

    pub(in crate::service::network) fn nearby_link_watch(&self) -> watch::Receiver<Option<u16>> {
        self.nearby_link.subscribe()
    }

    pub(in crate::service::network) fn nearby_deadline(&self) -> Option<Instant> {
        self.lock()
            .nearby_until
            .filter(|deadline| *deadline > Instant::now())
    }

    pub(in crate::service::network) fn nearby_open(&self) -> bool {
        self.nearby_deadline().is_some()
    }

    pub(super) fn held(&self) -> bool {
        self.lock().held(Instant::now())
    }

    pub(super) fn refresh(&self) {
        let state = self.lock();
        self.publish(&state);
    }

    pub(super) fn hold_for_invitation(&self) -> Result<u16, Error> {
        self.hold(INVITATION_RESERVATION, false)
    }

    pub(in crate::service::network) fn hold_for_nearby(&self) -> Result<u16, Error> {
        self.hold(NEARBY_LEASE, true)
    }

    fn hold(&self, lifetime: Duration, nearby: bool) -> Result<u16, Error> {
        let mut state = self.lock();
        if state.closed {
            return Err(refusal(EnrollmentFailure::Unavailable));
        }
        let deadline = Instant::now() + lifetime;
        let slot = if nearby {
            &mut state.nearby_until
        } else {
            &mut state.reserved_until
        };
        *slot = Some(slot.map_or(deadline, |current| current.max(deadline)));
        let port = match state.bound_port() {
            Some(port) => port,
            None => {
                let socket = std::net::TcpListener::bind(SocketAddrV4::new(self.bind, 0))
                    .map_err(|_| refusal(EnrollmentFailure::Unavailable))?;
                socket
                    .set_nonblocking(true)
                    .map_err(|_| refusal(EnrollmentFailure::Unavailable))?;
                let port = socket
                    .local_addr()
                    .map_err(|_| refusal(EnrollmentFailure::Unavailable))?
                    .port();
                state.unadopted = Some(socket);
                port
            }
        };
        self.publish(&state);
        drop(state);
        self.wake.send_replace(());
        Ok(port)
    }

    pub(super) fn open(&self) -> std::io::Result<TcpListener> {
        let mut state = self.lock();
        let socket = match state.unadopted.take() {
            Some(socket) => socket,
            None => {
                let socket = std::net::TcpListener::bind(SocketAddrV4::new(self.bind, 0))?;
                socket.set_nonblocking(true)?;
                socket
            }
        };
        let listener = TcpListener::from_std(socket)?;
        state.port = Some(listener.local_addr()?.port());
        self.publish(&state);
        Ok(listener)
    }

    pub(super) fn release(&self) -> bool {
        let mut state = self.lock();
        if state.held(Instant::now()) {
            return false;
        }
        state.port = None;
        state.unadopted = None;
        self.publish(&state);
        true
    }

    pub(super) fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.port = None;
        state.unadopted = None;
        self.publish(&state);
        drop(state);
        self.wake.send_replace(());
    }
}
