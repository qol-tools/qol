use super::*;
use crate::network::{DiscoveryStatus, IdleReason};

const NO_PEERS: ListenerStatus = ListenerStatus::Idle {
    reason: IdleReason::NoLinkedPeers,
};
const NOTHING_OPEN: ListenerStatus = ListenerStatus::Idle {
    reason: IdleReason::NothingOpen,
};

impl Fixture {
    async fn stop_without_discovery(self) {
        self.control.stop();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), self.task)
                .await
                .unwrap()
                .unwrap(),
            Ok(())
        );
    }
}

#[tokio::test]
async fn without_peers_nothing_listens_and_nothing_is_advertised() {
    let owner = Fixture::start(authority("local"));
    let view = owner
        .wait(|view| {
            view.status.listener == NO_PEERS && view.status.enrollment_listener == NOTHING_OPEN
        })
        .await;
    assert_eq!(view.status.discovery, DiscoveryStatus::Closed {});
    assert!(owner.advertised.borrow().is_none());
    owner.stop_without_discovery().await;
}

#[tokio::test]
async fn the_first_linked_peer_starts_listening_and_the_last_removal_stops_it() {
    let local = authority("local");
    let remote = authority("remote");
    let remote_id = remote.local_pin().unwrap().peer_id();
    let mut owner = Fixture::start(local.clone());
    owner.wait(|view| view.status.listener == NO_PEERS).await;
    local
        .insert_link(
            local.projection().unwrap().revision,
            remote.local_pin().unwrap(),
            "remote".into(),
        )
        .unwrap();
    let port = owner.port().await;
    let view = owner
        .wait(|view| matches!(view.status.listener, ListenerStatus::Listening { .. }))
        .await;
    assert_eq!(view.status.listener, ListenerStatus::Listening { port });
    local
        .revoke(local.projection().unwrap().revision, remote_id)
        .unwrap();
    let view = owner.wait(|view| view.status.listener == NO_PEERS).await;
    assert_eq!(view.status.discovery, DiscoveryStatus::Closed {});
    assert_eq!(owner.closed.load(Ordering::SeqCst), 1);
    assert!(tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_err());
    owner.stop_without_discovery().await;
}

#[tokio::test(start_paused = true)]
async fn an_open_invitation_holds_the_enrollment_listener_until_it_closes() {
    let local = authority("local");
    let owner = Fixture::start(local.clone());
    owner
        .wait(|view| view.status.enrollment_listener == NOTHING_OPEN)
        .await;
    let endpoints = owner
        .control
        .invitation_endpoints(vec![Ipv4Addr::LOCALHOST.into()])
        .unwrap();
    let port = endpoints[0].port();
    let view = owner
        .wait(|view| {
            matches!(
                view.status.enrollment_listener,
                ListenerStatus::Listening { .. }
            )
        })
        .await;
    assert_eq!(
        view.status.enrollment_listener,
        ListenerStatus::Listening { port }
    );
    let invitation = local.create_invitation(endpoints).unwrap();
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        owner.control.snapshot().status.enrollment_listener,
        ListenerStatus::Listening { port }
    );
    local.cancel_invitation(invitation.id()).unwrap();
    owner
        .wait(|view| view.status.enrollment_listener == NOTHING_OPEN)
        .await;
    assert!(tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_err());
    owner.stop_without_discovery().await;
}

#[tokio::test(start_paused = true)]
async fn an_expired_invitation_closes_the_enrollment_listener() {
    let local = authority("local");
    let owner = Fixture::start(local.clone());
    let endpoints = owner
        .control
        .invitation_endpoints(vec![Ipv4Addr::LOCALHOST.into()])
        .unwrap();
    local.create_invitation(endpoints).unwrap();
    owner
        .wait(|view| {
            matches!(
                view.status.enrollment_listener,
                ListenerStatus::Listening { .. }
            )
        })
        .await;
    tokio::time::sleep(Duration::from_secs(125)).await;
    owner
        .wait(|view| view.status.enrollment_listener == NOTHING_OPEN)
        .await;
    owner.stop_without_discovery().await;
}

#[tokio::test(start_paused = true)]
async fn the_nearby_lease_advertises_the_link_port_and_lapses_to_idle() {
    let mut owner = Fixture::start(authority("local"));
    owner.wait(|view| view.status.listener == NO_PEERS).await;
    owner.control.open_to_nearby().unwrap();
    owner.port().await;
    let view = owner
        .wait(|view| {
            matches!(
                view.status.enrollment_listener,
                ListenerStatus::Listening { .. }
            )
        })
        .await;
    let ListenerStatus::Listening { port } = view.status.enrollment_listener else {
        unreachable!();
    };
    assert_eq!(
        owner
            .advertised
            .borrow()
            .as_ref()
            .and_then(|advertised| advertised.link),
        Some(port)
    );
    tokio::time::sleep(Duration::from_secs(12)).await;
    let view = owner
        .wait(|view| {
            view.status.listener == NO_PEERS && view.status.enrollment_listener == NOTHING_OPEN
        })
        .await;
    assert_eq!(view.status.discovery, DiscoveryStatus::Closed {});
    assert_eq!(owner.closed.load(Ordering::SeqCst), 1);
    owner.stop_without_discovery().await;
}
