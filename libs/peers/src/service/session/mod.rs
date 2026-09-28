mod active;
mod wire;

#[cfg(test)]
mod tests;

use std::{future::Future, time::Duration};

use tokio::{
    io::{split, AsyncRead, AsyncWrite},
    sync::watch,
    time::{timeout, Instant},
};

use super::{
    framing::{read_json, write_json, FrameError, FrameLimit},
    PeerAuthority, PeerConnection, PeerPin, SessionKind,
};
use crate::{
    session::{AuthenticatedSession, SessionGeneration, SessionNonce},
    AuthorityError, PeerId,
};
use wire::{Message, Version};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SessionError {
    #[error("normal session requires normal TLS")]
    WrongMode,
    #[error("normal session peer is not currently trusted")]
    Untrusted,
    #[error("normal session authority is unavailable")]
    AuthorityUnavailable,
    #[error("normal session randomness failed")]
    Randomness,
    #[error("normal session protocol failed")]
    Protocol,
    #[error("normal session generation differs")]
    Generation,
    #[error("normal session transport closed or failed")]
    Transport,
    #[error("normal session frame deadline exceeded")]
    FrameTimeout,
    #[error("normal session hello deadline exceeded")]
    HelloTimeout,
    #[error("normal session heartbeat expired")]
    IdleExpired,
    #[error("normal session control rate exceeded")]
    RateExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionOutcome {
    Cancelled,
    Closed(SessionError),
}

pub struct NormalSession<S> {
    connection: PeerConnection<S>,
    authority: PeerAuthority,
    authenticated: AuthenticatedSession,
    opened: Instant,
}

impl<S: AsyncRead + AsyncWrite + Unpin> NormalSession<S> {
    pub async fn connect(
        io: S,
        authority: PeerAuthority,
        peer: PeerId,
    ) -> Result<Self, SessionError> {
        let config = authority
            .normal_client_config(peer)
            .map_err(authority_error)?;
        let connection = timeout(Duration::from_secs(5), config.connect(io))
            .await
            .map_err(|_| SessionError::Transport)?
            .map_err(|_| SessionError::Transport)?;
        Self::open(connection, authority).await
    }

    pub async fn accept(io: S, authority: PeerAuthority) -> Result<Self, SessionError> {
        let config = authority.server_config().map_err(authority_error)?;
        let connection = timeout(Duration::from_secs(5), config.accept(io))
            .await
            .map_err(|_| SessionError::Transport)?
            .map_err(|_| SessionError::Transport)?;
        Self::open(connection, authority).await
    }

    async fn open(
        mut connection: PeerConnection<S>,
        authority: PeerAuthority,
    ) -> Result<Self, SessionError> {
        if connection.session_kind() != SessionKind::Normal {
            return Err(SessionError::WrongMode);
        }
        let changes = authority.watch_changes();
        let pin = connection.remote_identity().pin().clone();
        let local_peer = authority
            .normal_session_identity(&pin)
            .map_err(authority_error)?;
        let mut bytes = [0; 16];
        rustls::crypto::ring::default_provider()
            .secure_random
            .fill(&mut bytes)
            .map_err(|_| SessionError::Randomness)?;
        let local = SessionNonce::from_random(bytes);
        let hello = Message::Hello {
            version: Version,
            sender: local_peer,
            recipient: pin.peer_id(),
            nonce: local,
        };
        let received = {
            let (mut reader, mut writer) = split(&mut connection);
            let exchange = async {
                let (received, ()) = tokio::try_join!(
                    read_json::<_, Message>(&mut reader, FrameLimit::Normal),
                    write_json(&mut writer, &hello, FrameLimit::Normal),
                )
                .map_err(|error| {
                    if error == FrameError::Timeout {
                        return SessionError::HelloTimeout;
                    }
                    frame_error(error)
                })?;
                Ok::<Message, SessionError>(received)
            };
            tokio::select! {
                biased;
                error = watch_authority(&authority, &pin, changes) => return Err(error),
                result = timeout(Duration::from_secs(5), exchange) => result.map_err(|_| SessionError::HelloTimeout)??,
            }
        };
        authority
            .normal_session_identity(&pin)
            .map_err(authority_error)?;
        let Message::Hello {
            sender,
            recipient,
            nonce: remote,
            ..
        } = received
        else {
            return Err(SessionError::Protocol);
        };
        if sender != pin.peer_id() || recipient != local_peer {
            return Err(SessionError::Protocol);
        }
        Ok(Self {
            connection,
            authority,
            opened: Instant::now(),
            authenticated: AuthenticatedSession {
                local_peer,
                remote_peer: sender,
                generation: SessionGeneration { local, remote },
            },
        })
    }

    pub fn authenticated(&self) -> Result<AuthenticatedSession, SessionError> {
        self.authority
            .normal_session_identity(self.connection.remote_identity().pin())
            .map_err(authority_error)?;
        Ok(self.authenticated)
    }

    pub async fn run(self, cancellation: impl Future<Output = ()>) -> SessionOutcome {
        active::run(self, cancellation, None).await
    }

    pub(crate) async fn run_operations(
        self,
        cancellation: impl Future<Output = ()>,
        hub: std::sync::Arc<crate::service::network::operations::Hub>,
        io: crate::service::network::operations::SessionIo,
    ) -> SessionOutcome {
        active::run(self, cancellation, Some((hub, io))).await
    }
}

async fn watch_authority(
    authority: &PeerAuthority,
    pin: &PeerPin,
    mut changes: watch::Receiver<()>,
) -> SessionError {
    loop {
        if let Err(error) = authority.normal_session_identity(pin) {
            return authority_error(error);
        }
        if changes.changed().await.is_err() {
            return SessionError::AuthorityUnavailable;
        }
    }
}

fn authority_error(error: AuthorityError) -> SessionError {
    match error {
        AuthorityError::UnknownPeer | AuthorityError::LocalPeer => SessionError::Untrusted,
        AuthorityError::Faulted
        | AuthorityError::StaleRevision { .. }
        | AuthorityError::RevisionExhausted
        | AuthorityError::Capacity
        | AuthorityError::InvalidName
        | AuthorityError::InvalidGrant
        | AuthorityError::DuplicateGrant
        | AuthorityError::DuplicatePeer
        | AuthorityError::AlreadyExists
        | AuthorityError::MissingStore
        | AuthorityError::WriterBusy
        | AuthorityError::UnsafeStore
        | AuthorityError::InvalidSnapshot
        | AuthorityError::UnsupportedVersion
        | AuthorityError::Identity
        | AuthorityError::UnsupportedPlatform
        | AuthorityError::Storage
        | AuthorityError::Transport
        | AuthorityError::PointzAbsent
        | AuthorityError::PointzMigrated
        | AuthorityError::UnknownDevice => SessionError::AuthorityUnavailable,
    }
}

fn frame_error(error: FrameError) -> SessionError {
    match error {
        FrameError::Timeout => SessionError::FrameTimeout,
        FrameError::Io | FrameError::IncompletePrefix | FrameError::IncompletePayload => {
            SessionError::Transport
        }
        FrameError::Empty
        | FrameError::TooLarge
        | FrameError::InvalidUtf8
        | FrameError::InvalidJson
        | FrameError::DuplicateKey
        | FrameError::TooDeep
        | FrameError::TooManyValues
        | FrameError::Serialization => SessionError::Protocol,
    }
}
