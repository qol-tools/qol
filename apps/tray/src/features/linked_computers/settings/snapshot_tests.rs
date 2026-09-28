use qol_peers::admin::{
    ActivationId, Lifecycle, NetworkRevision, Page, PageCursor, Request, Response,
};
use qol_peers::StoreRevision;

use super::{load_with, pages, Failure};
use crate::features::linked_computers::tests::{attach, authority, host_at};

#[test]
fn settings_reads_an_attached_authority_without_creating_links() {
    let temporary = tempfile::tempdir().unwrap();
    let host = host_at(&temporary.path().join("peers"), false);
    let shared = attach(&host);
    for active in [false, true] {
        if active {
            shared.peer_admin(Request::StartSession {
                name: "fixture".into(),
            });
        }
        let snapshot = load_with(|request| {
            assert!(!request.is_mutation());
            match shared.peer_admin(request) {
                Response::Error { error } => Err(Failure::Authority(error)),
                response => Ok(response),
            }
        })
        .unwrap();
        assert_eq!(snapshot.status.authority.is_some(), active);
        assert!(snapshot.peers.is_empty());
        assert!(snapshot.sessions.is_empty());
        assert!(snapshot.pending.is_empty());
    }
    assert!(!temporary.path().join("peers").exists());
    host.shutdown();
}

#[test]
fn changing_authority_or_network_during_refresh_discards_the_snapshot() {
    for changed in ["activation", "revision", "network"] {
        let temporary = tempfile::tempdir().unwrap();
        let host = host_at(&temporary.path().join("peers"), false);
        let shared = attach(&host);
        shared.peer_admin(Request::StartSession {
            name: "fixture".into(),
        });
        let mut statuses = 0;
        let mut networks = 0;
        let result = load_with(|request| {
            let mut response = shared.peer_admin(request);
            match &mut response {
                Response::Status { status } => {
                    statuses += 1;
                    if statuses == 2 {
                        let authority = status.authority.as_mut().unwrap();
                        match changed {
                            "activation" => {
                                authority.activation_id = ActivationId::from_bytes([9; 16])
                            }
                            "revision" => authority.revision = StoreRevision::new(1),
                            _ => {}
                        }
                    }
                }
                Response::Network { network } => {
                    networks += 1;
                    if changed == "network" && networks == 2 {
                        network.network_revision = NetworkRevision(StoreRevision::new(1));
                    }
                }
                _ => {}
            }
            Ok(response)
        });
        assert!(matches!(result, Err(Failure::Inconsistent)), "{changed}");
        host.shutdown();
    }
}

#[test]
fn stopping_and_faulted_authorities_remain_visible_without_invalid_followup_reads() {
    let temporary = tempfile::tempdir().unwrap();
    let host = host_at(&temporary.path().join("peers"), false);
    let shared = attach(&host);
    shared.peer_admin(Request::StartSession {
        name: "fixture".into(),
    });
    for (lifecycle, status) in [
        (Lifecycle::Stopping, qol_peers::AuthorityStatus::Ready),
        (Lifecycle::Active, qol_peers::AuthorityStatus::Faulted),
    ] {
        let mut summary = authority(&shared);
        summary.status = status;
        let mut reads = 0;
        let snapshot = load_with(|request| {
            reads += 1;
            assert_eq!(request, Request::Status);
            Ok(Response::Status {
                status: qol_peers::admin::Status {
                    lifecycle,
                    authority: Some(summary.clone()),
                },
            })
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(snapshot.status.lifecycle, lifecycle);
        assert_eq!(snapshot.status.authority.unwrap().status, status);
    }
    host.shutdown();
}

#[test]
fn pagination_preserves_all_items_and_refuses_incoherent_continuations() {
    let temporary = tempfile::tempdir().unwrap();
    let host = host_at(&temporary.path().join("peers"), false);
    let shared = attach(&host);
    shared.peer_admin(Request::StartSession {
        name: "fixture".into(),
    });
    let expected = authority(&shared).expected();
    for changed in [
        "none",
        "activation",
        "revision",
        "offset",
        "total",
        "truncated",
    ] {
        let result = pages(expected, |cursor| {
            let mut page = Page {
                cursor,
                total: 3,
                items: if cursor.offset == 0 {
                    vec![1, 2]
                } else {
                    vec![3]
                },
                next: (cursor.offset == 0).then_some(PageCursor {
                    offset: 2,
                    ..cursor
                }),
            };
            if let Some(next) = &mut page.next {
                match changed {
                    "activation" => next.activation_id = ActivationId::from_bytes([9; 16]),
                    "revision" => next.revision = StoreRevision::new(1),
                    "offset" => next.offset = 0,
                    _ => {}
                }
            }
            if cursor.offset > 0 {
                match changed {
                    "total" => page.total = 4,
                    "truncated" => page.items.clear(),
                    _ => {}
                }
            }
            Ok(page)
        });
        match changed {
            "none" => assert_eq!(result.unwrap(), vec![1, 2, 3]),
            _ => assert!(matches!(result, Err(Failure::Inconsistent)), "{changed}"),
        }
    }
    host.shutdown();
}
