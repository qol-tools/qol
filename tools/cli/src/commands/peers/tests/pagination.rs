use qol_conventions::operations::OperationKey;
use qol_peers::admin::{Error, Page, PeerSummary};

use super::*;
use crate::commands::peers::input::Action;
use crate::commands::peers::pagination::Pages;

fn page<T>(item: T, offset: u32) -> Page<T> {
    Page {
        cursor: cursor(offset),
        total: 2,
        items: vec![item],
        next: (offset == 0).then(|| cursor(1)),
    }
}

fn peers(offset: u32) -> Response {
    Response::Peers {
        page: page(
            PeerSummary {
                peer_id: peer(),
                name: "fixture".into(),
                grant_count: 0,
            },
            offset,
        ),
    }
}

#[test]
fn every_projection_follows_only_returned_cursors_from_fresh_status() {
    let grant: OperationKey = serde_json::from_str(
        r#"{"identity":{"scope":"stable","value":"fixture"},"kind":"query","name":"read"}"#,
    )
    .unwrap();
    let cases = [
        (
            Pages::Peers,
            [peers(0), peers(1)],
            [
                Request::Peers { cursor: cursor(0) },
                Request::Peers { cursor: cursor(1) },
            ],
        ),
        (
            Pages::Grants(peer()),
            [
                Response::Grants {
                    peer_id: peer(),
                    page: page(grant.clone(), 0),
                },
                Response::Grants {
                    peer_id: peer(),
                    page: page(grant, 1),
                },
            ],
            [
                Request::Grants {
                    peer_id: peer(),
                    cursor: cursor(0),
                },
                Request::Grants {
                    peer_id: peer(),
                    cursor: cursor(1),
                },
            ],
        ),
    ];
    for (kind, replies, requests) in cases {
        let mut client =
            FakeClient::new([Ok(status()), Ok(replies[0].clone()), Ok(replies[1].clone())]);
        let result = execute(Action::Pages(kind), &mut |r| client.call(r)).unwrap();
        assert_eq!(result, replies, "{kind:?}");
        assert_eq!(
            client.requests,
            [Request::Status, requests[0].clone(), requests[1].clone()],
            "{kind:?}"
        );
    }
}

#[test]
fn stale_first_or_later_pages_fail_without_refetch_or_partial_results() {
    for successful_pages in 0..=1 {
        let mut replies = vec![Ok(status())];
        if successful_pages == 1 {
            replies.push(Ok(peers(0)));
        }
        replies.push(Ok(Response::Error {
            error: Error::StaleCursor,
        }));
        let mut client = FakeClient::new(replies);
        let result = execute(Action::Pages(Pages::Peers), &mut |r| client.call(r));
        assert!(
            result.unwrap_err().to_string().contains("stale_cursor"),
            "{successful_pages}"
        );
        assert_eq!(
            client.requests.len(),
            successful_pages + 2,
            "{successful_pages}"
        );
        assert_eq!(
            client
                .requests
                .iter()
                .filter(|r| **r == Request::Status)
                .count(),
            1,
            "{successful_pages}"
        );
    }
}

#[test]
fn rejects_identity_activation_revision_offset_and_total_drift() {
    for corruption in 0..5 {
        let Response::Peers { mut page } = peers(1) else {
            unreachable!()
        };
        match corruption {
            0 => page.cursor.authority_id = OTHER_PEER.parse().unwrap(),
            1 => page.cursor.activation_id = ActivationId::from_bytes([2; 16]),
            2 => page.cursor.revision = StoreRevision::new(8),
            3 => page.cursor.offset = 0,
            4 => page.total = 3,
            _ => unreachable!(),
        }
        let mut client =
            FakeClient::new([Ok(status()), Ok(peers(0)), Ok(Response::Peers { page })]);
        assert!(
            execute(Action::Pages(Pages::Peers), &mut |r| client.call(r)).is_err(),
            "{corruption}"
        );
        assert_eq!(client.requests.len(), 3, "{corruption}");
    }
}

#[test]
fn rejects_changed_or_nonprogressing_continuations_before_following_them() {
    for corruption in 0..8 {
        let Response::Peers { mut page } = peers(0) else {
            unreachable!()
        };
        let mut next = cursor(1);
        match corruption {
            0 => next.authority_id = OTHER_PEER.parse().unwrap(),
            1 => next.activation_id = ActivationId::from_bytes([2; 16]),
            2 => next.revision = StoreRevision::new(8),
            3 => next.offset = 0,
            4 => next.offset = 2,
            5 => page.items.clear(),
            6 => page.total = 0,
            7 => page.total = 1,
            _ => unreachable!(),
        }
        page.next = Some(next);
        let mut client = FakeClient::new([Ok(status()), Ok(Response::Peers { page })]);
        assert!(
            execute(Action::Pages(Pages::Peers), &mut |r| client.call(r)).is_err(),
            "{corruption}"
        );
        assert_eq!(client.requests.len(), 2, "{corruption}");
    }
}

#[test]
fn rejects_wrong_page_kind_wrong_peer_and_premature_end() {
    for response in [
        peers(0),
        Response::Grants {
            peer_id: OTHER_PEER.parse().unwrap(),
            page: Page {
                cursor: cursor(0),
                total: 0,
                items: Vec::new(),
                next: None,
            },
        },
        Response::Grants {
            peer_id: peer(),
            page: Page {
                cursor: cursor(0),
                total: 1,
                items: Vec::new(),
                next: None,
            },
        },
    ] {
        let mut client = FakeClient::new([Ok(status()), Ok(response)]);
        assert!(
            execute(Action::Pages(Pages::Grants(peer())), &mut |r| client
                .call(r))
            .is_err()
        );
        assert_eq!(client.requests.len(), 2);
    }
}

#[test]
fn empty_projection_preserves_its_single_canonical_page() {
    let response = Response::Peers {
        page: Page {
            cursor: cursor(0),
            total: 0,
            items: Vec::new(),
            next: None,
        },
    };
    let mut client = FakeClient::new([Ok(status()), Ok(response.clone())]);
    assert_eq!(
        execute(Action::Pages(Pages::Peers), &mut |r| client.call(r)).unwrap(),
        [response]
    );
    assert_eq!(client.requests.len(), 2);
}
