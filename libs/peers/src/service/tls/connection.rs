use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::SystemTime,
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::TlsStream;

use crate::{
    service::{identity::unix_seconds, validate_certificate, CertificateError, PeerError, PeerPin},
    PeerId,
};

use super::{ENROLLMENT_ALPN, NORMAL_ALPN};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionKind {
    Normal,
    Enrollment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteIdentity(PeerPin);

impl RemoteIdentity {
    pub fn pin(&self) -> &PeerPin {
        &self.0
    }

    pub fn peer_id(&self) -> PeerId {
        self.0.peer_id()
    }
}

pub struct PeerConnection<S> {
    stream: TlsStream<S>,
    remote: RemoteIdentity,
    kind: SessionKind,
}

impl<S: AsyncRead + AsyncWrite + Unpin> PeerConnection<S> {
    pub fn remote_identity(&self) -> &RemoteIdentity {
        &self.remote
    }

    pub fn session_kind(&self) -> SessionKind {
        self.kind
    }

    pub(super) fn from_verified_stream(
        stream: TlsStream<S>,
        kind: SessionKind,
    ) -> Result<Self, PeerError> {
        let (_, connection) = stream.get_ref();
        let alpn = match kind {
            SessionKind::Normal => NORMAL_ALPN,
            SessionKind::Enrollment => ENROLLMENT_ALPN,
        };
        if connection.is_handshaking()
            || connection.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_3)
            || connection.alpn_protocol() != Some(alpn)
        {
            return Err(PeerError::Protocol);
        }
        let certificates = connection
            .peer_certificates()
            .ok_or(PeerError::MissingIdentity)?;
        let [certificate] = certificates else {
            return Err(CertificateError::Chain.into());
        };
        let pin = validate_certificate(certificate.as_ref(), unix_seconds(SystemTime::now())?)?;
        Ok(Self {
            stream,
            remote: RemoteIdentity(pin),
            kind,
        })
    }

    #[cfg(test)]
    pub(super) fn handshake_kind(&self) -> Option<rustls::HandshakeKind> {
        self.stream.get_ref().1.handshake_kind()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for PeerConnection<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for PeerConnection<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
