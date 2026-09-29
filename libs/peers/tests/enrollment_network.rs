#![cfg(feature = "service")]

use std::{
    net::Ipv4Addr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use qol_peers::{
    admin::{AttemptState, EnrollmentFailure, Error},
    service::{
        network::{
            discovery::{Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture},
            prepare, NetworkOptions,
        },
        PeerAuthority,
    },
    StoreRevision,
};
use tokio::sync::{mpsc, watch};

struct Discovery;

impl DiscoveryFactory for Discovery {
    fn run(
        &self,
        _: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        Box::pin(async move {
            let _ = events.send(DiscoveryEvent::Ready).await;
            while !*stop.borrow_and_update() {
                if stop.changed().await.is_err() {
                    break;
                }
            }
            Ok(())
        })
    }
}

fn authority() -> PeerAuthority {
    PeerAuthority::session("fixture".into(), SystemTime::now()).unwrap()
}

fn revision(authority: &PeerAuthority) -> StoreRevision {
    authority.projection().unwrap().revision
}

fn options(loopback: bool) -> NetworkOptions {
    NetworkOptions {
        bind: Ipv4Addr::LOCALHOST,
        allow_loopback_hints: loopback,
        discovery: Arc::new(Discovery),
    }
}

#[tokio::test]
async fn admission_bounds_queued_work_without_spawning_or_evicting_transactions() {
    let local = authority();
    let (control, task) = prepare(local.clone(), options(true));
    let mut prepared = Vec::new();
    for _ in 0..9 {
        let remote = authority();
        let invitation = remote
            .create_invitation(vec![(Ipv4Addr::LOCALHOST, 1234).into()])
            .unwrap();
        let transaction = local.prepare_join(revision(&local), &invitation).unwrap();
        prepared.push((transaction, invitation.export().unwrap()));
    }
    let expected = revision(&local);
    for (transaction, document) in prepared.iter().take(8) {
        assert_eq!(
            control.admit_enrollment(
                &local,
                expected,
                *transaction,
                Some(document.clone()),
                vec![]
            ),
            Ok(AttemptState::Queued {})
        );
    }
    let (transaction, document) = &prepared[0];
    assert_eq!(
        control.admit_enrollment(
            &local,
            expected,
            *transaction,
            Some(document.clone()),
            vec![]
        ),
        Err(Error::Enrollment {
            error: EnrollmentFailure::AlreadyRunning
        })
    );
    let (transaction, document) = &prepared[8];
    assert_eq!(
        control.admit_enrollment(
            &local,
            expected,
            *transaction,
            Some(document.clone()),
            vec![]
        ),
        Err(Error::Enrollment {
            error: EnrollmentFailure::Capacity
        })
    );
    assert_eq!(local.outbound_enrollments().unwrap().len(), 9);
    assert_eq!(revision(&local), expected);
    control.stop();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap(),
        Ok(())
    );
    for (transaction, _) in prepared.iter().take(8) {
        assert!(matches!(
            control.enrollment_attempt(*transaction).unwrap(),
            AttemptState::Unknown { .. }
        ));
    }
}

#[tokio::test]
async fn original_transaction_pin_and_endpoint_policy_are_checked_before_admission() {
    let local = authority();
    let remote = authority();
    let invitation = remote
        .create_invitation(vec![(Ipv4Addr::LOCALHOST, 1234).into()])
        .unwrap();
    let transaction = local.prepare_join(revision(&local), &invitation).unwrap();
    let (control, task) = prepare(local.clone(), options(false));
    let expected = revision(&local);
    for (endpoints, error) in [
        (vec![], EnrollmentFailure::InvalidEndpoint),
        (vec!["127.0.0.1:1234"], EnrollmentFailure::InvalidEndpoint),
        (vec!["0.0.0.0:1234"], EnrollmentFailure::InvalidEndpoint),
        (vec!["224.0.0.1:1234"], EnrollmentFailure::InvalidEndpoint),
        (
            vec!["255.255.255.255:1234"],
            EnrollmentFailure::InvalidEndpoint,
        ),
        (vec!["192.0.2.1:0"], EnrollmentFailure::InvalidEndpoint),
        (
            vec!["[::1]:1234"],
            EnrollmentFailure::UnsupportedAddressFamily,
        ),
        (
            vec!["192.0.2.1:1234"; 9],
            EnrollmentFailure::InvalidEndpoint,
        ),
    ] {
        assert_eq!(
            control.admit_enrollment(
                &local,
                expected,
                transaction,
                None,
                endpoints
                    .iter()
                    .map(|value| value.parse().unwrap())
                    .collect()
            ),
            Err(Error::Enrollment { error }),
            "{endpoints:?}"
        );
    }
    let unknown = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
    assert_eq!(
        control.admit_enrollment(
            &local,
            expected,
            unknown,
            None,
            vec!["192.0.2.1:1234".parse().unwrap()]
        ),
        Err(Error::Enrollment {
            error: EnrollmentFailure::UnknownTransaction
        })
    );
    let other = authority()
        .create_invitation(vec!["192.0.2.1:1234".parse().unwrap()])
        .unwrap();
    assert_eq!(
        control.admit_enrollment(
            &local,
            expected,
            transaction,
            Some(other.export().unwrap()),
            vec![]
        ),
        Err(Error::Enrollment {
            error: EnrollmentFailure::Protocol
        })
    );
    local.abandon_join(expected, transaction).unwrap();
    assert_eq!(
        control.admit_enrollment(
            &local,
            revision(&local),
            transaction,
            None,
            vec!["192.0.2.1:1234".parse().unwrap()]
        ),
        Err(Error::Enrollment {
            error: EnrollmentFailure::Abandoned
        })
    );
    control.stop();
    task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn checked_invitation_expiry_and_stale_cancellation_preserve_the_authority() {
    let local = authority();
    let expected = revision(&local);
    let first = local.create_invitation_checked(expected, vec![]).unwrap();
    for _ in 1..8 {
        local.create_invitation_checked(expected, vec![]).unwrap();
    }
    assert!(local.create_invitation_checked(expected, vec![]).is_err());
    local.rename(expected, "renamed".into()).unwrap();
    assert!(matches!(
        local.cancel_invitation_checked(expected, first.id()),
        Err(qol_peers::service::enrollment::EnrollmentError::Authority(
            qol_peers::AuthorityError::StaleRevision { .. }
        ))
    ));
    assert!(local
        .create_invitation_checked(revision(&local), vec![])
        .is_err());
    tokio::time::advance(Duration::from_secs(121)).await;
    local
        .create_invitation_checked(revision(&local), vec![])
        .unwrap();
    assert_eq!(revision(&local).value(), 1);
    assert!(local.projection().unwrap().peers.is_empty());
}

#[tokio::test]
async fn inbound_tls_capacity_and_shutdown_are_owned_without_stopping_normal_listener() {
    let local = authority();
    let remote = authority();
    let (control, task) = prepare(local.clone(), options(true));
    let task = tokio::spawn(task);
    let mut view = control.subscribe();
    let port = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let qol_peers::network::ListenerStatus::Listening { port } =
                view.borrow().status.enrollment_listener
            {
                break port;
            }
            view.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let config = remote
        .enrollment_client_config(local.local_pin().unwrap())
        .unwrap();
    let mut connections = Vec::new();
    for _ in 0..8 {
        let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        connections.push(
            tokio::time::timeout(Duration::from_secs(2), config.connect(stream))
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), config.connect(stream))
            .await
            .is_err()
    );
    assert!(matches!(
        control.snapshot().status.listener,
        qol_peers::network::ListenerStatus::Listening { .. }
    ));
    control.stop();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap(),
        Ok(())
    );
    assert_eq!(
        control.snapshot().status.enrollment_listener,
        qol_peers::network::ListenerStatus::Closed {}
    );
    assert!(local.pending_enrollments().unwrap().is_empty());
    drop(connections);
}

#[test]
fn outbound_pages_bind_the_authority_revision_and_remain_bounded() {
    use qol_peers::admin::{ActivationId, PageCursor};
    let local = authority();
    for _ in 0..17 {
        let remote = authority();
        let invitation = remote.create_invitation(vec![]).unwrap();
        local.prepare_join(revision(&local), &invitation).unwrap();
    }
    let cursor = PageCursor {
        authority_id: local.local_pin().unwrap().peer_id(),
        activation_id: ActivationId::from_bytes([1; 16]),
        revision: revision(&local),
        offset: 0,
    };
    let page = local.outbound_enrollment_page(cursor).unwrap();
    assert_eq!(page.items.len(), 16);
    assert_eq!(page.total, 17);
    let next = page.next.unwrap();
    assert_eq!(next.offset, 16);
    assert_eq!(local.outbound_enrollment_page(next).unwrap().items.len(), 1);
    local
        .abandon_join(cursor.revision, page.items[0].key.transaction)
        .unwrap();
    assert_eq!(
        local.outbound_enrollment_page(next),
        Err(Error::StaleCursor)
    );
}
