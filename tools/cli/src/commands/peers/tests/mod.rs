mod dispatch;
mod input;
mod pagination;

use std::collections::VecDeque;

use anyhow::Result;
use qol_peers::admin::{
    ActivationId, AuthoritySummary, Lifecycle, PageCursor, Request, Response, Status,
};
use qol_peers::{AuthorityLifetime, AuthorityStatus, PeerId, StoreRevision};

use super::execute;

const PEER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const OTHER_PEER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";

fn peer() -> PeerId {
    PEER.parse().unwrap()
}

fn authority() -> AuthoritySummary {
    AuthoritySummary {
        peer_id: peer(),
        activation_id: ActivationId::from_bytes([1; 16]),
        name: "fixture".into(),
        lifetime: AuthorityLifetime::Session,
        revision: StoreRevision::new(7),
        status: AuthorityStatus::Ready,
        peer_count: 2,
        grant_count: 2,
        tombstone_count: 2,
    }
}

fn status() -> Response {
    Response::Status {
        status: Status {
            lifecycle: Lifecycle::Active,
            authority: Some(authority()),
        },
    }
}

fn changed() -> Response {
    let authority = authority();
    Response::Changed {
        authority_id: authority.peer_id,
        activation_id: authority.activation_id,
        revision: StoreRevision::new(8),
    }
}

fn cursor(offset: u32) -> PageCursor {
    let authority = authority();
    PageCursor {
        authority_id: authority.peer_id,
        activation_id: authority.activation_id,
        revision: authority.revision,
        offset,
    }
}

struct FakeClient {
    replies: VecDeque<Result<Response>>,
    requests: Vec<Request>,
}

impl FakeClient {
    fn new(replies: impl IntoIterator<Item = Result<Response>>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            requests: Vec::new(),
        }
    }

    fn call(&mut self, request: Request) -> Result<Response> {
        self.requests.push(request);
        self.replies
            .pop_front()
            .expect("unexpected additional dispatch")
    }
}
