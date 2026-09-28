#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CertificateError {
    #[error("certificate exceeds the 4096-byte limit")]
    TooLarge,
    #[error("certificate encoding is invalid or has trailing data")]
    Encoding,
    #[error("exactly one certificate is required")]
    Chain,
    #[error("certificate algorithm or public key is unsupported")]
    Algorithm,
    #[error("certificate extensions are duplicate, malformed, or unsupported")]
    Extensions,
    #[error("certificate usages do not satisfy the peer profile")]
    Usage,
    #[error("CA certificates are not peer identities")]
    CertificateAuthority,
    #[error("certificate is not yet valid")]
    NotYetValid,
    #[error("certificate has expired")]
    Expired,
    #[error("certificate validity interval is invalid")]
    Validity,
    #[error("certificate is not self-signed with a valid signature")]
    Signature,
    #[error("certificate does not contain the peer service name")]
    Name,
}

#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    #[error(transparent)]
    Certificate(#[from] CertificateError),
    #[error("pin must contain canonical DER for a valid uncompressed P-256 public key")]
    InvalidPin,
    #[error("clock cannot represent the requested certificate time")]
    Clock,
    #[error("identity key generation failed")]
    KeyGeneration,
    #[error("private key encoding or algorithm is invalid")]
    PrivateKey,
    #[error("private key does not match the certificate")]
    KeyMismatch,
    #[error("certificate generation failed")]
    CertificateGeneration,
    #[error("TLS configuration failed")]
    TlsConfiguration,
    #[error("TLS handshake or transport failed ({0:?})")]
    Handshake(std::io::ErrorKind),
    #[error("remote peer pin is not trusted for this connection")]
    UntrustedPeer,
    #[error("remote peer failed TLS proof of private key possession")]
    HandshakeSignature,
    #[error("negotiated TLS protocol or ALPN does not match the session")]
    Protocol,
    #[error("TLS session did not provide a verified remote certificate")]
    MissingIdentity,
}

impl PeerError {
    pub(crate) fn from_handshake(error: std::io::Error) -> Self {
        let tls = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>());
        if let Some(rustls::Error::InvalidCertificate(rustls::CertificateError::Other(inner))) = tls
        {
            if let Some(certificate) = inner.0.downcast_ref::<CertificateError>() {
                return Self::Certificate(*certificate);
            }
        }
        if matches!(
            tls,
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure
            ))
        ) {
            return Self::UntrustedPeer;
        }
        if matches!(
            tls,
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature
            ))
        ) {
            return Self::HandshakeSignature;
        }
        if matches!(tls, Some(rustls::Error::NoCertificatesPresented)) {
            return Self::MissingIdentity;
        }
        if matches!(
            tls,
            Some(rustls::Error::NoApplicationProtocol | rustls::Error::PeerIncompatible(_))
        ) {
            return Self::Protocol;
        }
        Self::Handshake(error.kind())
    }
}
