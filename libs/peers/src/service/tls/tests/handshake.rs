use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::SystemTime,
};

use rustls::{
    client::Resumption,
    crypto::ring,
    pki_types::{CertificateDer, ServerName},
    sign::{CertifiedKey, Signer, SigningKey, SingleCertAndKey},
    ClientConfig, Error, HandshakeKind, SignatureAlgorithm, SignatureScheme,
};
use tokio::{
    io::{duplex, AsyncReadExt, AsyncWriteExt},
    time::timeout,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::{exchange_data, raw_handshake, tls_error, Fixture, LiveTrust, DEADLINE};
use crate::service::tls::{verify::PinnedServer, ENROLLMENT_ALPN, NORMAL_ALPN};
use crate::service::{
    EnrollmentClientConfig, EnrollmentServerConfig, Identity, NormalClientConfig,
    NormalServerConfig, PeerError, SessionKind,
};

#[tokio::test]
async fn normal_exchange_exposes_verified_identity_and_current_trust_runs_again() {
    let fixture = Fixture::new();
    for connection_number in 1..=3 {
        let (client_io, server_io) = duplex(4096);
        let (client, server) = timeout(DEADLINE, async {
            tokio::join!(
                fixture.client_config.connect(client_io),
                fixture.server_config.accept(server_io)
            )
        })
        .await
        .unwrap();
        let mut client = client.unwrap();
        let mut server = server.unwrap();
        assert_eq!(client.remote_identity().pin(), fixture.server.pin());
        assert_eq!(
            server.remote_identity().peer_id(),
            fixture.client.pin().peer_id()
        );
        assert_eq!(client.session_kind(), SessionKind::Normal);
        assert_eq!(server.session_kind(), SessionKind::Normal);
        assert_eq!(client.handshake_kind(), Some(HandshakeKind::Full));
        assert_eq!(server.handshake_kind(), Some(HandshakeKind::Full));
        timeout(DEADLINE, async {
            tokio::join!(
                async {
                    client.write_all(b"hello").await.unwrap();
                    client.flush().await.unwrap();
                },
                async {
                    let mut bytes = [0; 5];
                    server.read_exact(&mut bytes).await.unwrap();
                    assert_eq!(&bytes, b"hello");
                }
            );
        })
        .await
        .unwrap();
        assert_eq!(
            fixture.trust.checks.load(Ordering::SeqCst),
            connection_number
        );
    }
    *fixture.trust.trusted.write().unwrap() = None;
    let (_, server) = raw_handshake(
        fixture.client_config.0.clone(),
        fixture.server_config.0.clone(),
    )
    .await;
    assert!(matches!(
        tls_error(&server.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
    ));
    assert_eq!(fixture.trust.checks.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn unknown_client_and_wrong_server_pin_fail() {
    let fixture = Fixture::new();
    let stranger = Identity::generate(SystemTime::now()).unwrap();
    let wrong_pin = NormalClientConfig::new(&fixture.client, stranger.pin().clone()).unwrap();
    let (client, _) = raw_handshake(wrong_pin.0, fixture.server_config.0.clone()).await;
    assert!(matches!(
        tls_error(&client.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
    ));
    let unknown_client = NormalClientConfig::new(&stranger, fixture.server.pin().clone()).unwrap();
    let (_, server) = raw_handshake(unknown_client.0, fixture.server_config.0).await;
    assert!(matches!(
        tls_error(&server.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
    ));
}

#[tokio::test]
async fn trusted_certificate_with_another_private_key_fails_possession_proof() {
    let fixture = Fixture::new();
    *fixture.trust.trusted.write().unwrap() = Some(fixture.server.pin().clone());
    let client = fixture.client_with_chain(vec![CertificateDer::from(
        fixture.server.certificate_der().to_vec(),
    )]);
    let (_, server) = raw_handshake(client, fixture.server_config.0).await;
    assert!(matches!(
        tls_error(&server.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::BadSignature)
    ));
    assert_eq!(fixture.trust.checks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn public_transport_reports_typed_failures_without_secret_text() {
    let fixture = Fixture::new();
    let signed = Arc::new(AtomicUsize::new(0));
    let mut client = (*fixture.client_config.0).clone();
    client.client_auth_cert_resolver = Arc::new(corrupting_certificate(&fixture.client, signed));
    let connector = TlsConnector::from(Arc::new(client));
    let name = ServerName::try_from("qol-peer.invalid").unwrap();
    let (client_io, server_io) = duplex(4096);
    let (_, server) = timeout(DEADLINE, async {
        tokio::join!(
            connector.connect(name, client_io),
            fixture.server_config.accept(server_io)
        )
    })
    .await
    .unwrap();
    let error = server.err().unwrap();
    assert!(matches!(error, PeerError::HandshakeSignature));
    assert_eq!(
        error.to_string(),
        "remote peer failed TLS proof of private key possession"
    );
    let secret = fixture.client.export_secret();
    assert!(!format!("{error:?} {error}").contains(&format!("{:?}", secret.expose_pkcs8())));
    let wrong_pin = NormalClientConfig::new(&fixture.client, fixture.client.pin().clone()).unwrap();
    let (client_io, server_io) = duplex(4096);
    let (client, _) = timeout(DEADLINE, async {
        tokio::join!(
            wrong_pin.connect(client_io),
            fixture.server_config.accept(server_io)
        )
    })
    .await
    .unwrap();
    assert!(matches!(client, Err(PeerError::UntrustedPeer)));
}

#[tokio::test]
async fn client_certificate_is_mandatory_for_normal_and_enrollment() {
    let fixture = Fixture::new();
    let enrollment = EnrollmentServerConfig::new(&fixture.server).unwrap();
    for (alpn, server) in [
        (NORMAL_ALPN, fixture.server_config.0),
        (ENROLLMENT_ALPN, enrollment.0),
    ] {
        let mut client = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedServer(fixture.server.pin().clone())))
            .with_no_client_auth();
        client.alpn_protocols = vec![alpn.to_vec()];
        let (_, server) = raw_handshake(Arc::new(client), server).await;
        assert!(
            matches!(
                tls_error(&server.err().unwrap()),
                Error::NoCertificatesPresented
            ),
            "ALPN: {alpn:?}"
        );
    }
}

#[tokio::test]
async fn enrollment_proves_identity_without_establishing_normal_trust() {
    let fixture = Fixture::new();
    let client =
        EnrollmentClientConfig::new(&fixture.client, fixture.server.pin().clone()).unwrap();
    let server = EnrollmentServerConfig::new(&fixture.server).unwrap();
    let (client_io, server_io) = duplex(4096);
    let (client, server) = timeout(DEADLINE, async {
        tokio::join!(client.connect(client_io), server.accept(server_io))
    })
    .await
    .unwrap();
    let client = client.unwrap();
    let server = server.unwrap();
    assert_eq!(client.session_kind(), SessionKind::Enrollment);
    assert_eq!(server.session_kind(), SessionKind::Enrollment);
    assert_eq!(client.remote_identity().pin(), fixture.server.pin());
    assert_eq!(server.remote_identity().pin(), fixture.client.pin());
    let normal = NormalServerConfig::new(&fixture.server, LiveTrust::new(None)).unwrap();
    let (_, result) = raw_handshake(fixture.client_config.0, normal.0).await;
    assert!(matches!(
        tls_error(&result.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
    ));
}

#[tokio::test]
async fn enrollment_client_rejects_wrong_inviter_pin() {
    let fixture = Fixture::new();
    let client =
        EnrollmentClientConfig::new(&fixture.client, fixture.client.pin().clone()).unwrap();
    let server = EnrollmentServerConfig::new(&fixture.server).unwrap();
    let (client, _) = raw_handshake(client.0, server.0).await;
    assert!(matches!(
        tls_error(&client.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
    ));
}

#[tokio::test]
async fn normal_and_enrollment_alpn_cannot_cross() {
    let fixture = Fixture::new();
    let enrollment_client =
        EnrollmentClientConfig::new(&fixture.client, fixture.server.pin().clone()).unwrap();
    let enrollment_server = EnrollmentServerConfig::new(&fixture.server).unwrap();
    for (client, server) in [
        (fixture.client_config.0, enrollment_server.0),
        (enrollment_client.0, fixture.server_config.0),
    ] {
        let (_, server) = raw_handshake(client, server).await;
        assert!(matches!(
            tls_error(&server.err().unwrap()),
            Error::NoApplicationProtocol
        ));
    }
}

#[tokio::test]
async fn missing_alpn_is_rejected_before_a_peer_stream_is_exposed() {
    let fixture = Fixture::new();
    let mut client = (*fixture.client_config.0).clone();
    client.alpn_protocols.clear();
    let (client_io, server_io) = duplex(4096);
    let connector = TlsConnector::from(Arc::new(client));
    let name = ServerName::try_from("qol-peer.invalid").unwrap();
    let (_, server) = timeout(DEADLINE, async {
        tokio::join!(
            connector.connect(name, client_io),
            fixture.server_config.accept(server_io)
        )
    })
    .await
    .unwrap();
    assert!(matches!(server, Err(PeerError::Protocol)));
    let mut server = (*fixture.server_config.0).clone();
    server.alpn_protocols.clear();
    let acceptor = TlsAcceptor::from(Arc::new(server));
    let (client_io, server_io) = duplex(4096);
    let (client, _) = timeout(DEADLINE, async {
        tokio::join!(
            fixture.client_config.connect(client_io),
            acceptor.accept(server_io)
        )
    })
    .await
    .unwrap();
    assert!(matches!(client, Err(PeerError::Protocol)));
}

#[tokio::test]
async fn tls12_client_hello_cannot_negotiate_a_peer_session() {
    let fixture = Fixture::new();
    let enrollment = EnrollmentServerConfig::new(&fixture.server).unwrap();
    for (alpn, server) in [
        (NORMAL_ALPN, fixture.server_config.0.clone()),
        (ENROLLMENT_ALPN, enrollment.0),
    ] {
        let mut client = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedServer(fixture.server.pin().clone())))
            .with_client_cert_resolver(fixture.client_config.0.client_auth_cert_resolver.clone());
        client.alpn_protocols = vec![alpn.to_vec()];
        let (client, server) = raw_handshake(Arc::new(client), server).await;
        assert!(matches!(
            tls_error(&client.expect_err("TLS1.2 client must be rejected")),
            Error::AlertReceived(rustls::AlertDescription::ProtocolVersion)
        ));
        assert!(matches!(
            tls_error(&server.expect_err("peer server requires TLS1.3")),
            Error::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOfferedOrEnabled)
        ));
    }
}

#[derive(Debug)]
struct CorruptingKey {
    real: Arc<dyn SigningKey>,
    signed: Arc<AtomicUsize>,
}

impl SigningKey for CorruptingKey {
    fn choose_scheme(&self, schemes: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        self.real.choose_scheme(schemes).map(|real| {
            Box::new(CorruptingSigner {
                real,
                signed: self.signed.clone(),
            }) as Box<dyn Signer>
        })
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        self.real.algorithm()
    }
}

#[derive(Debug)]
struct CorruptingSigner {
    real: Box<dyn Signer>,
    signed: Arc<AtomicUsize>,
}

impl Signer for CorruptingSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, Error> {
        self.signed.fetch_add(1, Ordering::SeqCst);
        let mut signature = self.real.sign(message)?;
        *signature.last_mut().unwrap() ^= 1;
        Ok(signature)
    }

    fn scheme(&self) -> SignatureScheme {
        self.real.scheme()
    }
}

fn corrupting_certificate(identity: &Identity, signed: Arc<AtomicUsize>) -> SingleCertAndKey {
    let key = Arc::new(CorruptingKey {
        real: identity.certified_key().unwrap().key.clone(),
        signed,
    });
    SingleCertAndKey::from(CertifiedKey::new(
        vec![CertificateDer::from(identity.certificate_der().to_vec())],
        key,
    ))
}

#[tokio::test]
async fn certificate_verify_signatures_are_checked_on_both_sides_and_during_enrollment() {
    let fixture = Fixture::new();
    let signed = Arc::new(AtomicUsize::new(0));
    let mut client = (*fixture.client_config.0).clone();
    client.client_auth_cert_resolver =
        Arc::new(corrupting_certificate(&fixture.client, signed.clone()));
    let (_, server) = raw_handshake(Arc::new(client), fixture.server_config.0.clone()).await;
    assert!(matches!(
        tls_error(&server.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::BadSignature)
    ));
    assert_eq!(signed.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.trust.checks.load(Ordering::SeqCst), 1);
    let mut server = (*fixture.server_config.0).clone();
    server.cert_resolver = Arc::new(corrupting_certificate(&fixture.server, signed.clone()));
    let (client, _) = raw_handshake(fixture.client_config.0, Arc::new(server)).await;
    assert!(matches!(
        tls_error(&client.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::BadSignature)
    ));
    assert_eq!(signed.load(Ordering::SeqCst), 2);
    let enrollment =
        EnrollmentClientConfig::new(&fixture.client, fixture.server.pin().clone()).unwrap();
    let mut client = (*enrollment.0).clone();
    client.client_auth_cert_resolver =
        Arc::new(corrupting_certificate(&fixture.client, signed.clone()));
    let server = EnrollmentServerConfig::new(&fixture.server).unwrap();
    let (_, server) = raw_handshake(Arc::new(client), server.0).await;
    assert!(matches!(
        tls_error(&server.err().unwrap()),
        Error::InvalidCertificate(rustls::CertificateError::BadSignature)
    ));
    assert_eq!(signed.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn disabled_resumption_and_early_data_require_fresh_proof_despite_willing_remote() {
    for willing_remote in ["neither", "client", "server"] {
        let fixture = Fixture::new();
        let mut client = (*fixture.client_config.0).clone();
        let mut server = (*fixture.server_config.0).clone();
        assert!(!server.ticketer.enabled());
        assert_eq!(server.send_tls13_tickets, 0);
        assert_eq!(server.max_early_data_size, 0);
        assert!(!client.enable_early_data);
        if willing_remote == "client" {
            client.resumption = Resumption::in_memory_sessions(64);
            client.enable_early_data = true;
        }
        if willing_remote == "server" {
            server.session_storage = rustls::server::ServerSessionMemoryCache::new(64);
            server.send_tls13_tickets = 2;
            server.max_early_data_size = 1024;
        }
        let client = Arc::new(client);
        let server = Arc::new(server);
        for round in 1..=3 {
            let (client_result, server_result) =
                raw_handshake(client.clone(), server.clone()).await;
            let mut client_stream = client_result.unwrap();
            let mut server_stream = server_result.unwrap();
            exchange_data(&mut client_stream, &mut server_stream).await;
            assert_eq!(
                client_stream.get_ref().1.handshake_kind(),
                Some(HandshakeKind::Full),
                "remote: {willing_remote}, round: {round}"
            );
            assert_eq!(
                server_stream.get_ref().1.handshake_kind(),
                Some(HandshakeKind::Full),
                "remote: {willing_remote}, round: {round}"
            );
            assert!(
                !client_stream.get_ref().1.is_early_data_accepted(),
                "remote: {willing_remote}, round: {round}"
            );
            assert_eq!(
                fixture.trust.checks.load(Ordering::SeqCst),
                round,
                "remote: {willing_remote}"
            );
        }
        let name = ServerName::try_from("qol-peer.invalid").unwrap();
        let mut probe = rustls::ClientConnection::new(client.clone(), name).unwrap();
        assert!(probe.early_data().is_none(), "remote: {willing_remote}");
    }
}
