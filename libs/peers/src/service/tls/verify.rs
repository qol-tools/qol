use std::{fmt, sync::Arc};

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{ring, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    DigitallySignedStruct, DistinguishedName, Error, SignatureScheme,
};

use super::TrustPolicy;
use crate::service::{validate_certificate, CertificateError, PeerPin};

pub(super) enum Expected {
    Pin(PeerPin),
    Peer(crate::PeerId),
}

pub(super) struct PinnedServer(pub(super) Expected);

impl fmt::Debug for PinnedServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PinnedServer")
    }
}

impl ServerCertVerifier for PinnedServer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let pin = verified_pin(end_entity, intermediates, now)?;
        let matches = match &self.0 {
            Expected::Pin(expected) => pin == *expected,
            Expected::Peer(expected) => pin.peer_id() == *expected,
        };
        if !matches {
            return Err(untrusted());
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("peer sessions require TLS 1.3".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_signature(message, cert, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

pub(super) enum ClientPolicy {
    Normal(Arc<dyn TrustPolicy>),
    Enrollment,
}

impl fmt::Debug for ClientPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal(_) => f.write_str("CurrentNormalSessionTrust"),
            Self::Enrollment => f.write_str("EnrollmentIdentityProofOnly"),
        }
    }
}

impl ClientCertVerifier for ClientPolicy {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let pin = verified_pin(end_entity, intermediates, now)?;
        if let Self::Normal(policy) = self {
            if !policy.is_trusted(&pin) {
                return Err(untrusted());
            }
        }
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("peer sessions require TLS 1.3".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_signature(message, cert, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

fn verified_pin(
    cert: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
    now: UnixTime,
) -> Result<PeerPin, Error> {
    if !intermediates.is_empty() {
        return Err(certificate_error(CertificateError::Chain));
    }
    validate_certificate(cert.as_ref(), now.as_secs()).map_err(certificate_error)
}

fn certificate_error(error: CertificateError) -> Error {
    Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
        Arc::new(error),
    )))
}

fn untrusted() -> Error {
    Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
}

fn verify_signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    signature: &DigitallySignedStruct,
) -> Result<HandshakeSignatureValid, Error> {
    if signature.scheme != SignatureScheme::ECDSA_NISTP256_SHA256 {
        return Err(Error::General(
            "unsupported peer handshake signature algorithm".into(),
        ));
    }
    verify_tls13_signature(
        message,
        cert,
        signature,
        &ring::default_provider().signature_verification_algorithms,
    )
}
