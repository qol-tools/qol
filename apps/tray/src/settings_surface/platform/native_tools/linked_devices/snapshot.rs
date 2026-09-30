use qol_peers::admin::{
    EnrollmentRequest, ExpectedAuthority, Page, PageCursor, PeerSummary, PointzRequest,
    PointzStatus, Request, Response, SessionCursor, SessionSummary, Status,
};
use qol_peers::enrollment::{OutboundEnrollment, PendingEnrollment};
use qol_peers::pointz::PointzDevice;
use qol_peers::PeerId;
use qol_plugin_api::operations::OperationKey;
use qol_runtime::PlatformStateClient;

use crate::features::linked_devices::settings::Failure;

pub(crate) fn request(client: &PlatformStateClient, request: Request) -> Result<Response, Failure> {
    match client.peer_admin(request)? {
        Response::Error { error } => Err(Failure::Authority(error)),
        response => Ok(response),
    }
}

pub(crate) struct Snapshot {
    pub status: Status,
    pub peers: Vec<PeerSummary>,
    pub sessions: Vec<SessionSummary>,
    pub grants: Vec<(PeerId, Vec<OperationKey>)>,
    pub pending: Vec<PendingEnrollment>,
    pub outbound: Vec<OutboundEnrollment>,
    pub attempts: Vec<(
        qol_peers::enrollment::TransactionId,
        qol_peers::admin::AttemptState,
    )>,
    pub pointz: Option<PointzStatus>,
    pub phones: Vec<PointzDevice>,
    pub nearby: Vec<qol_peers::admin::NearbyDevice>,
}

pub(crate) fn load(client: &PlatformStateClient) -> Result<Snapshot, Failure> {
    load_with(|query| request(client, query))
}

fn load_with(
    mut read: impl FnMut(Request) -> Result<Response, Failure>,
) -> Result<Snapshot, Failure> {
    let Response::Status { status } = read(Request::Status)? else {
        return Err(Failure::Inconsistent);
    };
    let mut snapshot = Snapshot {
        status,
        peers: Vec::new(),
        sessions: Vec::new(),
        grants: Vec::new(),
        pending: Vec::new(),
        outbound: Vec::new(),
        attempts: Vec::new(),
        pointz: None,
        phones: Vec::new(),
        nearby: Vec::new(),
    };
    let Some(authority) = &snapshot.status.authority else {
        return Ok(snapshot);
    };
    if snapshot.status.lifecycle != qol_peers::admin::Lifecycle::Active
        || authority.status != qol_peers::AuthorityStatus::Ready
    {
        return Ok(snapshot);
    }
    let expected = authority.expected();
    snapshot.peers = pages(expected, |cursor| match read(Request::Peers { cursor })? {
        Response::Peers { page } => Ok(page),
        _ => Err(Failure::Inconsistent),
    })?;
    for peer in &snapshot.peers {
        let grants = pages(expected, |cursor| {
            match read(Request::Grants {
                cursor,
                peer_id: peer.peer_id,
            })? {
                Response::Grants { peer_id, page } if peer_id == peer.peer_id => Ok(page),
                _ => Err(Failure::Inconsistent),
            }
        })?;
        snapshot.grants.push((peer.peer_id, grants));
    }
    let Response::PendingEnrollments { authority, items } = read(Request::Enrollment {
        request: EnrollmentRequest::Pending {},
    })?
    else {
        return Err(Failure::Inconsistent);
    };
    if authority != expected {
        return Err(Failure::Inconsistent);
    }
    snapshot.pending = items;
    snapshot.outbound = pages(expected, |cursor| {
        match read(Request::Enrollment {
            request: EnrollmentRequest::Outbound { cursor },
        })? {
            Response::OutboundEnrollments { page } => Ok(page),
            _ => Err(Failure::Inconsistent),
        }
    })?;
    for item in &snapshot.outbound {
        let transaction = item.key.transaction;
        let Response::EnrollmentAttempt {
            authority,
            transaction: returned,
            state,
        } = read(Request::Enrollment {
            request: EnrollmentRequest::Attempt {
                expected,
                transaction,
            },
        })?
        else {
            return Err(Failure::Inconsistent);
        };
        if authority != expected || returned != transaction {
            return Err(Failure::Inconsistent);
        }
        snapshot.attempts.push((transaction, state));
    }
    snapshot.sessions = sessions(&mut read, expected)?;
    let Response::Nearby { authority, devices } = read(Request::Nearby {
        request: qol_peers::admin::NearbyRequest::List {},
    })?
    else {
        return Err(Failure::Inconsistent);
    };
    if authority != expected {
        return Err(Failure::Inconsistent);
    }
    snapshot.nearby = devices;
    let Response::PointzStatus { status: pointz } = read(Request::Pointz {
        request: PointzRequest::Status {},
    })?
    else {
        return Err(Failure::Inconsistent);
    };
    if pointz.authority != Some(expected) {
        return Err(Failure::Inconsistent);
    }
    if pointz.migration.is_some() {
        snapshot.phones = pages(expected, |cursor| {
            match read(Request::Pointz {
                request: PointzRequest::Devices { cursor },
            })? {
                Response::PointzDevices { page } => Ok(page),
                _ => Err(Failure::Inconsistent),
            }
        })?;
    }
    snapshot.pointz = Some(pointz);
    let Response::Status { status } = read(Request::Status)? else {
        return Err(Failure::Inconsistent);
    };
    if status != snapshot.status {
        return Err(Failure::Inconsistent);
    }
    Ok(snapshot)
}

fn pages<T>(
    expected: ExpectedAuthority,
    mut read: impl FnMut(PageCursor) -> Result<Page<T>, Failure>,
) -> Result<Vec<T>, Failure> {
    let mut cursor = PageCursor {
        authority_id: expected.authority_id,
        activation_id: expected.activation_id,
        revision: expected.revision,
        offset: 0,
    };
    let mut items = Vec::new();
    let mut total = None;
    loop {
        let page = read(cursor)?;
        if page.cursor != cursor || total.is_some_and(|total| total != page.total) {
            return Err(Failure::Inconsistent);
        }
        total = Some(page.total);
        items.extend(page.items);
        let Some(next) = page.next else {
            if items.len() != page.total as usize {
                return Err(Failure::Inconsistent);
            }
            return Ok(items);
        };
        let wanted = PageCursor {
            offset: items.len() as u32,
            ..cursor
        };
        if next != wanted || next.offset <= cursor.offset || next.offset >= page.total {
            return Err(Failure::Inconsistent);
        }
        cursor = next;
    }
}

fn sessions(
    read: &mut impl FnMut(Request) -> Result<Response, Failure>,
    expected: ExpectedAuthority,
) -> Result<Vec<SessionSummary>, Failure> {
    let Response::Network { network } = read(Request::Network)? else {
        return Err(Failure::Inconsistent);
    };
    if network.authority != expected {
        return Err(Failure::Inconsistent);
    }
    let mut cursor = SessionCursor {
        authority_id: expected.authority_id,
        activation_id: expected.activation_id,
        revision: expected.revision,
        network_revision: network.network_revision,
        offset: 0,
    };
    let mut items = Vec::new();
    let mut total = None;
    loop {
        let Response::Sessions { page } = read(Request::Sessions { cursor })? else {
            return Err(Failure::Inconsistent);
        };
        if page.cursor != cursor || total.is_some_and(|total| total != page.total) {
            return Err(Failure::Inconsistent);
        }
        total = Some(page.total);
        items.extend(page.items);
        let Some(next) = page.next else {
            if items.len() != page.total as usize {
                return Err(Failure::Inconsistent);
            }
            break;
        };
        if next
            != (SessionCursor {
                offset: items.len() as u32,
                ..cursor
            })
            || next.offset <= cursor.offset
            || next.offset >= page.total
        {
            return Err(Failure::Inconsistent);
        }
        cursor = next;
    }
    let Response::Network { network: current } = read(Request::Network)? else {
        return Err(Failure::Inconsistent);
    };
    if current.authority != expected || current.network_revision != network.network_revision {
        return Err(Failure::Inconsistent);
    }
    Ok(items)
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
