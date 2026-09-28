mod lifecycle;
mod sessions;

use std::{
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime},
};

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::{
    discovery::{cancelled, Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture},
    prepare, NetworkControl, NetworkOptions, NetworkSnapshot,
};
use crate::{
    network::{ListenerStatus, NetworkFailure},
    service::PeerAuthority,
};

struct FakeDiscovery {
    incoming: Mutex<Option<mpsc::Receiver<DiscoveryEvent>>>,
    advertised: watch::Sender<Option<Advertisement>>,
    release: watch::Receiver<bool>,
    closed: Arc<AtomicUsize>,
    cleanup_failure: bool,
}

impl DiscoveryFactory for FakeDiscovery {
    fn run(
        &self,
        advertisement: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        let mut incoming = self.incoming.lock().unwrap().take().unwrap();
        self.advertised.send_replace(Some(advertisement));
        let mut release = self.release.clone();
        let closed = self.closed.clone();
        let cleanup_failure = self.cleanup_failure;
        Box::pin(async move {
            loop {
                tokio::select! {
                    biased;
                    () = cancelled(&mut stop) => break,
                    event = incoming.recv() => {
                        let Some(event) = event else { break; };
                        if events.send(event).await.is_err() { break; }
                    },
                }
            }
            drop(events);
            cancelled(&mut release).await;
            closed.fetch_add(1, Ordering::SeqCst);
            if cleanup_failure {
                return Err(NetworkFailure::Cleanup);
            }
            Ok(())
        })
    }
}

struct Fixture {
    control: NetworkControl,
    task: JoinHandle<Result<(), NetworkFailure>>,
    events: mpsc::Sender<DiscoveryEvent>,
    advertised: watch::Receiver<Option<Advertisement>>,
    release: watch::Sender<bool>,
    closed: Arc<AtomicUsize>,
}

impl Fixture {
    fn start(authority: PeerAuthority) -> Self {
        let (events, incoming) = mpsc::channel(64);
        let (advertised, observation) = watch::channel(None);
        let (release, receipt) = watch::channel(true);
        let closed = Arc::new(AtomicUsize::new(0));
        let discovery = Arc::new(FakeDiscovery {
            incoming: Mutex::new(Some(incoming)),
            advertised,
            release: receipt,
            closed: closed.clone(),
            cleanup_failure: false,
        });
        let (control, task) = prepare(
            authority,
            NetworkOptions {
                bind: Ipv4Addr::LOCALHOST,
                allow_loopback_hints: true,
                discovery,
            },
        );
        Self {
            control,
            task: tokio::spawn(task),
            events,
            advertised: observation,
            release,
            closed,
        }
    }

    async fn port(&mut self) -> u16 {
        loop {
            if let Some(advertisement) = *self.advertised.borrow() {
                return advertisement.port;
            }
            self.advertised.changed().await.unwrap();
        }
    }

    async fn wait(&self, condition: impl Fn(&NetworkSnapshot) -> bool) -> NetworkSnapshot {
        let mut view = self.control.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = view.borrow().clone();
                if condition(&snapshot) {
                    return snapshot;
                }
                view.changed().await.unwrap();
            }
        })
        .await
        .unwrap()
    }

    async fn stop(self) {
        self.control.stop();
        self.release.send_replace(true);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), self.task)
                .await
                .unwrap()
                .unwrap(),
            Ok(())
        );
        assert_eq!(self.closed.load(Ordering::SeqCst), 1);
    }
}

fn authority(name: &str) -> PeerAuthority {
    PeerAuthority::session(name.into(), SystemTime::now()).unwrap()
}

fn linked() -> (PeerAuthority, PeerAuthority) {
    let left = authority("left");
    let right = authority("right");
    for (local, remote) in [(&left, &right), (&right, &left)] {
        local
            .insert_link(
                local.projection().unwrap().revision,
                remote.local_pin().unwrap(),
                "remote".into(),
            )
            .unwrap();
    }
    (left, right)
}

fn hint(peer: crate::PeerId, port: u16) -> DiscoveryEvent {
    DiscoveryEvent::Resolved {
        source: peer.to_string(),
        peer,
        endpoints: vec![std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)],
    }
}
