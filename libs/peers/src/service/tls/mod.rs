mod connection;
mod verify;

#[cfg(test)]
mod tests;

use std::{sync::Arc, time::SystemTime};

use rustls::{
    client::Resumption, crypto::ring, pki_types::ServerName, server::NoServerSessionStorage,
    sign::SingleCertAndKey, ClientConfig, ServerConfig,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

use super::{
    certificate::PEER_NAME, identity::unix_seconds, validate_certificate, Identity, PeerError,
    PeerPin,
};
use verify::{ClientPolicy, Expected, PinnedServer};

pub use connection::{PeerConnection, RemoteIdentity, SessionKind};

pub const NORMAL_ALPN: &[u8] = b"qol-peers/1";
pub const ENROLLMENT_ALPN: &[u8] = b"qol-link/1";
pub const NEARBY_ALPN: &[u8] = b"qol-nearby/1";

pub trait TrustPolicy: Send + Sync {
    fn is_trusted(&self, pin: &PeerPin) -> bool;
}

#[derive(Clone)]
pub struct NormalClientConfig(Arc<ClientConfig>);

#[derive(Clone)]
pub struct NormalServerConfig(Arc<ServerConfig>);

#[derive(Clone)]
pub struct EnrollmentClientConfig(Arc<ClientConfig>);

#[derive(Clone)]
pub struct EnrollmentServerConfig(Arc<ServerConfig>);

#[derive(Clone)]
pub struct NearbyClientConfig(Arc<ClientConfig>);

impl NormalClientConfig {
    pub fn new(identity: &Identity, intended_server: PeerPin) -> Result<Self, PeerError> {
        client_config(identity, Expected::Pin(intended_server), NORMAL_ALPN).map(Self)
    }

    pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        io: S,
    ) -> Result<PeerConnection<S>, PeerError> {
        connect(self.0.clone(), io, SessionKind::Normal).await
    }
}

impl NormalServerConfig {
    pub fn new(
        identity: &Identity,
        current_trust: Arc<dyn TrustPolicy>,
    ) -> Result<Self, PeerError> {
        server_config(
            identity,
            ClientPolicy::Normal(current_trust),
            &[NORMAL_ALPN],
        )
        .map(Self)
    }

    pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        io: S,
    ) -> Result<PeerConnection<S>, PeerError> {
        accept(self.0.clone(), io, SessionKind::Normal).await
    }
}

impl EnrollmentClientConfig {
    pub fn new(identity: &Identity, inviter_pin: PeerPin) -> Result<Self, PeerError> {
        client_config(identity, Expected::Pin(inviter_pin), ENROLLMENT_ALPN).map(Self)
    }

    pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        io: S,
    ) -> Result<PeerConnection<S>, PeerError> {
        connect(self.0.clone(), io, SessionKind::Enrollment).await
    }
}

impl EnrollmentServerConfig {
    pub fn new(identity: &Identity) -> Result<Self, PeerError> {
        server_config(
            identity,
            ClientPolicy::Enrollment,
            &[ENROLLMENT_ALPN, NEARBY_ALPN],
        )
        .map(Self)
    }

    pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        io: S,
    ) -> Result<PeerConnection<S>, PeerError> {
        let stream = TlsAcceptor::from(self.0.clone())
            .accept(io)
            .await
            .map_err(PeerError::from_handshake)?;
        let kind = if stream.get_ref().1.alpn_protocol() == Some(NEARBY_ALPN) {
            SessionKind::Nearby
        } else {
            SessionKind::Enrollment
        };
        PeerConnection::from_verified_stream(TlsStream::Server(stream), kind)
    }
}

impl NearbyClientConfig {
    pub fn new(identity: &Identity, peer: crate::PeerId) -> Result<Self, PeerError> {
        client_config(identity, Expected::Peer(peer), NEARBY_ALPN).map(Self)
    }

    pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        io: S,
    ) -> Result<PeerConnection<S>, PeerError> {
        connect(self.0.clone(), io, SessionKind::Nearby).await
    }
}

fn client_config(
    identity: &Identity,
    expected: Expected,
    alpn: &[u8],
) -> Result<Arc<ClientConfig>, PeerError> {
    validate_certificate(identity.certificate_der(), unix_seconds(SystemTime::now())?)?;
    let mut config = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| PeerError::TlsConfiguration)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServer(expected)))
        .with_client_cert_resolver(Arc::new(SingleCertAndKey::from(identity.certified_key()?)));
    config.alpn_protocols = vec![alpn.to_vec()];
    config.resumption = Resumption::disabled();
    config.enable_early_data = false;
    Ok(Arc::new(config))
}

fn server_config(
    identity: &Identity,
    policy: ClientPolicy,
    alpns: &[&[u8]],
) -> Result<Arc<ServerConfig>, PeerError> {
    validate_certificate(identity.certificate_der(), unix_seconds(SystemTime::now())?)?;
    let mut config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| PeerError::TlsConfiguration)?
        .with_client_cert_verifier(Arc::new(policy))
        .with_cert_resolver(Arc::new(SingleCertAndKey::from(identity.certified_key()?)));
    config.alpn_protocols = alpns.iter().map(|alpn| alpn.to_vec()).collect();
    config.session_storage = Arc::new(NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.send_half_rtt_data = false;
    Ok(Arc::new(config))
}

async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    config: Arc<ClientConfig>,
    io: S,
    kind: SessionKind,
) -> Result<PeerConnection<S>, PeerError> {
    let name = ServerName::try_from(PEER_NAME).map_err(|_| PeerError::TlsConfiguration)?;
    let stream = TlsConnector::from(config)
        .connect(name, io)
        .await
        .map_err(PeerError::from_handshake)?;
    PeerConnection::from_verified_stream(TlsStream::Client(stream), kind)
}

async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
    config: Arc<ServerConfig>,
    io: S,
    kind: SessionKind,
) -> Result<PeerConnection<S>, PeerError> {
    let stream = TlsAcceptor::from(config)
        .accept(io)
        .await
        .map_err(PeerError::from_handshake)?;
    PeerConnection::from_verified_stream(TlsStream::Server(stream), kind)
}
