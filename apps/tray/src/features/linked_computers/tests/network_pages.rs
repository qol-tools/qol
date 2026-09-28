use qol_peers::{
    admin::{ActivationId, NetworkRevision, SessionCursor},
    network::NetworkStatus,
    service::{network::NetworkSnapshot, PeerAuthority},
    session::{AuthenticatedSession, SessionGeneration},
};

use super::*;

#[test]
fn session_pages_derive_names_and_reject_each_stale_generation_of_the_view() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peers = super::persistent::populated(&root, 17, 0);
    let authority = PeerAuthority::open_persistent(&root, std::time::SystemTime::now()).unwrap();
    let projection = authority.projection().unwrap();
    let activation = ActivationId::from_bytes([1; 16]);
    let nonce = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
    let snapshot = NetworkSnapshot {
        revision: NetworkRevision(StoreRevision::new(8)),
        status: NetworkStatus::default(),
        sessions: peers
            .iter()
            .map(|peer| AuthenticatedSession {
                local_peer: projection.peer_id,
                remote_peer: *peer,
                generation: SessionGeneration {
                    local: nonce,
                    remote: nonce,
                },
            })
            .collect(),
    };
    let cursor = SessionCursor {
        authority_id: projection.peer_id,
        activation_id: activation,
        revision: projection.revision,
        network_revision: snapshot.revision,
        offset: 0,
    };
    let page = |cursor| {
        super::super::projection::session_page(
            projection.clone(),
            activation,
            snapshot.clone(),
            cursor,
        )
    };
    let first = page(cursor).unwrap();
    assert!(serde_json::to_vec(&first).unwrap().len() < qol_runtime::local_ipc::MAX_MESSAGE_BYTES);
    let Response::Sessions { page: first } = first else {
        panic!("session page");
    };
    assert_eq!(first.items.len(), 16);
    assert_eq!(first.total, 17);
    for entry in &first.items {
        assert_eq!(
            entry.name,
            projection
                .peers
                .iter()
                .find(|peer| peer.peer_id == entry.peer_id)
                .unwrap()
                .name
        );
    }
    let Response::Sessions { page: second } = page(first.next.unwrap()).unwrap() else {
        panic!("session page");
    };
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.next, None);
    assert_eq!(second.cursor.offset, 16);
    for stale in [
        SessionCursor {
            authority_id: peers[0],
            ..cursor
        },
        SessionCursor {
            activation_id: ActivationId::from_bytes([2; 16]),
            ..cursor
        },
        SessionCursor {
            revision: StoreRevision::new(99),
            ..cursor
        },
        SessionCursor {
            network_revision: NetworkRevision(StoreRevision::new(9)),
            ..cursor
        },
    ] {
        assert_eq!(page(stale), Err(Error::StaleCursor), "{stale:?}");
    }
    assert_eq!(
        page(SessionCursor {
            offset: 18,
            ..cursor
        }),
        Err(Error::InvalidCursor)
    );
}
