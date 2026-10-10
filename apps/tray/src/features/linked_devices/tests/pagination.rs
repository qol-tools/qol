use qol_peers::admin::{ActivationId, PageCursor, GRANTS_PER_PAGE, PEERS_PER_PAGE};
use qol_peers::{AuthorityProjection, AuthorityStatus, PeerId, PeerProjection};
use qol_plugin_api::operations::{OperationKey, OperationKind};

use super::*;
use crate::features::linked_devices::projection;

fn identity(index: u32) -> PeerId {
    use base64::Engine;
    let mut bytes = [0; 32];
    bytes[..4].copy_from_slice(&index.to_be_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(bytes)
        .parse()
        .unwrap()
}

fn maximum_projection() -> AuthorityProjection {
    let peer_id = identity(0);
    let grants: Vec<_> = (0..128)
        .map(|index| {
            OperationKey::new(
                crate::plugins::manifest::PluginUid::new("\\\"".repeat(128)),
                OperationKind::Query,
                format!("g{index:03}{}", "a".repeat(252)),
            )
        })
        .collect();
    AuthorityProjection {
        peer_id,
        name: "\\\"".repeat(128),
        lifetime: AuthorityLifetime::Persistent,
        revision: StoreRevision::new(u64::MAX),
        status: AuthorityStatus::Ready,
        peers: (1..257)
            .map(|index| PeerProjection {
                peer_id: identity(index),
                name: "\\\"".repeat(128),
                grants: grants.clone(),
            })
            .collect(),
    }
}

fn assert_bounded(reply: &Response) {
    assert!(serde_json::to_vec(reply).unwrap().len() < qol_runtime::local_ipc::MAX_MESSAGE_BYTES);
}

#[test]
fn maximum_projection_pages_preserve_all_items_and_bound_serialized_replies() {
    let projection = maximum_projection();
    let activation_id = ActivationId::from_bytes([1; 16]);
    let cursor = PageCursor {
        authority_id: projection.peer_id,
        activation_id,
        revision: projection.revision,
        offset: 0,
    };
    let summary = projection::summary(projection.clone(), activation_id);
    assert_eq!((summary.peer_count, summary.grant_count), (256, 32768));
    assert_bounded(&Response::Status {
        status: Status {
            lifecycle: Lifecycle::Active,
            authority: Some(summary),
        },
    });
    for (count, page_size, kind) in [
        (256, PEERS_PER_PAGE, "peers"),
        (128, GRANTS_PER_PAGE, "grants"),
    ] {
        let mut next = Some(cursor);
        let mut observed = 0;
        while let Some(cursor) = next {
            let reply = match kind {
                "peers" => projection::peers(projection.clone(), activation_id, cursor),
                "grants" => projection::grants(
                    projection.clone(),
                    activation_id,
                    projection.peers[0].peer_id,
                    cursor,
                ),
                _ => unreachable!(),
            }
            .unwrap();
            assert_bounded(&reply);
            let (length, total, following) = match reply {
                Response::Peers { page } => (page.items.len(), page.total, page.next),
                Response::Grants { page, .. } => (page.items.len(), page.total, page.next),
                _ => panic!("page required"),
            };
            assert!(length <= page_size, "{kind}");
            assert_eq!(total, count, "{kind}");
            observed += length;
            next = following;
        }
        assert_eq!(observed, count as usize, "{kind}");
    }
}

#[test]
fn cursors_reject_changed_identity_revision_and_out_of_range_offsets() {
    let projection = maximum_projection();
    let activation_id = ActivationId::from_bytes([1; 16]);
    for (authority_id, revision, offset, error) in [
        (identity(1), projection.revision, 0, Error::StaleCursor),
        (
            projection.peer_id,
            StoreRevision::INITIAL,
            0,
            Error::StaleCursor,
        ),
        (
            projection.peer_id,
            projection.revision,
            u32::MAX,
            Error::InvalidCursor,
        ),
    ] {
        assert_eq!(
            projection::peers(
                projection.clone(),
                activation_id,
                PageCursor {
                    authority_id,
                    activation_id,
                    revision,
                    offset
                }
            ),
            Err(error)
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn real_store_pages_are_complete_and_old_cursors_fail_after_a_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("peers");
    let peers = super::persistent::populated(&root, 17);
    let host = host_at(&root, false);
    let shared = attach(&host);
    let view = authority(&shared);
    let cursor = PageCursor {
        authority_id: view.peer_id,
        activation_id: view.activation_id,
        revision: view.revision,
        offset: 0,
    };
    let Response::Peers { page } = shared.peer_admin(Request::Peers { cursor }) else {
        panic!("peers page");
    };
    assert_eq!(page.total, 17);
    assert_eq!(page.items.len(), PEERS_PER_PAGE);
    let next = page.next.unwrap();
    let Response::Peers { page } = shared.peer_admin(Request::Peers { cursor: next }) else {
        panic!("peers page");
    };
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next, None);
    let mut grants = Vec::new();
    let mut next = Some(cursor);
    while let Some(cursor) = next {
        let reply = shared.peer_admin(Request::Grants {
            peer_id: peers[0],
            cursor,
        });
        assert_bounded(&reply);
        let Response::Grants { page, .. } = reply else {
            panic!("grants page");
        };
        grants.extend(page.items);
        next = page.next;
    }
    assert_eq!(grants.len(), 128);
    assert_eq!(
        grants
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        128
    );
    shared.peer_admin(Request::Rename {
        expected: view.expected(),
        name: "new".into(),
    });
    assert_eq!(
        shared.peer_admin(Request::Peers { cursor }),
        Response::Error {
            error: Error::StaleCursor
        }
    );
}
