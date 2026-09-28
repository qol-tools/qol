use std::time::SystemTime;

use qol_peers::admin::{Error, ExpectedAuthority, Request, Response};
use qol_peers::service::PeerAuthority;
use qol_peers::{AuthorityError, PeerId, StoreRevision};
use qol_plugin_api::operations::{OperationIdentity, OperationKey};

use super::{projection, ActiveAuthority, Host, PeerHostHandle, State};

impl PeerHostHandle {
    pub(super) fn dispatch(&self, mut request: Request) -> Result<Response, Error> {
        super::enrollment::preflight_addresses(
            &mut request,
            qol_peers::service::network::local_addresses::current,
        )?;
        let catalog = if matches!(&request, Request::SetGrants { grants, .. } if !grants.is_empty())
        {
            let capture = {
                let manager = self.plugins.lock().map_err(|_| Error::HostUnavailable)?;
                crate::plugins::operation_catalog::CatalogCapture::capture(&manager)
            };
            Some(capture.resolve())
        } else {
            None
        };
        let mut host = self.inner.lock().map_err(|_| Error::HostUnavailable)?;
        match request {
            Request::Enrollment { request } => {
                super::enrollment::dispatch(host.authority()?, request)
            }
            Request::Status => Ok(Response::Status {
                status: host.status()?,
            }),
            Request::Network => projection::network(&host),
            Request::Sessions { cursor } => projection::sessions(&host, cursor),
            Request::Peers { cursor } => {
                let active = host.authority()?;
                projection::peers(active.authority.projection()?, active.activation_id, cursor)
            }
            Request::Grants { peer_id, cursor } => {
                let active = host.authority()?;
                projection::grants(
                    active.authority.projection()?,
                    active.activation_id,
                    peer_id,
                    cursor,
                )
            }
            Request::Tombstones { cursor } => {
                let active = host.authority()?;
                projection::tombstones(active.authority.projection()?, active.activation_id, cursor)
            }
            Request::StartSession { name } => {
                host.start(|_| PeerAuthority::session(name, SystemTime::now()).map_err(Error::from))
            }
            Request::CreatePersistent { name } => host.start(|host| {
                let root = host.root.as_ref().map_err(|error| *error)?;
                PeerAuthority::create_persistent(root, name, SystemTime::now()).map_err(Error::from)
            }),
            Request::OpenPersistent => host.start(|host| {
                let root = host.root.as_ref().map_err(|error| *error)?;
                PeerAuthority::open_persistent(root, SystemTime::now()).map_err(Error::from)
            }),
            Request::Stop { expected } => host.stop(expected),
            Request::Rename { expected, name } => {
                let active = host.authority()?;
                let revision = active
                    .check_expected(expected)?
                    .rename(expected.revision, name)?;
                changed(active, revision)
            }
            Request::SetGrants {
                expected,
                peer_id,
                grants,
            } => {
                let active = host.authority()?;
                let authority = active.check_expected(expected)?;
                let revision = self.replace_grants(
                    authority,
                    expected.revision,
                    peer_id,
                    grants,
                    catalog.as_ref(),
                )?;
                changed(active, revision)
            }
            Request::Revoke { expected, peer_id } => {
                let active = host.authority()?;
                let revision = active
                    .check_expected(expected)?
                    .revoke(expected.revision, peer_id)?;
                changed(active, revision)
            }
        }
    }

    fn replace_grants(
        &self,
        authority: &PeerAuthority,
        expected: StoreRevision,
        peer_id: PeerId,
        grants: Vec<OperationKey>,
        catalog: Option<&crate::plugins::operation_catalog::ResolvedCatalog>,
    ) -> Result<StoreRevision, Error> {
        if grants.is_empty() {
            return authority
                .set_grants(expected, peer_id, grants)
                .map_err(Error::from);
        }
        let existing = authority
            .projection()?
            .peers
            .into_iter()
            .find(|peer| peer.peer_id == peer_id)
            .map(|peer| peer.grants)
            .unwrap_or_default();
        let manager = self.plugins.lock().map_err(|_| Error::HostUnavailable)?;
        for grant in &grants {
            if existing.contains(grant) {
                continue;
            }
            if !matches!(grant.identity, OperationIdentity::Stable(_))
                || !catalog
                    .and_then(|catalog| catalog.select(grant))
                    .is_some_and(|selected| {
                        selected.operation.peer.is_some() && selected.matches_manager(&manager)
                    })
            {
                return Err(Error::GrantUnavailable);
            }
        }
        authority
            .set_grants(expected, peer_id, grants)
            .map_err(Error::from)
    }
}

impl Host {
    fn start(
        &mut self,
        create: impl FnOnce(&Host) -> Result<PeerAuthority, Error>,
    ) -> Result<Response, Error> {
        match self.state {
            State::Active(_) => return Err(Error::AlreadyActive),
            State::Standby => return Err(Error::Standby),
            State::Stopping(_) => return Err(Error::Stopping),
            State::Shutdown => return Err(Error::Shutdown),
            State::Inactive | State::Unavailable(_) => {}
        }
        let result = (self.activation)().and_then(|activation_id| {
            create(self).map(|authority| ActiveAuthority::new(authority, activation_id))
        });
        match result {
            Ok(active) => self.activate(active),
            Err(error) => {
                self.state = State::Unavailable(error);
                return Err(error);
            }
        }
        Ok(Response::Status {
            status: self.status()?,
        })
    }

    fn stop(&mut self, expected: ExpectedAuthority) -> Result<Response, Error> {
        let current = self
            .authority()?
            .check_expected(expected)?
            .projection()?
            .revision;
        if expected.revision != current {
            return Err(AuthorityError::StaleRevision {
                expected: expected.revision,
                current,
            }
            .into());
        }
        self.begin_stop();
        Ok(Response::Status {
            status: self.status()?,
        })
    }
}

fn changed(active: &ActiveAuthority, revision: StoreRevision) -> Result<Response, Error> {
    Ok(Response::Changed {
        authority_id: active.authority.projection()?.peer_id,
        activation_id: active.activation_id,
        revision,
    })
}

pub(super) fn operation_name(request: &Request) -> &'static str {
    match request {
        Request::Enrollment { request } => request.action_name(),
        Request::Status => "status",
        Request::Network => "network",
        Request::Sessions { .. } => "sessions",
        Request::Peers { .. } => "peers",
        Request::Grants { .. } => "grants",
        Request::Tombstones { .. } => "tombstones",
        Request::StartSession { .. } => "start_session",
        Request::CreatePersistent { .. } => "create_persistent",
        Request::OpenPersistent => "open_persistent",
        Request::Stop { .. } => "stop",
        Request::Rename { .. } => "rename",
        Request::SetGrants { .. } => "set_grants",
        Request::Revoke { .. } => "revoke",
    }
}

pub(super) fn trace_outcome(operation: &str, result: &Result<Response, Error>) {
    match result {
        Err(error) => qol_runtime::probe!(
            "TRAY_PEERS",
            "event=admin operation={} error={:?}",
            operation,
            error
        ),
        Ok(Response::Changed {
            authority_id,
            revision,
            ..
        }) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=admin operation={} authority={} revision={}",
                operation,
                authority_id,
                revision
            );
        }
        Ok(Response::Status { status }) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=admin operation={} lifecycle={:?}",
                operation,
                status.lifecycle
            );
            if let Some(authority) = &status.authority {
                qol_runtime::probe!(
                    "TRAY_PEERS",
                    "event=authority authority={} revision={} status={:?}",
                    authority.peer_id,
                    authority.revision,
                    authority.status
                );
            }
        }
        Ok(Response::Network { network }) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=network authority={} revision={} listener={:?} enrollment_listener={:?} discovery={:?} stopping={} failure={:?}",
                network.authority.authority_id,
                network.network_revision.0,
                network.status.listener,
                network.status.enrollment_listener,
                network.status.discovery,
                network.status.stopping,
                network.status.failure
            );
        }
        Ok(Response::JoinPrepared {
            authority,
            transaction,
        }) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=enrollment operation={} activation={} transaction={} outcome=prepared",
                operation,
                authority.activation_id,
                transaction
            );
        }
        Ok(Response::EnrollmentAttempt {
            authority,
            transaction,
            state,
        }) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=enrollment operation={} activation={} transaction={} outcome={}",
                operation,
                authority.activation_id,
                transaction,
                super::enrollment::attempt_name(state)
            );
        }
        Ok(
            Response::Invitation { .. }
            | Response::PendingEnrollments { .. }
            | Response::OutboundEnrollments { .. }
            | Response::Sessions { .. }
            | Response::Peers { .. }
            | Response::Grants { .. }
            | Response::Tombstones { .. }
            | Response::Error { .. },
        ) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=admin operation={} outcome=reply",
                operation
            );
        }
    }
}
