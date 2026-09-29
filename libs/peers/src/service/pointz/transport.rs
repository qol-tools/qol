use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{net::UdpSocket, runtime::Handle, sync::watch, task::JoinHandle};

use super::{
    envelope,
    pairing::{self, HandleOutcome, PairingSession},
    replay::ReplayWindow,
};
use crate::pointz::{PointzDeviceId, PointzPairing, PointzSocket, PointzTransport};
use crate::service::{AuthorityError, PeerAuthority, WeakPeerAuthority};

pub const DISCOVERY_PORT: u16 = 45454;
pub const COMMAND_PORT: u16 = 45455;
const DISCOVER_MESSAGE: &str = "DISCOVER";
const AUTHENTICATION: &str = "pair-x25519-v1";
const DATAGRAM_BYTES: usize = 4096;
const FORWARD_QUEUE: usize = 256;
const COMMAND_CLOCK_SKEW_MS: u64 = 30_000;

pub trait PointzSink: Send + Sync + 'static {
    fn deliver(&self, command: serde_json::Value) -> bool;
}

#[derive(Clone)]
pub struct PointzOptions {
    pub bind: Ipv4Addr,
    pub discovery_port: u16,
    pub command_port: u16,
    pub hostname: String,
}

impl PointzOptions {
    pub fn new(hostname: String) -> Self {
        Self {
            bind: Ipv4Addr::UNSPECIFIED,
            discovery_port: DISCOVERY_PORT,
            command_port: COMMAND_PORT,
            hostname,
        }
    }
}

pub struct PointzAdapter {
    pairing: Arc<Mutex<Option<PairingSession>>>,
    dropped: Arc<AtomicU64>,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    ports: (u16, u16),
}

#[derive(Serialize)]
struct DiscoveryResponse<'a> {
    hostname: &'a str,
    server_id: &'a str,
    authentication: &'static str,
    pairing_open: bool,
}

impl PointzAdapter {
    pub fn start(
        runtime: &Handle,
        authority: &PeerAuthority,
        options: PointzOptions,
        sink: Arc<dyn PointzSink>,
    ) -> Result<Self, PointzTransport> {
        let _entered = runtime.enter();
        let discovery = bind(
            options.bind,
            options.discovery_port,
            PointzSocket::Discovery,
        )?;
        let command = bind(options.bind, options.command_port, PointzSocket::Command)?;
        let ports = (port(&discovery), port(&command));
        let pairing = Arc::new(Mutex::new(None));
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = watch::channel(false).0;
        let (queue, received) = mpsc::sync_channel::<serde_json::Value>(FORWARD_QUEUE);
        let forwarded = dropped.clone();
        std::thread::Builder::new()
            .name("pointz-forward".into())
            .spawn(move || {
                for command in received {
                    if !sink.deliver(command) {
                        forwarded.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
            .map_err(|_| PointzTransport::Failed {
                socket: PointzSocket::Command,
            })?;
        let tasks = vec![
            runtime.spawn(run_discovery(
                discovery,
                authority.downgrade(),
                options.hostname,
                pairing.clone(),
                stop.subscribe(),
            )),
            runtime.spawn(run_commands(
                command,
                authority.downgrade(),
                queue,
                dropped.clone(),
                stop.subscribe(),
            )),
        ];
        Ok(Self {
            pairing,
            dropped,
            stop,
            tasks,
            ports,
        })
    }

    pub fn begin_pairing(&self) -> Result<PointzPairing, AuthorityError> {
        let session = PairingSession::open().ok_or(AuthorityError::Identity)?;
        let mut pairing = self.pairing.lock().map_err(|_| AuthorityError::Faulted)?;
        *pairing = Some(session);
        Ok(snapshot(&mut pairing))
    }

    pub fn cancel_pairing(&self) -> PointzPairing {
        let Ok(mut pairing) = self.pairing.lock() else {
            return PointzPairing::CLOSED;
        };
        *pairing = None;
        PointzPairing::CLOSED
    }

    pub fn pairing(&self) -> PointzPairing {
        self.pairing
            .lock()
            .map(|mut pairing| snapshot(&mut pairing))
            .unwrap_or(PointzPairing::CLOSED)
    }

    pub fn transport(&self) -> PointzTransport {
        PointzTransport::Running {
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }

    pub fn ports(&self) -> (u16, u16) {
        self.ports
    }
}

impl Drop for PointzAdapter {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Ok(mut pairing) = self.pairing.lock() {
            *pairing = None;
        }
        for task in &self.tasks {
            task.abort();
        }
    }
}

fn bind(address: Ipv4Addr, port: u16, socket: PointzSocket) -> Result<UdpSocket, PointzTransport> {
    let failed = |_| PointzTransport::Failed { socket };
    let bound = std::net::UdpSocket::bind((address, port)).map_err(|error| match error.kind() {
        io::ErrorKind::AddrInUse => PointzTransport::PortBusy { socket },
        _ => PointzTransport::Failed { socket },
    })?;
    bound.set_broadcast(true).map_err(failed)?;
    bound.set_nonblocking(true).map_err(failed)?;
    UdpSocket::from_std(bound).map_err(failed)
}

fn port(socket: &UdpSocket) -> u16 {
    socket
        .local_addr()
        .map(|address| address.port())
        .unwrap_or(0)
}

fn snapshot(pairing: &mut Option<PairingSession>) -> PointzPairing {
    if pairing.as_ref().is_some_and(PairingSession::is_expired) {
        *pairing = None;
    }
    match pairing {
        Some(session) => PointzPairing {
            open: true,
            code: Some(session.pin().to_string()),
            seconds_remaining: session.seconds_remaining(),
        },
        None => PointzPairing::CLOSED,
    }
}

async fn run_discovery(
    socket: UdpSocket,
    authority: WeakPeerAuthority,
    hostname: String,
    pairing: Arc<Mutex<Option<PairingSession>>>,
    mut stop: watch::Receiver<bool>,
) {
    let mut buffer = vec![0; DATAGRAM_BYTES];
    loop {
        let (size, from) = tokio::select! {
            _ = stop.wait_for(|stopped| *stopped) => return,
            received = socket.recv_from(&mut buffer) => match received {
                Ok(received) => received,
                Err(_) => continue,
            },
        };
        let Some(authority) = authority.upgrade() else {
            return;
        };
        let Ok(seed) = authority.pointz_seed() else {
            continue;
        };
        let server_id = seed.server_id();
        let datagram = &buffer[..size];
        let reply = if String::from_utf8_lossy(datagram).trim() == DISCOVER_MESSAGE {
            discovery_reply(&hostname, &server_id, &pairing)
        } else {
            pairing_reply(&authority, &server_id, datagram, &pairing)
        };
        drop(authority);
        if let Some(reply) = reply {
            send(&socket, &reply, from).await;
        }
    }
}

fn discovery_reply(
    hostname: &str,
    server_id: &str,
    pairing: &Mutex<Option<PairingSession>>,
) -> Option<Vec<u8>> {
    let pairing_open = pairing
        .lock()
        .map(|mut pairing| snapshot(&mut pairing).open)
        .unwrap_or(false);
    serde_json::to_vec(&DiscoveryResponse {
        hostname,
        server_id,
        authentication: AUTHENTICATION,
        pairing_open,
    })
    .ok()
}

fn pairing_reply(
    authority: &PeerAuthority,
    server_id: &str,
    datagram: &[u8],
    pairing: &Mutex<Option<PairingSession>>,
) -> Option<Vec<u8>> {
    let mut guard = pairing.lock().ok()?;
    let session = guard.as_mut()?;
    match session.handle(server_id, datagram) {
        HandleOutcome::Ignore => None,
        HandleOutcome::Reply(reply) => Some(reply),
        HandleOutcome::Closed(reply) => {
            *guard = None;
            Some(reply)
        }
        HandleOutcome::Paired {
            device_id,
            device_key,
            name,
            reply,
        } => {
            *guard = None;
            let device = PointzDeviceId::from_bytes(device_id);
            match authority.pair_pointz(device, zeroize::Zeroizing::new(device_key), &name) {
                Ok(_) => Some(reply),
                Err(_) => Some(pairing::unavailable()),
            }
        }
    }
}

async fn run_commands(
    socket: UdpSocket,
    authority: WeakPeerAuthority,
    queue: SyncSender<serde_json::Value>,
    dropped: Arc<AtomicU64>,
    mut stop: watch::Receiver<bool>,
) {
    let mut buffer = vec![0; DATAGRAM_BYTES];
    let mut replay = ReplayWindow::default();
    loop {
        let size = tokio::select! {
            _ = stop.wait_for(|stopped| *stopped) => return,
            received = socket.recv_from(&mut buffer) => match received {
                Ok((size, _)) => size,
                Err(_) => continue,
            },
        };
        let Some(authority) = authority.upgrade() else {
            return;
        };
        let command = authenticate(&authority, &buffer[..size], &mut replay);
        drop(authority);
        let Some(command) = command else {
            continue;
        };
        match queue.try_send(command) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

fn authenticate(
    authority: &PeerAuthority,
    packet: &[u8],
    replay: &mut ReplayWindow,
) -> Option<serde_json::Value> {
    let parsed = envelope::parse(packet)?;
    if parsed.sent_at_ms.abs_diff(unix_time_ms()) > COMMAND_CLOCK_SKEW_MS {
        return None;
    }
    let key = authority.pointz_key(&PointzDeviceId::from_bytes(parsed.device_id))?;
    let payload = parsed.verify(&key)?;
    let command: serde_json::Value = serde_json::from_slice(payload).ok()?;
    if !command.is_object() || !replay.insert(&parsed.device_id, &parsed.nonce) {
        return None;
    }
    Some(command)
}

async fn send(socket: &UdpSocket, reply: &[u8], to: SocketAddr) {
    let _ = socket.send_to(reply, to).await;
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}
