use super::*;
use crate::network::DiscoveryStatus;

#[tokio::test]
async fn queued_start_is_not_readiness_and_late_discovery_failure_preserves_listener() {
    let mut owner = Fixture::start(authority("local"));
    assert_eq!(
        owner.control.snapshot().status.discovery,
        DiscoveryStatus::Starting {}
    );
    owner.port().await;
    assert!(matches!(
        owner.control.snapshot().status.listener,
        ListenerStatus::Listening { .. }
    ));
    assert_eq!(
        owner.control.snapshot().status.discovery,
        DiscoveryStatus::Starting {}
    );
    owner.events.send(DiscoveryEvent::Ready).await.unwrap();
    owner
        .wait(|view| view.status.discovery == DiscoveryStatus::Ready {})
        .await;
    owner.events.send(DiscoveryEvent::Failed).await.unwrap();
    owner
        .wait(|view| matches!(view.status.discovery, DiscoveryStatus::Failed { .. }))
        .await;
    assert!(matches!(
        owner.control.snapshot().status.listener,
        ListenerStatus::Listening { .. }
    ));
    owner.stop().await;
}

#[tokio::test]
async fn dropping_control_cancels_but_completion_waits_for_discovery_receipt() {
    let mut owner = Fixture::start(authority("local"));
    owner.port().await;
    owner.release.send_replace(false);
    drop(owner.control);
    tokio::task::yield_now().await;
    assert!(!owner.task.is_finished());
    assert_eq!(owner.closed.load(Ordering::SeqCst), 0);
    owner.release.send_replace(true);
    assert_eq!(owner.task.await.unwrap(), Ok(()));
    assert_eq!(owner.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn startup_failure_closes_the_partial_discovery_owner_and_leaves_listener_usable() {
    let mut owner = Fixture::start(authority("local"));
    let port = owner.port().await;
    let (unused, _) = mpsc::channel(1);
    drop(std::mem::replace(&mut owner.events, unused));
    owner
        .wait(|view| matches!(view.status.discovery, DiscoveryStatus::Failed { .. }))
        .await;
    assert_eq!(owner.closed.load(Ordering::SeqCst), 1);
    let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    owner.control.stop();
    assert_eq!(owner.task.await.unwrap(), Ok(()));
    drop(stream);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn persistent_writer_is_reacquired_only_after_all_network_work_is_reaped() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let authority =
        PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
    let identity = authority.local_pin().unwrap().peer_id();
    let mut owner = Fixture::start(authority);
    let port = owner.port().await;
    let _handshake = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    owner.release.send_replace(false);
    owner.control.stop();
    assert!(matches!(
        PeerAuthority::open_persistent(&root, SystemTime::now()),
        Err(crate::AuthorityError::WriterBusy)
    ));
    owner.release.send_replace(true);
    assert_eq!(owner.task.await.unwrap(), Ok(()));
    let reopened = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    assert_eq!(reopened.local_pin().unwrap().peer_id(), identity);
    assert!(owner.control.operations.upgrade().is_none());
}

struct ConstructorFailure;

impl DiscoveryFactory for ConstructorFailure {
    fn run(
        &self,
        _: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        _: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        Box::pin(async move {
            let _ = events.send(DiscoveryEvent::Failed).await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn discovery_constructor_failure_does_not_lose_the_listener_or_authority() {
    let authority = authority("local");
    let identity = authority.local_pin().unwrap().peer_id();
    let (control, task) = prepare(
        authority.clone(),
        NetworkOptions {
            bind: Ipv4Addr::LOCALHOST,
            allow_loopback_hints: true,
            discovery: Arc::new(ConstructorFailure),
        },
    );
    let task = tokio::spawn(task);
    let mut view = control.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                view.borrow().status.discovery,
                DiscoveryStatus::Failed { .. }
            ) {
                break;
            }
            view.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        view.borrow().status.listener,
        ListenerStatus::Listening { .. }
    ));
    assert_eq!(authority.local_pin().unwrap().peer_id(), identity);
    control.stop();
    assert_eq!(task.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn closed_discovery_events_with_pending_cleanup_allow_trusted_inbound_tls() {
    use crate::service::session::NormalSession;

    let (local, remote) = linked();
    let local_id = local.local_pin().unwrap().peer_id();
    let remote_id = remote.local_pin().unwrap().peer_id();
    let mut owner = Fixture::start(local);
    let port = owner.port().await;
    owner.release.send_replace(false);
    let (unused, _) = mpsc::channel(1);
    drop(std::mem::replace(&mut owner.events, unused));
    owner
        .wait(|view| {
            matches!(
                view.status.discovery,
                DiscoveryStatus::Failed {
                    error: NetworkFailure::Discovery
                }
            )
        })
        .await;
    assert_eq!(owner.closed.load(Ordering::SeqCst), 0);
    assert!(!owner.task.is_finished());
    let io = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let session = tokio::time::timeout(
        Duration::from_secs(3),
        NormalSession::connect(io, remote, local_id),
    )
    .await
    .unwrap()
    .unwrap();
    let authenticated = session.authenticated().unwrap();
    let (stop, mut stopping) = watch::channel(false);
    let client = tokio::spawn(async move { session.run(cancelled(&mut stopping)).await });
    let view = owner.wait(|view| view.sessions.len() == 1).await;
    assert_eq!(view.sessions[0].remote_peer, remote_id);
    assert_eq!(
        view.sessions[0].generation.local,
        authenticated.generation.remote
    );
    assert_eq!(
        view.sessions[0].generation.remote,
        authenticated.generation.local
    );
    assert!(matches!(
        view.status.listener,
        ListenerStatus::Listening { .. }
    ));
    assert_eq!(owner.closed.load(Ordering::SeqCst), 0);
    stop.send_replace(true);
    client.await.unwrap();
    owner.stop().await;
}

#[tokio::test(start_paused = true)]
async fn cancelled_discovery_that_never_completes_fails_within_the_owner_budget() {
    let mut owner = Fixture::start(authority("local"));
    owner.port().await;
    owner.release.send_replace(false);
    let mut view = owner.control.subscribe();
    owner.control.stop();
    while !view.borrow().status.stopping {
        view.changed().await.unwrap();
    }
    let started = tokio::time::Instant::now();
    assert_eq!(owner.task.await.unwrap(), Err(NetworkFailure::Cleanup));
    assert_eq!(
        tokio::time::Instant::now() - started,
        super::super::discovery::CLOSE_TIMEOUT
    );
    assert_eq!(owner.closed.load(Ordering::SeqCst), 0);
    assert!(owner.release.is_closed());
    assert_eq!(view.borrow().status.failure, Some(NetworkFailure::Cleanup));
    assert_eq!(view.borrow().status.listener, ListenerStatus::Closed {});
}

#[tokio::test(start_paused = true)]
async fn closed_event_channel_cleanup_deadline_is_not_restarted_by_stop() {
    let mut owner = Fixture::start(authority("local"));
    owner.port().await;
    owner.release.send_replace(false);
    let (unused, _) = mpsc::channel(1);
    drop(std::mem::replace(&mut owner.events, unused));
    owner
        .wait(|view| matches!(view.status.discovery, DiscoveryStatus::Failed { .. }))
        .await;
    let started = tokio::time::Instant::now();
    tokio::time::advance(Duration::from_secs(2)).await;
    owner.control.stop();
    assert_eq!(owner.task.await.unwrap(), Err(NetworkFailure::Cleanup));
    assert_eq!(
        tokio::time::Instant::now() - started,
        super::super::discovery::CLOSE_TIMEOUT
    );
    assert_eq!(owner.closed.load(Ordering::SeqCst), 0);
    assert!(owner.release.is_closed());
}

#[tokio::test(start_paused = true)]
async fn event_channel_close_exposes_cleanup_timeout_before_an_explicit_stop() {
    let mut owner = Fixture::start(authority("local"));
    owner.port().await;
    owner.release.send_replace(false);
    let (unused, _) = mpsc::channel(1);
    drop(std::mem::replace(&mut owner.events, unused));
    owner
        .wait(|view| matches!(view.status.discovery, DiscoveryStatus::Failed { .. }))
        .await;
    tokio::time::advance(super::super::discovery::CLOSE_TIMEOUT).await;
    let snapshot = owner
        .wait(|view| view.status.failure == Some(NetworkFailure::Cleanup))
        .await;
    assert!(matches!(
        snapshot.status.listener,
        ListenerStatus::Listening { .. }
    ));
    assert!(!owner.task.is_finished());
    assert!(!owner.release.is_closed());
    let started = tokio::time::Instant::now();
    owner.control.stop();
    assert_eq!(owner.task.await.unwrap(), Err(NetworkFailure::Cleanup));
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(owner.closed.load(Ordering::SeqCst), 0);
    assert!(owner.release.is_closed());
}
