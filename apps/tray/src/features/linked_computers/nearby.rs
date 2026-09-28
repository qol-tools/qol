use qol_peers::admin::{Error, NearbyComputer, NearbyLink, NearbyRequest, NearbyState, Response};
use qol_peers::PeerId;

use super::enrollment::{changed, network, stamp};
use super::ActiveAuthority;

pub(super) fn dispatch(
    active: &ActiveAuthority,
    request: NearbyRequest,
) -> Result<Response, Error> {
    match request {
        NearbyRequest::List {} => list(active),
        NearbyRequest::Link {
            expected,
            peer_id,
            grants,
        } => {
            let authority = active.check_expected(expected)?;
            if inbound(active, peer_id)? {
                authority.confirm_nearby(expected.revision, peer_id, grants)?;
            } else {
                network(active)?.link_nearby(authority, peer_id, grants)?;
            }
            list(active)
        }
        NearbyRequest::Confirm {
            expected,
            peer_id,
            grants,
        } => {
            let authority = active.check_expected(expected)?;
            if inbound(active, peer_id)? {
                authority.confirm_nearby(expected.revision, peer_id, grants)?;
            } else {
                network(active)?.confirm_nearby(authority, expected.revision, peer_id, grants)?;
            }
            changed(active)
        }
        NearbyRequest::Decline { expected, peer_id } => {
            let authority = active.check_expected(expected)?;
            if inbound(active, peer_id)? {
                authority.decline_nearby(expected.revision, peer_id)?;
            }
            if let Some(transaction) = active
                .network
                .as_ref()
                .and_then(|network| network.decline_nearby(peer_id))
            {
                authority.abandon_join(authority.projection()?.revision, transaction)?;
            }
            changed(active)
        }
    }
}

fn inbound(active: &ActiveAuthority, peer: PeerId) -> Result<bool, Error> {
    Ok(active
        .authority
        .inbound_nearby()?
        .iter()
        .any(|request| request.peer_id == peer)
        && !joins(active, peer)?)
}

fn joins(active: &ActiveAuthority, peer: PeerId) -> Result<bool, Error> {
    let Some(network) = &active.network else {
        return Ok(false);
    };
    if active.authority.projection()?.peer_id >= peer {
        return Ok(false);
    }
    Ok(network
        .nearby_links()?
        .iter()
        .any(|(id, _, link)| *id == peer && !matches!(link.state, NearbyState::Failed { .. })))
}

fn list(active: &ActiveAuthority) -> Result<Response, Error> {
    let linked: Vec<PeerId> = active
        .authority
        .projection()?
        .peers
        .iter()
        .map(|peer| peer.peer_id)
        .collect();
    let mut computers: Vec<NearbyComputer> = Vec::new();
    if let Some(network) = &active.network {
        for claim in network.snapshot().nearby {
            upsert(&mut computers, claim.peer_id, claim.name, None);
        }
        for (peer, name, link) in network.nearby_links()? {
            upsert(&mut computers, peer, name, Some(link));
        }
    }
    for request in active.authority.inbound_nearby()? {
        if joins(active, request.peer_id)? {
            continue;
        }
        let state = if request.confirmed && !request.redeemed {
            NearbyState::WaitingForPeer {}
        } else {
            NearbyState::Confirm {}
        };
        let link = NearbyLink {
            code: Some(request.code),
            state,
        };
        upsert(&mut computers, request.peer_id, request.name, Some(link));
    }
    computers.retain(|computer| !linked.contains(&computer.peer_id));
    computers.sort_by(|left, right| {
        (left.name.to_lowercase(), left.peer_id.to_string())
            .cmp(&(right.name.to_lowercase(), right.peer_id.to_string()))
    });
    Ok(Response::Nearby {
        authority: stamp(active)?,
        computers,
    })
}

fn upsert(
    computers: &mut Vec<NearbyComputer>,
    peer_id: PeerId,
    name: String,
    link: Option<NearbyLink>,
) {
    match computers
        .iter_mut()
        .find(|computer| computer.peer_id == peer_id)
    {
        Some(computer) => {
            if link.is_some() {
                computer.link = link;
            }
        }
        None => computers.push(NearbyComputer {
            peer_id,
            name,
            link,
        }),
    }
}
