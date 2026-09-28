use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::Duration,
};

use qol_peers::admin::{Lifecycle, NearbyDevice, NearbyRequest, NearbyState, Request, Response};
use qol_peers::service::network::discovery::{
    Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture, NearbyAdvertisement,
};
use qol_peers::{AuthorityLifetime, PeerId};
use tokio::sync::{mpsc, watch};

use super::enrollment::Fixture;
use super::{authority, status};
use crate::features::linked_devices::Defaults;

type Members = Arc<Mutex<Vec<(Advertisement, mpsc::Sender<DiscoveryEvent>)>>>;

#[derive(Clone, Default)]
struct Lan(Members);

impl DiscoveryFactory for Lan {
    fn run(
        &self,
        advertisement: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        let members = self.0.clone();
        Box::pin(async move {
            let _ = events.send(DiscoveryEvent::Ready).await;
            let others = {
                let mut members = members.lock().unwrap();
                let others = members.clone();
                members.push((advertisement.clone(), events.clone()));
                others
            };
            for (other, sender) in others {
                let _ = sender.send(resolved(&advertisement)).await;
                let _ = events.send(resolved(&other)).await;
            }
            while !*stop.borrow_and_update() {
                if stop.changed().await.is_err() {
                    break;
                }
            }
            Ok(())
        })
    }
}

fn resolved(advertisement: &Advertisement) -> DiscoveryEvent {
    DiscoveryEvent::Resolved {
        source: advertisement.peer.to_string(),
        peer: advertisement.peer,
        endpoints: vec![SocketAddrV4::new(Ipv4Addr::LOCALHOST, advertisement.port)],
        claim: advertisement.link.map(|link| NearbyAdvertisement {
            name: advertisement.name.clone(),
            link,
        }),
    }
}

async fn device(root: &std::path::Path, lan: &Lan, defaults: Defaults) -> Fixture {
    let fixture = Fixture::with_discovery(root, Arc::new(lan.clone())).await;
    fixture.owner.handle.inner.lock().unwrap().defaults = defaults;
    fixture
}

fn nearby(fixture: &Fixture) -> Vec<NearbyDevice> {
    match fixture.shared.peer_admin(Request::Nearby {
        request: NearbyRequest::List {},
    }) {
        Response::Nearby { devices, .. } => devices,
        other => panic!("nearby list expected, got {other:?}"),
    }
}

async fn until<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(value) = probe() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition within ten seconds")
}

fn peer_id(fixture: &Fixture) -> PeerId {
    authority(&fixture.shared).peer_id
}

fn confirm(fixture: &Fixture, peer_id: PeerId) -> Response {
    fixture.shared.peer_admin(Request::Nearby {
        request: NearbyRequest::Confirm {
            expected: fixture.expected(),
            peer_id,
            grants: Vec::new(),
        },
    })
}

#[tokio::test]
async fn enabling_follows_residency_and_names_the_device() {
    let temporary = tempfile::tempdir().unwrap();
    let lan = Lan::default();
    let resident = device(
        &temporary.path().join("resident"),
        &lan,
        Defaults {
            resident: || true,
            name: || "Workstation".into(),
        },
    )
    .await;
    let portable = device(
        &temporary.path().join("portable"),
        &lan,
        Defaults {
            resident: || false,
            name: || "Borrowed".into(),
        },
    )
    .await;
    for (fixture, name, lifetime) in [
        (&resident, "Workstation", AuthorityLifetime::Persistent),
        (&portable, "Borrowed", AuthorityLifetime::Session),
    ] {
        assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Inactive);
        fixture.shared.peer_admin(Request::Enable);
        let active = authority(&fixture.shared);
        assert_eq!((active.name.as_str(), active.lifetime), (name, lifetime));
        fixture.shared.peer_admin(Request::Enable);
        assert_eq!(authority(&fixture.shared), active);
    }
    assert!(temporary.path().join("resident").exists());
    assert!(!temporary.path().join("portable").exists());
    let before = authority(&resident.shared);
    resident.shared.peer_admin(Request::Stop {
        expected: before.expected(),
    });
    until(|| (status(&resident.shared).lifecycle == Lifecycle::Inactive).then_some(())).await;
    resident.shared.peer_admin(Request::Enable);
    assert_eq!(authority(&resident.shared).peer_id, before.peer_id);
    resident.close().await;
    portable.close().await;
}

async fn pair(temporary: &std::path::Path) -> (Fixture, Fixture) {
    let lan = Lan::default();
    let laptop = device(
        &temporary.join("laptop"),
        &lan,
        Defaults {
            resident: || false,
            name: || "Laptop".into(),
        },
    )
    .await;
    let desk = device(
        &temporary.join("desk"),
        &lan,
        Defaults {
            resident: || false,
            name: || "Desk".into(),
        },
    )
    .await;
    laptop.shared.peer_admin(Request::Enable);
    desk.shared.peer_admin(Request::Enable);
    (laptop, desk)
}

fn link(fixture: &Fixture, peer_id: PeerId) -> Response {
    fixture.shared.peer_admin(Request::Nearby {
        request: NearbyRequest::Link {
            expected: fixture.expected(),
            peer_id,
            grants: Vec::new(),
        },
    })
}

fn code(fixture: &Fixture, peer: PeerId) -> Option<qol_peers::admin::LinkCode> {
    nearby(fixture)
        .into_iter()
        .find(|device| device.peer_id == peer)
        .and_then(|device| device.link)
        .filter(|link| link.state == NearbyState::Confirm {})
        .and_then(|link| link.code)
}

fn connected(fixture: &Fixture, peer: PeerId) -> bool {
    fixture
        .owner
        .handle
        .inner
        .lock()
        .unwrap()
        .authority()
        .unwrap()
        .network
        .as_ref()
        .is_some_and(|network| {
            network
                .snapshot()
                .sessions
                .iter()
                .any(|session| session.remote_peer == peer)
        })
}

fn linked(fixture: &Fixture, peer: PeerId) -> bool {
    fixture
        .owner
        .handle
        .inner
        .lock()
        .unwrap()
        .authority()
        .unwrap()
        .authority
        .projection()
        .unwrap()
        .peers
        .iter()
        .any(|linked| linked.peer_id == peer)
}

#[tokio::test]
async fn a_device_links_once_the_other_accepts_its_request() {
    let temporary = tempfile::tempdir().unwrap();
    let (laptop, desk) = pair(temporary.path()).await;
    let laptop_id = peer_id(&laptop);
    let desk_id = peer_id(&desk);
    let listed = until(|| {
        nearby(&desk)
            .into_iter()
            .find(|device| device.peer_id == laptop_id)
    })
    .await;
    assert_eq!(listed.name, "Laptop");
    assert_eq!(listed.link, None);

    let linking = link(&desk, laptop_id);
    assert!(matches!(linking, Response::Nearby { .. }), "{linking:?}");
    let laptop_code = until(|| code(&laptop, desk_id)).await;
    let desk_link = until(|| {
        nearby(&desk)
            .into_iter()
            .find(|device| device.peer_id == laptop_id)
            .and_then(|device| device.link)
            .filter(|link| link.state == NearbyState::WaitingForPeer {})
    })
    .await;
    assert_eq!(desk_link.code, Some(laptop_code));
    assert_eq!(nearby(&laptop)[0].name, "Desk");

    assert!(matches!(
        confirm(&laptop, desk_id),
        Response::Changed { .. }
    ));
    until(|| (linked(&laptop, desk_id) && linked(&desk, laptop_id)).then_some(())).await;
    until(|| (connected(&laptop, desk_id) && connected(&desk, laptop_id)).then_some(())).await;
    until(|| (nearby(&laptop).is_empty() && nearby(&desk).is_empty()).then_some(())).await;
    laptop.close().await;
    desk.close().await;
}

#[tokio::test]
async fn unlinking_forgets_the_link_on_both_devices_and_they_can_link_again() {
    let temporary = tempfile::tempdir().unwrap();
    let (laptop, desk) = pair(temporary.path()).await;
    let laptop_id = peer_id(&laptop);
    let desk_id = peer_id(&desk);
    until(|| {
        nearby(&desk)
            .iter()
            .any(|device| device.peer_id == laptop_id)
            .then_some(())
    })
    .await;
    link(&desk, laptop_id);
    until(|| code(&laptop, desk_id)).await;
    confirm(&laptop, desk_id);
    until(|| (connected(&laptop, desk_id) && connected(&desk, laptop_id)).then_some(())).await;
    let unlinked = laptop.shared.peer_admin(Request::Revoke {
        expected: laptop.expected(),
        peer_id: desk_id,
    });
    assert!(matches!(unlinked, Response::Changed { .. }), "{unlinked:?}");
    until(|| (!linked(&desk, laptop_id)).then_some(())).await;
    until(|| {
        (nearby(&laptop)
            .iter()
            .any(|device| device.peer_id == desk_id)
            && nearby(&desk)
                .iter()
                .any(|device| device.peer_id == laptop_id))
        .then_some(())
    })
    .await;
    laptop.close().await;
    desk.close().await;
}

#[tokio::test]
async fn two_devices_that_both_click_link_need_one_accept() {
    let temporary = tempfile::tempdir().unwrap();
    let (laptop, desk) = pair(temporary.path()).await;
    let laptop_id = peer_id(&laptop);
    let desk_id = peer_id(&desk);
    until(|| {
        (nearby(&laptop)
            .iter()
            .any(|device| device.peer_id == desk_id)
            && nearby(&desk)
                .iter()
                .any(|device| device.peer_id == laptop_id))
        .then_some(())
    })
    .await;
    link(&laptop, desk_id);
    link(&desk, laptop_id);
    let (accepter, requester) = if laptop_id < desk_id {
        ((&desk, laptop_id), (&laptop, desk_id))
    } else {
        ((&laptop, desk_id), (&desk, laptop_id))
    };
    until(|| code(accepter.0, accepter.1)).await;
    assert_eq!(code(requester.0, requester.1), None);
    assert!(matches!(
        confirm(accepter.0, accepter.1),
        Response::Changed { .. }
    ));
    until(|| (linked(&laptop, desk_id) && linked(&desk, laptop_id)).then_some(())).await;
    laptop.close().await;
    desk.close().await;
}

fn decline(fixture: &Fixture, peer_id: PeerId) -> Response {
    fixture.shared.peer_admin(Request::Nearby {
        request: NearbyRequest::Decline {
            expected: fixture.expected(),
            peer_id,
        },
    })
}

fn link_state(fixture: &Fixture, peer: PeerId) -> Option<NearbyState> {
    nearby(fixture)
        .into_iter()
        .find(|device| device.peer_id == peer)
        .and_then(|device| device.link)
        .map(|link| link.state)
}

#[tokio::test]
async fn a_cancelled_request_disappears_from_the_other_device() {
    let temporary = tempfile::tempdir().unwrap();
    let (laptop, desk) = pair(temporary.path()).await;
    let laptop_id = peer_id(&laptop);
    let desk_id = peer_id(&desk);
    until(|| {
        nearby(&desk)
            .iter()
            .any(|device| device.peer_id == laptop_id)
            .then_some(())
    })
    .await;
    link(&desk, laptop_id);
    until(|| code(&laptop, desk_id)).await;
    until(|| (link_state(&desk, laptop_id) == Some(NearbyState::WaitingForPeer {})).then_some(()))
        .await;
    assert!(matches!(
        decline(&desk, laptop_id),
        Response::Changed { .. }
    ));
    until(|| link_state(&laptop, desk_id).is_none().then_some(())).await;
    assert!(!matches!(
        confirm(&laptop, desk_id),
        Response::Changed { .. }
    ));
    assert!(!linked(&laptop, desk_id));
    laptop.close().await;
    desk.close().await;
}

#[tokio::test]
async fn a_declined_request_tells_the_requester() {
    let temporary = tempfile::tempdir().unwrap();
    let (laptop, desk) = pair(temporary.path()).await;
    let laptop_id = peer_id(&laptop);
    let desk_id = peer_id(&desk);
    until(|| {
        nearby(&desk)
            .iter()
            .any(|device| device.peer_id == laptop_id)
            .then_some(())
    })
    .await;
    link(&desk, laptop_id);
    until(|| code(&laptop, desk_id)).await;
    until(|| (link_state(&desk, laptop_id) == Some(NearbyState::WaitingForPeer {})).then_some(()))
        .await;
    assert!(matches!(
        decline(&laptop, desk_id),
        Response::Changed { .. }
    ));
    until(|| {
        matches!(
            link_state(&desk, laptop_id),
            Some(NearbyState::Failed { .. })
        )
        .then_some(())
    })
    .await;
    laptop.close().await;
    desk.close().await;
}
