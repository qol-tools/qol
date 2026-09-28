use qol_peers::admin::{
    ActivationId, AuthoritySummary, Error, Page, PageCursor, PeerSummary, Response,
    GRANTS_PER_PAGE, PEERS_PER_PAGE, TOMBSTONES_PER_PAGE,
};
use qol_peers::{AuthorityError, AuthorityProjection, PeerId};

pub(super) fn summary(
    projection: AuthorityProjection,
    activation_id: ActivationId,
) -> AuthoritySummary {
    AuthoritySummary {
        activation_id,
        peer_id: projection.peer_id,
        name: projection.name,
        lifetime: projection.lifetime,
        revision: projection.revision,
        status: projection.status,
        peer_count: projection.peers.len() as u32,
        grant_count: projection
            .peers
            .iter()
            .map(|peer| peer.grants.len() as u32)
            .sum(),
        tombstone_count: projection.tombstones.len() as u32,
    }
}

pub(super) fn peers(
    projection: AuthorityProjection,
    activation_id: ActivationId,
    cursor: PageCursor,
) -> Result<Response, Error> {
    validate_cursor(&projection, activation_id, cursor)?;
    let peers: Vec<_> = projection
        .peers
        .into_iter()
        .map(|peer| PeerSummary {
            peer_id: peer.peer_id,
            name: peer.name,
            grant_count: peer.grants.len() as u32,
        })
        .collect();
    Ok(Response::Peers {
        page: page(peers, cursor, PEERS_PER_PAGE)?,
    })
}

pub(super) fn grants(
    projection: AuthorityProjection,
    activation_id: ActivationId,
    peer_id: PeerId,
    cursor: PageCursor,
) -> Result<Response, Error> {
    validate_cursor(&projection, activation_id, cursor)?;
    if projection.tombstones.contains(&peer_id) {
        return Err(AuthorityError::Revoked.into());
    }
    let peer = projection
        .peers
        .into_iter()
        .find(|peer| peer.peer_id == peer_id)
        .ok_or(AuthorityError::UnknownPeer)?;
    Ok(Response::Grants {
        peer_id,
        page: page(peer.grants, cursor, GRANTS_PER_PAGE)?,
    })
}

pub(super) fn tombstones(
    projection: AuthorityProjection,
    activation_id: ActivationId,
    cursor: PageCursor,
) -> Result<Response, Error> {
    validate_cursor(&projection, activation_id, cursor)?;
    Ok(Response::Tombstones {
        page: page(projection.tombstones, cursor, TOMBSTONES_PER_PAGE)?,
    })
}

pub(super) fn pointz_devices(
    projection: AuthorityProjection,
    activation_id: ActivationId,
    devices: Vec<qol_peers::pointz::PointzDevice>,
    cursor: PageCursor,
) -> Result<Response, Error> {
    validate_cursor(&projection, activation_id, cursor)?;
    Ok(Response::PointzDevices {
        page: page(devices, cursor, PEERS_PER_PAGE)?,
    })
}

fn validate_cursor(
    projection: &AuthorityProjection,
    activation_id: ActivationId,
    cursor: PageCursor,
) -> Result<(), Error> {
    if projection.peer_id != cursor.authority_id
        || activation_id != cursor.activation_id
        || projection.revision != cursor.revision
    {
        return Err(Error::StaleCursor);
    }
    Ok(())
}

fn page<T>(items: Vec<T>, cursor: PageCursor, limit: usize) -> Result<Page<T>, Error> {
    let total = items.len();
    let start = cursor.offset as usize;
    if start > total {
        return Err(Error::InvalidCursor);
    }
    let end = start.saturating_add(limit).min(total);
    let next = (end < total).then_some(PageCursor {
        offset: end as u32,
        ..cursor
    });
    Ok(Page {
        cursor,
        total: total as u32,
        items: items.into_iter().skip(start).take(limit).collect(),
        next,
    })
}

pub(super) fn network(host: &super::Host) -> Result<Response, Error> {
    let active = network_authority(host)?;
    let authority = summary(active.authority.projection()?, active.activation_id).expected();
    let mut snapshot = network_snapshot(active);
    snapshot.status.stopping = matches!(host.state, super::State::Stopping(_));
    Ok(Response::Network {
        network: qol_peers::admin::NetworkSummary {
            authority,
            network_revision: snapshot.revision,
            status: snapshot.status,
        },
    })
}

pub(super) fn sessions(
    host: &super::Host,
    cursor: qol_peers::admin::SessionCursor,
) -> Result<Response, Error> {
    let active = network_authority(host)?;
    let authority = active.authority.projection()?;
    session_page(
        authority,
        active.activation_id,
        network_snapshot(active),
        cursor,
    )
}

pub(super) fn session_page(
    authority: AuthorityProjection,
    activation_id: ActivationId,
    snapshot: qol_peers::service::network::NetworkSnapshot,
    cursor: qol_peers::admin::SessionCursor,
) -> Result<Response, Error> {
    validate_cursor(&authority, activation_id, cursor.authority_cursor())?;
    if snapshot.revision != cursor.network_revision {
        return Err(Error::StaleCursor);
    }
    let items: Vec<_> = snapshot
        .sessions
        .into_iter()
        .filter_map(|session| {
            let peer = authority
                .peers
                .iter()
                .find(|peer| peer.peer_id == session.remote_peer)?;
            Some(qol_peers::admin::SessionSummary {
                peer_id: peer.peer_id,
                name: peer.name.clone(),
                generation: session.generation,
            })
        })
        .collect();
    let total = items.len();
    let start = cursor.offset as usize;
    if start > total {
        return Err(Error::InvalidCursor);
    }
    let end = start.saturating_add(PEERS_PER_PAGE).min(total);
    Ok(Response::Sessions {
        page: qol_peers::admin::SessionPage {
            cursor,
            total: total as u32,
            items: items.into_iter().skip(start).take(PEERS_PER_PAGE).collect(),
            next: (end < total).then_some(qol_peers::admin::SessionCursor {
                offset: end as u32,
                ..cursor
            }),
        },
    })
}

fn network_authority(host: &super::Host) -> Result<&super::ActiveAuthority, Error> {
    match &host.state {
        super::State::Stopping(active) => Ok(active),
        _ => host.authority(),
    }
}

fn network_snapshot(
    active: &super::ActiveAuthority,
) -> qol_peers::service::network::NetworkSnapshot {
    use qol_peers::network::{DiscoveryStatus, ListenerStatus, NetworkFailure, NetworkStatus};
    let mut snapshot = active
        .network
        .as_ref()
        .map(|network| network.snapshot())
        .unwrap_or_else(|| qol_peers::service::network::NetworkSnapshot {
            revision: qol_peers::admin::NetworkRevision::default(),
            sessions: Vec::new(),
            nearby: Vec::new(),
            status: NetworkStatus {
                listener: ListenerStatus::Closed {},
                enrollment_listener: ListenerStatus::Closed {},
                discovery: DiscoveryStatus::Closed {},
                failure: Some(NetworkFailure::RuntimeUnavailable),
                ..NetworkStatus::default()
            },
        });
    snapshot.status.failure = active.network_failure.or(snapshot.status.failure);
    snapshot
}
