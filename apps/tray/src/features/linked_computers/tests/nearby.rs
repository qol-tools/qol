use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::Duration,
};

use qol_peers::admin::{Lifecycle, NearbyComputer, NearbyRequest, NearbyState, Request, Response};
use qol_peers::service::network::discovery::{
    Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture, NearbyAdvertisement,
};
use qol_peers::{AuthorityLifetime, PeerId};
use tokio::sync::{mpsc, watch};

use super::enrollment::Fixture;
use super::{authority, status};
use crate::features::linked_computers::Defaults;

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

async fn computer(root: &std::path::Path, lan: &Lan, defaults: Defaults) -> Fixture {
    let fixture = Fixture::with_discovery(root, Arc::new(lan.clone())).await;
    fixture.owner.handle.inner.lock().unwrap().defaults = defaults;
    fixture
}

fn nearby(fixture: &Fixture) -> Vec<NearbyComputer> {
    match fixture.shared.peer_admin(Request::Nearby {
        request: NearbyRequest::List {},
    }) {
        Response::Nearby { computers, .. } => computers,
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
async fn enabling_follows_residency_and_names_the_computer() {
    let temporary = tempfile::tempdir().unwrap();
    let lan = Lan::default();
    let resident = computer(
        &temporary.path().join("resident"),
        &lan,
        Defaults {
            resident: || true,
            name: || "Workstation".into(),
        },
    )
    .await;
    let portable = computer(
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

#[tokio::test]
async fn two_nearby_computers_link_after_both_confirm_the_same_code() {
    let temporary = tempfile::tempdir().unwrap();
    let lan = Lan::default();
    let laptop = computer(
        &temporary.path().join("laptop"),
        &lan,
        Defaults {
            resident: || false,
            name: || "Laptop".into(),
        },
    )
    .await;
    let desk = computer(
        &temporary.path().join("desk"),
        &lan,
        Defaults {
            resident: || false,
            name: || "Desk".into(),
        },
    )
    .await;
    laptop.shared.peer_admin(Request::Enable);
    desk.shared.peer_admin(Request::Enable);
    let laptop_id = peer_id(&laptop);
    let desk_id = peer_id(&desk);
    let listed = until(|| {
        nearby(&desk)
            .into_iter()
            .find(|computer| computer.peer_id == laptop_id)
    })
    .await;
    assert_eq!(listed.name, "Laptop");
    assert_eq!(listed.link, None);

    let linking = desk.shared.peer_admin(Request::Nearby {
        request: NearbyRequest::Link {
            expected: desk.expected(),
            peer_id: laptop_id,
        },
    });
    assert!(matches!(linking, Response::Nearby { .. }), "{linking:?}");
    let desk_code = until(|| {
        nearby(&desk)
            .into_iter()
            .find(|computer| computer.peer_id == laptop_id)
            .and_then(|computer| computer.link)
            .filter(|link| link.state == NearbyState::Confirm {})
            .and_then(|link| link.code)
    })
    .await;
    let laptop_view = until(|| {
        nearby(&laptop)
            .into_iter()
            .find(|computer| computer.peer_id == desk_id)
    })
    .await;
    assert_eq!(laptop_view.name, "Desk");
    let laptop_link = laptop_view.link.unwrap();
    assert_eq!(laptop_link.code, Some(desk_code));
    assert_eq!(laptop_link.state, NearbyState::Confirm {});

    assert!(matches!(
        confirm(&laptop, desk_id),
        Response::Changed { .. }
    ));
    assert_eq!(
        nearby(&laptop)[0].link.as_ref().unwrap().state,
        NearbyState::WaitingForPeer {}
    );
    assert!(matches!(
        confirm(&desk, laptop_id),
        Response::Changed { .. }
    ));
    until(|| {
        let linked = |fixture: &Fixture, peer: PeerId| {
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
        };
        (linked(&laptop, desk_id) && linked(&desk, laptop_id)).then_some(())
    })
    .await;
    assert!(nearby(&laptop).is_empty());
    until(|| nearby(&desk).is_empty().then_some(())).await;
    laptop.close().await;
    desk.close().await;
}
