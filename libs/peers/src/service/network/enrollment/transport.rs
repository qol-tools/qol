use std::{
    net::{SocketAddr, SocketAddrV4},
    time::Duration,
};

use tokio::{
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinSet,
    time::timeout,
};

use super::{Job, Owner, MAX_EXCHANGES};
use crate::admin::{AttemptState, EnrollmentFailure};
use crate::network::{ListenerStatus, NetworkFailure};
use crate::service::enrollment::{EnrollmentError, EnrollmentOutcome, Invitation};
use crate::service::network::{discovery::cancelled, NetworkOptions, NetworkSnapshot};
use crate::service::{PeerAuthority, PeerPin};
use crate::PeerId;

const IO_DEADLINE: Duration = Duration::from_secs(5);
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(130);

impl Owner {
    pub(in crate::service::network) async fn run(
        mut self,
        authority: PeerAuthority,
        options: NetworkOptions,
        mut stop: watch::Receiver<bool>,
        view: watch::Sender<NetworkSnapshot>,
    ) -> Result<(), NetworkFailure> {
        let mut inbound = JoinSet::new();
        let mut outbound = JoinSet::new();
        let result = self
            .serve(
                &authority,
                options,
                &mut stop,
                &view,
                &mut inbound,
                &mut outbound,
            )
            .await;
        self.commands.close();
        while self.commands.try_recv().is_ok() {}
        inbound.abort_all();
        outbound.abort_all();
        while inbound.join_next().await.is_some() {}
        while outbound.join_next().await.is_some() {}
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for attempt in attempts
            .values_mut()
            .filter(|attempt| attempt.cancel.is_some())
        {
            attempt.state = unknown(EnrollmentFailure::Unavailable);
            attempt.cancel.take();
        }
        view.send_modify(|view| {
            view.status.enrollment_listener = match result {
                Ok(()) => ListenerStatus::Closed {},
                Err(error) => ListenerStatus::Failed { error },
            };
        });
        result
    }

    async fn serve(
        &mut self,
        authority: &PeerAuthority,
        options: NetworkOptions,
        stop: &mut watch::Receiver<bool>,
        view: &watch::Sender<NetworkSnapshot>,
        inbound: &mut JoinSet<()>,
        outbound: &mut JoinSet<(crate::enrollment::TransactionId, AttemptState)>,
    ) -> Result<(), NetworkFailure> {
        if *stop.borrow() {
            return Ok(());
        }
        let mut changes = authority.watch_changes();
        let listener = TcpListener::bind(SocketAddrV4::new(options.bind, 0))
            .await
            .map_err(|_| NetworkFailure::Listener)?;
        let port = listener
            .local_addr()
            .map_err(|_| NetworkFailure::Listener)?
            .port();
        view.send_modify(|view| {
            view.status.enrollment_listener = ListenerStatus::Listening { port }
        });
        loop {
            if authority.check_enrollment_remote(None).is_err() {
                return Err(NetworkFailure::Authority);
            }
            tokio::select! {
                biased;
                () = cancelled(stop) => return Ok(()),
                change = changes.changed() => if change.is_err() { return Err(NetworkFailure::Authority); },
                result = outbound.join_next(), if !outbound.is_empty() => {
                    match result {
                        Some(Ok((transaction, state))) => self.state(transaction, state, true),
                        Some(Err(_)) => return Err(NetworkFailure::Task),
                        None => {},
                    }
                },
                result = inbound.join_next(), if !inbound.is_empty() => {
                    if matches!(result, Some(Err(_))) { return Err(NetworkFailure::Task); }
                },
                command = self.commands.recv(), if outbound.len() < MAX_EXCHANGES => {
                    let Some(job) = command else { return Ok(()); };
                    self.state(job.transaction, AttemptState::Running {}, false);
                    let authority = authority.clone();
                    outbound.spawn(async move {
                        let transaction = job.transaction;
                        (transaction, send(authority, job).await)
                    });
                },
                accepted = listener.accept(), if inbound.len() < MAX_EXCHANGES => {
                    let (stream, _) = accepted.map_err(|_| NetworkFailure::Listener)?;
                    let authority = authority.clone();
                    inbound.spawn(async move {
                        let _ = timeout(EXCHANGE_DEADLINE, receive(authority, stream)).await;
                    });
                },
            }
        }
    }
}

async fn receive(authority: PeerAuthority, stream: TcpStream) -> Result<(), EnrollmentError> {
    let config = authority.enrollment_server_config()?;
    let connection = tokio::select! {
        biased;
        () = invalidated(&authority, None) => return Err(EnrollmentError::Transport),
        result = timeout(IO_DEADLINE, config.accept(stream)) => result.map_err(|_| EnrollmentError::Transport)?.map_err(|_| EnrollmentError::Transport)?,
    };
    let peer = connection.remote_identity().peer_id();
    tokio::select! {
        biased;
        () = invalidated(&authority, Some(peer)) => Err(EnrollmentError::Transport),
        result = authority.serve_enrollment(connection) => result.map(|_| ()),
    }
}

async fn send(authority: PeerAuthority, mut job: Job) -> AttemptState {
    let result = tokio::select! {
        biased;
        () = cancelled(&mut job.cancel) => return unknown(EnrollmentFailure::Abandoned),
        () = outbound_invalidated(&authority, job.transaction, &job.pin) => return unknown(EnrollmentFailure::Unavailable),
        result = timeout(EXCHANGE_DEADLINE, exchange(&authority, &job.pin, job.transaction, &job.endpoints, job.document.as_ref())) => result.unwrap_or(Err(EnrollmentError::Transport)),
    };
    match result {
        Ok(EnrollmentOutcome::Completed(receipt)) => AttemptState::Completed { receipt },
        Ok(EnrollmentOutcome::Rejected(reason)) => AttemptState::Rejected { reason },
        Ok(EnrollmentOutcome::Pending(_)) => unknown(EnrollmentFailure::Unavailable),
        Ok(EnrollmentOutcome::Unknown { reason, .. }) | Err(reason) => unknown(failure(reason)),
    }
}

async fn exchange(
    authority: &PeerAuthority,
    pin: &PeerPin,
    transaction: crate::enrollment::TransactionId,
    endpoints: &[SocketAddr],
    document: Option<&crate::enrollment::ExportedInvitation>,
) -> Result<EnrollmentOutcome, EnrollmentError> {
    authority.outbound_join(transaction, pin)?;
    let config = authority.enrollment_client_config(pin.clone())?;
    let mut connected = None;
    for endpoint in endpoints {
        if let Ok(Ok(stream)) = timeout(IO_DEADLINE, TcpStream::connect(endpoint)).await {
            connected = Some(stream);
            break;
        }
    }
    let stream = connected.ok_or(EnrollmentError::Transport)?;
    let connection = timeout(IO_DEADLINE, config.connect(stream))
        .await
        .map_err(|_| EnrollmentError::Transport)?
        .map_err(|_| EnrollmentError::Transport)?;
    if let Some(document) = document {
        let invitation = Invitation::import(document.expose())?;
        return authority
            .redeem_enrollment(connection, &invitation, transaction)
            .await;
    }
    authority.recover_enrollment(connection, transaction).await
}

async fn invalidated(authority: &PeerAuthority, peer: Option<PeerId>) {
    let mut changes = authority.watch_changes();
    loop {
        if authority.check_enrollment_remote(peer).is_err() || changes.changed().await.is_err() {
            return;
        }
    }
}

async fn outbound_invalidated(
    authority: &PeerAuthority,
    transaction: crate::enrollment::TransactionId,
    pin: &PeerPin,
) {
    let mut changes = authority.watch_changes();
    loop {
        if authority.outbound_join(transaction, pin).is_err() || changes.changed().await.is_err() {
            return;
        }
    }
}

fn unknown(reason: EnrollmentFailure) -> AttemptState {
    AttemptState::Unknown { reason }
}

fn failure(error: EnrollmentError) -> EnrollmentFailure {
    match error {
        EnrollmentError::Authority(_) | EnrollmentError::Randomness => {
            EnrollmentFailure::Unavailable
        }
        EnrollmentError::InvalidInvitation => EnrollmentFailure::InvalidInvitation,
        EnrollmentError::Transport | EnrollmentError::Framing => EnrollmentFailure::Transport,
        EnrollmentError::Protocol => EnrollmentFailure::Protocol,
        EnrollmentError::Abandoned => EnrollmentFailure::Abandoned,
        EnrollmentError::Rejected(crate::enrollment::EnrollmentRejection::UnknownTransaction) => {
            EnrollmentFailure::UnknownTransaction
        }
        EnrollmentError::Rejected(reason) => EnrollmentFailure::Rejected(reason),
    }
}
