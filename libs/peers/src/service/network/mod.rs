pub mod discovery;
mod enrollment;
pub mod local_addresses;
mod nearby;
pub(crate) mod operations;
mod routing;
mod supervisor;
pub use nearby::NearbyClaim;
pub use operations::OperationDispatcher;

#[cfg(test)]
mod tests;

use std::{
    future::Future,
    net::Ipv4Addr,
    pin::Pin,
    sync::{Arc, Weak},
};

use tokio::sync::watch;

use super::PeerAuthority;
use crate::{
    admin::NetworkRevision,
    network::{NetworkFailure, NetworkStatus},
    session::AuthenticatedSession,
};
use discovery::{DiscoveryFactory, MdnsDiscovery};

pub type NetworkTask = Pin<Box<dyn Future<Output = Result<(), NetworkFailure>> + Send>>;

#[derive(Clone)]
pub struct NetworkOptions {
    pub bind: Ipv4Addr,
    pub allow_loopback_hints: bool,
    pub discovery: Arc<dyn DiscoveryFactory>,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        Self {
            bind: Ipv4Addr::UNSPECIFIED,
            allow_loopback_hints: false,
            discovery: Arc::new(MdnsDiscovery),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NetworkSnapshot {
    pub revision: NetworkRevision,
    pub status: NetworkStatus,
    pub sessions: Vec<AuthenticatedSession>,
    pub nearby: Vec<NearbyClaim>,
}

pub struct NetworkControl {
    stop: watch::Sender<bool>,
    enrollment: enrollment::Control,
    view: watch::Receiver<NetworkSnapshot>,
    operations: Weak<operations::Hub>,
}

impl NetworkControl {
    pub fn operations_ready(&self, peer: crate::PeerId) -> bool {
        self.operations.upgrade().is_some_and(|hub| hub.ready(peer))
    }

    pub fn snapshot(&self) -> NetworkSnapshot {
        self.view.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<NetworkSnapshot> {
        self.view.clone()
    }

    pub fn invitation_endpoints(
        &self,
        addresses: Vec<std::net::IpAddr>,
    ) -> Result<Vec<std::net::SocketAddr>, crate::admin::Error> {
        use crate::{
            admin::{EnrollmentFailure, Error},
            network::ListenerStatus,
        };
        if *self.stop.borrow() {
            return Err(Error::Enrollment {
                error: EnrollmentFailure::Unavailable,
            });
        }
        let ListenerStatus::Listening { port } = self.view.borrow().status.enrollment_listener
        else {
            return Err(Error::Enrollment {
                error: EnrollmentFailure::Unavailable,
            });
        };
        let endpoints: Vec<_> = addresses
            .into_iter()
            .map(|address| std::net::SocketAddr::new(address, port))
            .collect();
        enrollment::validate_endpoints(&endpoints, self.enrollment.loopback)?;
        Ok(endpoints)
    }

    pub fn validate_invitation(
        &self,
        invitation: &super::enrollment::Invitation,
    ) -> Result<(), crate::admin::Error> {
        enrollment::validate_endpoints(invitation.endpoints(), self.enrollment.loopback)
    }

    pub fn admit_enrollment(
        &self,
        authority: &PeerAuthority,
        expected: crate::StoreRevision,
        transaction: crate::enrollment::TransactionId,
        document: Option<crate::enrollment::ExportedInvitation>,
        endpoints: Vec<std::net::SocketAddr>,
    ) -> Result<crate::admin::AttemptState, crate::admin::Error> {
        if *self.stop.borrow() {
            return Err(crate::admin::Error::Enrollment {
                error: crate::admin::EnrollmentFailure::Unavailable,
            });
        }
        self.enrollment.admit(
            authority,
            expected,
            transaction,
            document,
            endpoints,
            Vec::new(),
        )
    }

    pub fn link_nearby(
        &self,
        authority: &PeerAuthority,
        peer: crate::PeerId,
        grants: Vec<qol_conventions::operations::OperationKey>,
    ) -> Result<(), crate::admin::Error> {
        self.running()?;
        let claim = self
            .view
            .borrow()
            .nearby
            .iter()
            .find(|claim| claim.peer_id == peer)
            .cloned()
            .ok_or(crate::admin::Error::Enrollment {
                error: crate::admin::EnrollmentFailure::InvalidEndpoint,
            })?;
        self.enrollment
            .link_nearby(authority, peer, claim.name, claim.endpoints, grants)
    }

    pub fn confirm_nearby(
        &self,
        authority: &PeerAuthority,
        expected: crate::StoreRevision,
        peer: crate::PeerId,
        grants: Vec<qol_conventions::operations::OperationKey>,
    ) -> Result<crate::admin::AttemptState, crate::admin::Error> {
        self.running()?;
        self.enrollment
            .confirm_nearby(authority, expected, peer, grants)
    }

    pub fn decline_nearby(&self, peer: crate::PeerId) -> Option<crate::enrollment::TransactionId> {
        self.enrollment.decline_nearby(peer)
    }

    pub fn nearby_links(
        &self,
    ) -> Result<Vec<(crate::PeerId, String, crate::admin::NearbyLink)>, crate::admin::Error> {
        self.enrollment.nearby_links()
    }

    fn running(&self) -> Result<(), crate::admin::Error> {
        if *self.stop.borrow() {
            return Err(crate::admin::Error::Enrollment {
                error: crate::admin::EnrollmentFailure::Unavailable,
            });
        }
        Ok(())
    }

    pub fn enrollment_attempt(
        &self,
        transaction: crate::enrollment::TransactionId,
    ) -> Result<crate::admin::AttemptState, crate::admin::Error> {
        self.enrollment.attempt(transaction)
    }

    pub fn cancel_enrollment(&self, transaction: crate::enrollment::TransactionId) {
        self.enrollment.cancel(transaction);
    }

    pub fn stop(&self) {
        if let Some(operations) = self.operations.upgrade() {
            operations.stop();
        }
        self.stop.send_replace(true);
    }
}

impl Drop for NetworkControl {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn prepare(authority: PeerAuthority, options: NetworkOptions) -> (NetworkControl, NetworkTask) {
    prepare_inner(authority, options, None)
}

pub fn prepare_with_operations(
    authority: PeerAuthority,
    options: NetworkOptions,
    dispatcher: Arc<dyn OperationDispatcher>,
) -> (NetworkControl, NetworkTask) {
    prepare_inner(authority, options, Some(dispatcher))
}

fn prepare_inner(
    authority: PeerAuthority,
    options: NetworkOptions,
    dispatcher: Option<Arc<dyn OperationDispatcher>>,
) -> (NetworkControl, NetworkTask) {
    let (operations, workers) = operations::Hub::new(authority.clone(), dispatcher);
    let (stop, stopping) = watch::channel(false);
    let (view, snapshot) = watch::channel(NetworkSnapshot {
        revision: NetworkRevision::default(),
        status: NetworkStatus::default(),
        sessions: Vec::new(),
        nearby: Vec::new(),
    });
    let (enrollment, owner) = enrollment::prepare(options.allow_loopback_hints);
    let task = Box::pin(run(
        authority,
        options,
        stopping,
        view,
        owner,
        operations.clone(),
        workers,
    ));
    (
        NetworkControl {
            stop,
            enrollment,
            view: snapshot,
            operations: Arc::downgrade(&operations),
        },
        task,
    )
}

async fn run(
    authority: PeerAuthority,
    options: NetworkOptions,
    mut stop: watch::Receiver<bool>,
    view: watch::Sender<NetworkSnapshot>,
    enrollment: enrollment::Owner,
    operations: Arc<operations::Hub>,
    workers: tokio::sync::mpsc::Receiver<operations::Worker>,
) -> Result<(), NetworkFailure> {
    let (closing, closed) = watch::channel(false);
    let listener = tokio::net::TcpListener::bind(std::net::SocketAddrV4::new(options.bind, 0))
        .await
        .map_err(|_| NetworkFailure::Listener);
    let link = listener
        .as_ref()
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|address| address.port());
    let normal = supervisor::run(
        authority.clone(),
        options.clone(),
        stop.clone(),
        view.clone(),
        operations,
        workers,
        link,
    );
    let enrollment = enrollment.run(authority, listener, closed, view);
    tokio::pin!(normal, enrollment);
    tokio::select! {
        biased;
        () = discovery::cancelled(&mut stop) => {
            closing.send_replace(true);
            let (normal, enrollment) = tokio::join!(normal, enrollment);
            combine(normal, enrollment)
        },
        normal = &mut normal => {
            closing.send_replace(true);
            combine(normal, enrollment.await)
        },
        enrollment = &mut enrollment => combine(normal.await, enrollment),
    }
}

fn combine(
    normal: Result<(), NetworkFailure>,
    enrollment: Result<(), NetworkFailure>,
) -> Result<(), NetworkFailure> {
    if matches!(
        enrollment,
        Err(NetworkFailure::Task | NetworkFailure::Cleanup)
    ) {
        return enrollment;
    }
    normal
}

impl NetworkControl {
    pub fn invoke_operation(
        &self,
        invocation: crate::operations::Invocation,
    ) -> Result<
        std::sync::mpsc::Receiver<Result<crate::operations::Outcome, crate::operations::Failure>>,
        crate::operations::Failure,
    > {
        if *self.stop.borrow() {
            return Err(crate::operations::Failure::Unavailable);
        }
        self.operations
            .upgrade()
            .ok_or(crate::operations::Failure::Unavailable)?
            .request(
                invocation.handle.recipient,
                operations::OperationMessage::Invoke { invocation },
            )
    }

    pub fn reconcile_operation(
        &self,
        handle: crate::operations::RequestHandle,
        cancel: bool,
    ) -> Result<
        std::sync::mpsc::Receiver<Result<crate::operations::Outcome, crate::operations::Failure>>,
        crate::operations::Failure,
    > {
        if *self.stop.borrow() {
            return Err(crate::operations::Failure::Unavailable);
        }
        let peer = handle.recipient;
        let message = if cancel {
            operations::OperationMessage::Cancel { handle }
        } else {
            operations::OperationMessage::Outcome { handle }
        };
        self.operations
            .upgrade()
            .ok_or(crate::operations::Failure::Unavailable)?
            .request(peer, message)
    }
}
