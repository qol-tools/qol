use std::{
    net::Ipv4Addr,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use qol_peers::{
    admin::{NetworkSummary, SessionCursor},
    network::NetworkFailure,
    service::network::{
        discovery::{Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture},
        NetworkOptions,
    },
};
use tokio::sync::{mpsc, watch};

use super::*;

struct Discovery {
    started: watch::Sender<usize>,
    release: watch::Receiver<bool>,
    closed: Arc<AtomicUsize>,
    failure: bool,
}

impl DiscoveryFactory for Discovery {
    fn run(
        &self,
        _: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        self.started.send_modify(|count| *count += 1);
        let mut release = self.release.clone();
        let closed = self.closed.clone();
        let failure = self.failure;
        Box::pin(async move {
            let _ = events.send(DiscoveryEvent::Ready).await;
            while !*stop.borrow_and_update() {
                if stop.changed().await.is_err() {
                    break;
                }
            }
            while !*release.borrow_and_update() {
                if release.changed().await.is_err() {
                    break;
                }
            }
            closed.fetch_add(1, Ordering::SeqCst);
            if failure {
                return Err(NetworkFailure::Cleanup);
            }
            Ok(())
        })
    }
}

struct Fixture {
    owner: LinkedComputers,
    host: PeerHostHandle,
    shared: Arc<SharedState>,
    started: watch::Receiver<usize>,
    release: watch::Sender<bool>,
    closed: Arc<AtomicUsize>,
    _shutdown: tokio::sync::broadcast::Sender<()>,
}

impl Fixture {
    async fn new(root: &Path, shadow: bool, failure: bool) -> Self {
        let (started, observation) = watch::channel(0);
        let (release, receipt) = watch::channel(false);
        let closed = Arc::new(AtomicUsize::new(0));
        let options = NetworkOptions {
            bind: Ipv4Addr::LOCALHOST,
            allow_loopback_hints: true,
            discovery: Arc::new(Discovery {
                started,
                release: receipt,
                closed: closed.clone(),
                failure,
            }),
        };
        let (shutdown, rx) = tokio::sync::broadcast::channel(1);
        let owner = LinkedComputers::start_at(
            Ok(root.into()),
            Arc::new(Mutex::new(PluginManager::new())),
            shadow,
            rx,
            Some(options),
        )
        .await;
        let host = owner.handle();
        let shared = Arc::new(attach(&host));
        Self {
            owner,
            host,
            shared,
            started: observation,
            release,
            closed,
            _shutdown: shutdown,
        }
    }

    async fn request(&self, request: Request) -> Response {
        let shared = self.shared.clone();
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let worker = std::thread::spawn(move || {
            assert!(tokio::runtime::Handle::try_current().is_err());
            let _ = reply.send(shared.peer_admin(request));
        });
        let response = receiver.await.unwrap();
        worker.join().unwrap();
        response
    }

    async fn ready(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if *self.started.borrow() > 0 {
                    break;
                }
                self.started.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
}

fn network(shared: &SharedState) -> NetworkSummary {
    let Response::Network { network } = shared.peer_admin(Request::Network) else {
        panic!("network reply");
    };
    network
}

#[tokio::test]
async fn socket_worker_starts_on_captured_runtime_and_stop_waits_for_cleanup() {
    let temporary = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&temporary.path().join("peers"), false, false).await;
    fixture
        .request(Request::StartSession {
            name: "local".into(),
        })
        .await;
    fixture.ready().await;
    let before = network(&fixture.shared);
    let expected = before.authority;
    let stop = fixture.request(Request::Stop { expected }).await;
    assert!(matches!(
        stop,
        Response::Status {
            status: Status {
                lifecycle: Lifecycle::Stopping,
                ..
            }
        }
    ));
    assert_eq!(
        fixture
            .request(Request::StartSession {
                name: "replacement".into()
            })
            .await,
        Response::Error {
            error: Error::Stopping
        }
    );
    assert_eq!(fixture.closed.load(Ordering::SeqCst), 0);
    assert_eq!(authority(&fixture.shared).peer_id, expected.authority_id);
    assert!(network(&fixture.shared).status.stopping);
    fixture.release.send_replace(true);
    fixture.host.wait_stopped().await.unwrap();
    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Inactive);
    assert_eq!(fixture.closed.load(Ordering::SeqCst), 1);
    fixture.host.shutdown_and_wait().await.unwrap();
    fixture.owner.closed().await.unwrap();
}

#[tokio::test]
async fn dropping_owner_retains_the_reap_path_and_delays_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&temporary.path().join("peers"), false, false).await;
    fixture
        .request(Request::StartSession {
            name: "local".into(),
        })
        .await;
    fixture.ready().await;
    drop(fixture.owner);
    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Stopping);
    fixture.release.send_replace(true);
    fixture.host.wait_stopped().await.unwrap();
    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Shutdown);
    assert_eq!(fixture.closed.load(Ordering::SeqCst), 1);
    fixture.host.shutdown_and_wait().await.unwrap();
}

#[tokio::test]
async fn cleanup_failure_is_visible_and_blocks_an_overlapping_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&temporary.path().join("peers"), false, true).await;
    fixture
        .request(Request::StartSession {
            name: "local".into(),
        })
        .await;
    fixture.ready().await;
    fixture
        .request(Request::Stop {
            expected: authority(&fixture.shared).expected(),
        })
        .await;
    fixture.release.send_replace(true);
    assert_eq!(
        fixture.host.wait_stopped().await,
        Err(Error::HostUnavailable)
    );
    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Stopping);
    assert_eq!(
        network(&fixture.shared).status.failure,
        Some(NetworkFailure::Cleanup)
    );
    assert_eq!(
        fixture.request(Request::OpenPersistent).await,
        Response::Error {
            error: Error::Stopping
        }
    );
    fixture.host.shutdown();
    assert_eq!(fixture.owner.closed().await, Err(Error::HostUnavailable));
}

#[tokio::test]
async fn network_pages_bind_both_revisions_and_activation_without_resetting_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::new(&temporary.path().join("peers"), false, false).await;
    fixture
        .request(Request::StartSession {
            name: "local".into(),
        })
        .await;
    fixture.ready().await;
    let initial = network(&fixture.shared);
    let cursor = SessionCursor {
        authority_id: initial.authority.authority_id,
        activation_id: initial.authority.activation_id,
        revision: initial.authority.revision,
        network_revision: initial.network_revision,
        offset: 0,
    };
    assert!(
        matches!(fixture.request(Request::Sessions { cursor }).await, Response::Sessions { page } if page.total == 0)
    );
    for invalid in [
        SessionCursor {
            network_revision: qol_peers::admin::NetworkRevision(StoreRevision::new(1)),
            ..cursor
        },
        SessionCursor {
            revision: StoreRevision::new(1),
            ..cursor
        },
        SessionCursor {
            activation_id: qol_peers::admin::ActivationId::from_bytes([7; 16]),
            ..cursor
        },
    ] {
        assert_eq!(
            fixture.request(Request::Sessions { cursor: invalid }).await,
            Response::Error {
                error: Error::StaleCursor
            },
            "{invalid:?}"
        );
    }
    fixture
        .request(Request::Rename {
            expected: initial.authority,
            name: "renamed".into(),
        })
        .await;
    assert_eq!(
        network(&fixture.shared).network_revision,
        initial.network_revision
    );
    fixture.release.send_replace(true);
    fixture.host.shutdown_and_wait().await.unwrap();
    fixture.owner.closed().await.unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn standby_promotion_and_writer_reopen_use_the_real_host_request_owner() {
    use qol_peers::service::PeerAuthority;
    use std::time::SystemTime;
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let writer =
        PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
    let identity = writer.local_pin().unwrap().peer_id();
    let mut fixture = Fixture::new(&root, true, false).await;
    assert_eq!(*fixture.started.borrow(), 0);
    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Standby);
    assert_eq!(
        fixture.request(Request::OpenPersistent).await,
        Response::Error {
            error: Error::Standby
        }
    );
    drop(writer);
    fixture.shared.peers().unwrap().promote();
    fixture.ready().await;
    assert_eq!(authority(&fixture.shared).peer_id, identity);
    fixture
        .request(Request::Stop {
            expected: authority(&fixture.shared).expected(),
        })
        .await;
    assert!(matches!(
        PeerAuthority::open_persistent(&root, SystemTime::now()),
        Err(AuthorityError::WriterBusy)
    ));
    fixture.release.send_replace(true);
    fixture.host.wait_stopped().await.unwrap();
    assert!(matches!(
        fixture.request(Request::OpenPersistent).await,
        Response::Status {
            status: Status {
                lifecycle: Lifecycle::Active,
                ..
            }
        }
    ));
    assert_eq!(authority(&fixture.shared).peer_id, identity);
    tokio::time::timeout(Duration::from_secs(5), async {
        while *fixture.started.borrow() < 2 {
            fixture.started.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    fixture.host.shutdown_and_wait().await.unwrap();
    fixture.owner.closed().await.unwrap();
    assert_eq!(fixture.closed.load(Ordering::SeqCst), 2);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn failed_or_stalled_cleanup_keeps_the_persistent_writer_and_bounds_shutdown() {
    use qol_peers::service::PeerAuthority;
    use std::time::SystemTime;

    for (name, failure, release) in [("failed", true, true), ("stalled", false, false)] {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("peers");
        let writer =
            PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
        let identity = writer.local_pin().unwrap().peer_id();
        drop(writer);
        let mut fixture = Fixture::new(&root, false, failure).await;
        fixture.ready().await;
        let expected = authority(&fixture.shared).expected();
        assert!(
            matches!(
                fixture.request(Request::Stop { expected }).await,
                Response::Status {
                    status: Status {
                        lifecycle: Lifecycle::Stopping,
                        ..
                    }
                }
            ),
            "{name}"
        );
        fixture.release.send_replace(release);
        let stopped = tokio::time::timeout(Duration::from_secs(8), fixture.host.wait_stopped())
            .await
            .unwrap();
        assert_eq!(stopped, Err(Error::HostUnavailable), "{name}");
        assert_eq!(
            status(&fixture.shared).lifecycle,
            Lifecycle::Stopping,
            "{name}"
        );
        assert_eq!(authority(&fixture.shared).peer_id, identity, "{name}");
        assert_eq!(
            network(&fixture.shared).status.failure,
            Some(NetworkFailure::Cleanup),
            "{name}"
        );
        assert!(
            matches!(
                PeerAuthority::open_persistent(&root, SystemTime::now()),
                Err(AuthorityError::WriterBusy)
            ),
            "{name}"
        );
        assert_eq!(
            fixture.request(Request::OpenPersistent).await,
            Response::Error {
                error: Error::Stopping
            },
            "{name}"
        );
        assert_eq!(
            fixture
                .request(Request::StartSession {
                    name: "replacement".into()
                })
                .await,
            Response::Error {
                error: Error::Stopping
            },
            "{name}"
        );
        assert_eq!(
            fixture.closed.load(Ordering::SeqCst),
            usize::from(release),
            "{name}"
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), fixture.host.shutdown_and_wait())
                .await
                .unwrap(),
            Err(Error::HostUnavailable),
            "{name}"
        );
        assert_eq!(
            fixture.owner.closed().await,
            Err(Error::HostUnavailable),
            "{name}"
        );
        assert!(
            matches!(
                PeerAuthority::open_persistent(&root, SystemTime::now()),
                Err(AuthorityError::WriterBusy)
            ),
            "{name}"
        );
    }
}
