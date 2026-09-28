use super::*;
use crate::{
    operations::{OperationBody, RequestId},
    session::{SessionGeneration, SessionNonce},
    StoreRevision,
};
use qol_conventions::{
    operations::{OperationKey, OperationKind},
    plugin_id::PluginUid,
};
#[cfg(target_os = "linux")]
use std::time::Duration;
use std::time::SystemTime;

fn key() -> OperationKey {
    OperationKey::new(
        PluginUid::new("fixture-uid"),
        OperationKind::Action,
        "count",
    )
}

fn linked(hub: &Hub) -> (AuthenticatedSession, SessionIo, Invocation) {
    let authority = &hub.authority;
    let remote = crate::service::Identity::generate(SystemTime::now()).unwrap();
    authority
        .insert_link(
            authority.projection().unwrap().revision,
            remote.pin().clone(),
            "remote".into(),
        )
        .unwrap();
    let session = AuthenticatedSession {
        local_peer: authority.local_pin().unwrap().peer_id(),
        remote_peer: remote.pin().peer_id(),
        generation: SessionGeneration {
            local: SessionNonce::from_random([1; 16]),
            remote: SessionNonce::from_random([2; 16]),
        },
    };
    authority
        .set_grants(
            authority.projection().unwrap().revision,
            session.remote_peer,
            vec![key()],
        )
        .unwrap();
    let io = hub.elect(session).unwrap();
    hub.epoch(session, OperationEpoch(SessionNonce::from_random([3; 16])))
        .unwrap();
    let body = serde_json::to_string(&OperationBody {
        version: 1,
        key: key(),
        arguments: "{}".into(),
        timeout_ms: 10_000,
    })
    .unwrap();
    let invocation = Invocation {
        handle: RequestHandle {
            recipient: session.local_peer,
            epoch: authority.operation_epoch(session.remote_peer).unwrap(),
            sequence: StoreRevision::new(1),
            id: RequestId(SessionNonce::from_random([4; 16])),
            body_digest: crate::service::body_digest(&body),
        },
        body,
    };
    (session, io, invocation)
}

#[test]
fn operation_queue_owns_sixteen_slots_and_releases_encoded_bytes() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let (hub, mut workers) = Hub::new(authority, None);
    let mut retained = Vec::new();
    for index in 0..17 {
        let (session, io, invocation) = linked(&hub);
        let result = hub.enqueue(session, invocation.clone());
        if index < 16 {
            assert!(result.is_none());
            assert!(hub.enqueue(session, invocation).is_none());
        } else {
            assert_eq!(result, Some(Err(Failure::Busy)));
        }
        retained.push((session, io));
    }
    assert_eq!(hub.slots.available_permits(), 0);
    assert!(hub.bytes.available_permits() < 1024 * 1024);
    for _ in 0..16 {
        let worker = workers.try_recv().unwrap();
        hub.completed(hub.execute(worker)).unwrap();
    }
    assert_eq!(hub.slots.available_permits(), 16);
    drop(retained);
    hub.stop();
    assert_eq!(hub.bytes.available_permits(), 1024 * 1024);
    assert!(hub.busy.lock().unwrap().is_empty());
}

#[test]
fn operation_byte_capacity_refuses_before_queueing_and_stale_close_keeps_replacement() {
    let authority = PeerAuthority::session("local".into(), SystemTime::now()).unwrap();
    let (hub, mut workers) = Hub::new(authority, None);
    let (session, _io, invocation) = linked(&hub);
    let reservation = hub
        .bytes
        .clone()
        .try_acquire_many_owned(1024 * 1024)
        .unwrap();
    assert_eq!(
        hub.enqueue(session, invocation.clone()),
        Some(Err(Failure::Capacity))
    );
    assert!(workers.try_recv().is_err());
    assert_eq!(hub.slots.available_permits(), 16);
    drop(reservation);
    let replacement = AuthenticatedSession {
        generation: SessionGeneration {
            local: SessionNonce::from_random([8; 16]),
            ..session.generation
        },
        ..session
    };
    let _replacement_io = hub.elect(replacement).unwrap();
    hub.epoch(
        replacement,
        OperationEpoch(SessionNonce::from_random([3; 16])),
    )
    .unwrap();
    hub.close(session);
    assert!(hub.ready(session.remote_peer));
    assert_eq!(
        hub.receive(session, OperationMessage::Invoke { invocation }),
        Err(Failure::StaleSession)
    );
    hub.stop();
    assert!(hub.elect(replacement).is_err());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn operation_shutdown_joins_actual_started_closure_and_keeps_writer_owned() {
    struct Dispatcher {
        started: blocking::SyncSender<()>,
        release: Mutex<blocking::Receiver<()>>,
    }
    impl OperationDispatcher for Dispatcher {
        fn invoke(
            &self,
            authority: &PeerAuthority,
            session: AuthenticatedSession,
            invocation: &Invocation,
        ) -> Reply {
            authority.admit_operation(session, invocation, "d".repeat(43))?;
            authority.start_operation(session, invocation, &"d".repeat(43))?;
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            authority.settle_operation(
                session.remote_peer,
                &invocation.handle,
                Outcome::Acknowledged,
            )
        }
    }
    struct Discovery;
    impl crate::service::network::discovery::DiscoveryFactory for Discovery {
        fn run(
            &self,
            _: crate::service::network::discovery::Advertisement,
            events: mpsc::Sender<crate::service::network::discovery::DiscoveryEvent>,
            mut stop: tokio::sync::watch::Receiver<bool>,
        ) -> crate::service::network::discovery::DiscoveryFuture {
            Box::pin(async move {
                let _events = events;
                while !*stop.borrow() {
                    if stop.changed().await.is_err() {
                        break;
                    }
                }
                Ok(())
            })
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let authority =
        PeerAuthority::create_persistent(&root, "local".into(), SystemTime::now()).unwrap();
    let (started, observed) = blocking::sync_channel(1);
    let (release, gate) = blocking::sync_channel(1);
    let dispatcher = Arc::new(Dispatcher {
        started,
        release: Mutex::new(gate),
    });
    let (control, task) = crate::service::network::prepare_with_operations(
        authority.clone(),
        crate::service::network::NetworkOptions {
            bind: std::net::Ipv4Addr::LOCALHOST,
            allow_loopback_hints: true,
            discovery: Arc::new(Discovery),
        },
        dispatcher,
    );
    let operations = control.operations.upgrade().unwrap();
    let (session, _io, invocation) = linked(&operations);
    let completed_invocation = invocation.clone();
    assert!(operations.enqueue(session, invocation).is_none());
    drop(operations);
    drop(authority);
    let task = tokio::spawn(task);
    tokio::time::timeout(Duration::from_secs(3), async {
        while observed.try_recv().is_err() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    control.stop();
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    assert_eq!(
        PeerAuthority::open_persistent(&root, SystemTime::now()).err(),
        Some(crate::AuthorityError::WriterBusy)
    );
    release.send(()).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap(),
        Ok(())
    );
    assert!(PeerAuthority::open_persistent(&root, SystemTime::now()).is_ok());
    assert!(control.operations.upgrade().is_none());
    assert!(!control.operations_ready(session.remote_peer));
    assert_eq!(
        control.invoke_operation(completed_invocation.clone()).err(),
        Some(Failure::Unavailable)
    );
    for cancel in [false, true] {
        assert_eq!(
            control
                .reconcile_operation(completed_invocation.handle.clone(), cancel)
                .err(),
            Some(Failure::Unavailable)
        );
    }
}
