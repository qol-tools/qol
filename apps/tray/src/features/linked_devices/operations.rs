use super::{Host, PeerHostHandle};
use crate::plugins::{action_executor::remote::PreparedRemote, PluginManager};
use qol_peers::{
    operations::{Failure, Invocation, Outcome, Request, RequestStatus, Response},
    service::{network::OperationDispatcher, PeerAuthority},
    session::AuthenticatedSession,
};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub(super) struct Dispatcher {
    pub host: Weak<Mutex<Host>>,
    pub plugins: Weak<Mutex<PluginManager>>,
}

impl OperationDispatcher for Dispatcher {
    fn invoke(
        &self,
        authority: &PeerAuthority,
        session: AuthenticatedSession,
        invocation: &Invocation,
    ) -> Result<Outcome, Failure> {
        let body = qol_peers::service::decode_body(&invocation.body)?;
        let host = self.host.upgrade().ok_or(Failure::Unavailable)?;
        let plugins = self.plugins.upgrade().ok_or(Failure::Unavailable)?;
        let captured = {
            let host = host.lock().map_err(|_| Failure::Unavailable)?;
            check_host(&host, session)?;
            let manager = plugins.lock().map_err(|_| Failure::Unavailable)?;
            PreparedRemote::capture(&manager)?
        };
        let prepared = PreparedRemote::prepare(captured, &body)?;
        {
            let host = host.lock().map_err(|_| Failure::Unavailable)?;
            check_host(&host, session)?;
            let manager = plugins.lock().map_err(|_| Failure::Unavailable)?;
            prepared.recheck(&manager)?;
            let (outcome, owned) = authority.admit_operation(
                session,
                invocation,
                prepared.declaration().to_string(),
            )?;
            if !owned {
                return Ok(outcome);
            }
        }
        let result = self.start(authority, session, invocation, &prepared, &host, &plugins);
        let outcome = match result {
            Ok(deadline) => {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    Outcome::Refused {
                        reason: Failure::Deadline,
                    }
                } else {
                    prepared.execute(remaining)
                }
            }
            Err(reason) => Outcome::Refused { reason },
        };
        let outcome =
            authority.settle_operation(session.remote_peer, &invocation.handle, outcome)?;
        qol_runtime::probe!(
            "TRAY_PEERS",
            "event=operation_settled peer={} sequence={} terminal=true",
            session.remote_peer,
            invocation.handle.sequence
        );
        Ok(outcome)
    }
}

impl Dispatcher {
    fn start(
        &self,
        authority: &PeerAuthority,
        session: AuthenticatedSession,
        invocation: &Invocation,
        prepared: &PreparedRemote,
        host: &Arc<Mutex<Host>>,
        plugins: &Arc<Mutex<PluginManager>>,
    ) -> Result<tokio::time::Instant, Failure> {
        let captured = {
            let host = host.lock().map_err(|_| Failure::Unavailable)?;
            check_host(&host, session)?;
            let manager = plugins.lock().map_err(|_| Failure::Unavailable)?;
            prepared.recheck(&manager)?;
            PreparedRemote::capture(&manager)?
        };
        prepared.probe(Duration::from_millis(500))?;
        let refreshed = prepared.refresh(captured)?;
        let host = host.lock().map_err(|_| Failure::Unavailable)?;
        check_host(&host, session)?;
        let manager = plugins.lock().map_err(|_| Failure::Unavailable)?;
        prepared.recheck(&manager)?;
        refreshed.recheck(&manager)?;
        let deadline = authority.start_operation(session, invocation, prepared.declaration())?;
        qol_runtime::probe!(
            "TRAY_PEERS",
            "event=operation_dispatch_started peer={} sequence={}",
            session.remote_peer,
            invocation.handle.sequence
        );
        Ok(deadline)
    }
}

fn check_host(host: &Host, session: AuthenticatedSession) -> Result<(), Failure> {
    let active = host.authority().map_err(|_| Failure::Unavailable)?;
    if active
        .authority
        .local_pin()
        .map_err(|_| Failure::Unavailable)?
        .peer_id()
        != session.local_peer
    {
        return Err(Failure::StaleAuthority);
    }
    Ok(())
}

impl PeerHostHandle {
    pub fn operation_request(&self, request: Request) -> Response {
        self.operation_request_inner(request)
            .unwrap_or_else(|error| Response::Error { error })
    }

    fn operation_request_inner(&self, request: Request) -> Result<Response, Failure> {
        let (handle, incoming) = {
            let host = self.inner.lock().map_err(|_| Failure::Unavailable)?;
            let active = host.authority().map_err(|_| Failure::Unavailable)?;
            let expected = match &request {
                Request::Invoke { expected, .. }
                | Request::Outcome { expected, .. }
                | Request::Cancel { expected, .. }
                | Request::Requests { expected, .. } => *expected,
            };
            let authority = active
                .check_expected(expected)
                .map_err(|_| Failure::StaleAuthority)?;
            if let Request::Requests { peer, .. } = request {
                return Ok(Response::Requests {
                    requests: authority.operation_requests(peer)?,
                });
            }
            let network = active.network.as_ref().ok_or(Failure::Unavailable)?;
            match request {
                Request::Invoke { peer, body, .. } => {
                    let invocation = authority.prepare_operation(peer, body)?;
                    let handle = invocation.handle.clone();
                    (handle, network.invoke_operation(invocation))
                }
                Request::Outcome { handle, .. } => {
                    authority.check_operation_handle(&handle)?;
                    (handle.clone(), network.reconcile_operation(handle, false))
                }
                Request::Cancel { handle, .. } => {
                    authority.check_operation_handle(&handle)?;
                    (handle.clone(), network.reconcile_operation(handle, true))
                }
                Request::Requests { .. } => unreachable!(),
            }
        };
        let Ok(incoming) = incoming else {
            return Ok(Response::Unknown {
                handle: Some(handle),
            });
        };
        match incoming.recv_timeout(Duration::from_secs(11)) {
            Ok(Ok(outcome)) => Ok(Response::Status {
                status: RequestStatus { handle, outcome },
            }),
            Ok(Err(error)) => Ok(Response::Error { error }),
            Err(_) => Ok(Response::Unknown {
                handle: Some(handle),
            }),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
impl PeerHostHandle {
    pub(crate) fn change_operation_selection_for_test(
        &self,
        change: impl FnOnce(&mut PluginManager),
    ) {
        let _host = self.inner.lock().unwrap();
        let mut manager = self.plugins.lock().unwrap();
        change(&mut manager);
    }
}
