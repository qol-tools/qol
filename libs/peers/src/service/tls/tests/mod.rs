mod certificates;
mod handshake;

use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
    time::{Duration, SystemTime},
};

use rustls::{
    sign::{CertifiedKey, SingleCertAndKey},
    ClientConfig, ServerConfig,
};
use tokio::{
    io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream},
    time::timeout,
};
use tokio_rustls::{client, server, TlsAcceptor, TlsConnector};

use super::{NormalClientConfig, NormalServerConfig, TrustPolicy};
use crate::service::{Identity, PeerPin};

const DEADLINE: Duration = Duration::from_secs(5);

struct LiveTrust {
    trusted: RwLock<Option<PeerPin>>,
    checks: AtomicUsize,
}

impl LiveTrust {
    fn new(pin: Option<PeerPin>) -> Arc<Self> {
        Arc::new(Self {
            trusted: RwLock::new(pin),
            checks: AtomicUsize::new(0),
        })
    }
}

impl TrustPolicy for LiveTrust {
    fn is_trusted(&self, pin: &PeerPin) -> bool {
        self.checks.fetch_add(1, Ordering::SeqCst);
        self.trusted.read().unwrap().as_ref() == Some(pin)
    }
}

struct Fixture {
    client: Identity,
    server: Identity,
    trust: Arc<LiveTrust>,
    client_config: NormalClientConfig,
    server_config: NormalServerConfig,
}

impl Fixture {
    fn new() -> Self {
        let client = Identity::generate(SystemTime::now()).unwrap();
        let server = Identity::generate(SystemTime::now()).unwrap();
        let trust = LiveTrust::new(Some(client.pin().clone()));
        let client_config = NormalClientConfig::new(&client, server.pin().clone()).unwrap();
        let server_config = NormalServerConfig::new(&server, trust.clone()).unwrap();
        Self {
            client,
            server,
            trust,
            client_config,
            server_config,
        }
    }

    fn client_with_chain(
        &self,
        chain: Vec<rustls::pki_types::CertificateDer<'static>>,
    ) -> Arc<ClientConfig> {
        let mut config = (*self.client_config.0).clone();
        let key = CertifiedKey::new(chain, self.client.certified_key().unwrap().key.clone());
        config.client_auth_cert_resolver = Arc::new(SingleCertAndKey::from(key));
        Arc::new(config)
    }

    fn server_with_chain(
        &self,
        chain: Vec<rustls::pki_types::CertificateDer<'static>>,
    ) -> Arc<ServerConfig> {
        let mut config = (*self.server_config.0).clone();
        let key = CertifiedKey::new(chain, self.server.certified_key().unwrap().key.clone());
        config.cert_resolver = Arc::new(SingleCertAndKey::from(key));
        Arc::new(config)
    }
}

type ClientResult = io::Result<client::TlsStream<DuplexStream>>;
type ServerResult = io::Result<server::TlsStream<DuplexStream>>;

async fn raw_handshake(
    client_config: Arc<ClientConfig>,
    server_config: Arc<ServerConfig>,
) -> (ClientResult, ServerResult) {
    let (client_io, server_io) = duplex(16 * 1024);
    let connector = TlsConnector::from(client_config);
    let acceptor = TlsAcceptor::from(server_config);
    let name = rustls::pki_types::ServerName::try_from("qol-peer.invalid").unwrap();
    timeout(DEADLINE, async {
        tokio::join!(
            connector.connect(name, client_io),
            acceptor.accept(server_io)
        )
    })
    .await
    .expect("bounded in-memory handshake")
}

async fn exchange_data(
    client: &mut client::TlsStream<DuplexStream>,
    server: &mut server::TlsStream<DuplexStream>,
) {
    timeout(DEADLINE, async {
        tokio::join!(
            async {
                client.write_all(b"request").await.unwrap();
                client.flush().await.unwrap();
                let mut response = [0; 8];
                client.read_exact(&mut response).await.unwrap();
                assert_eq!(&response, b"response");
            },
            async {
                let mut request = [0; 7];
                server.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"request");
                server.write_all(b"response").await.unwrap();
                server.flush().await.unwrap();
            }
        );
    })
    .await
    .expect("bounded in-memory exchange");
}

fn tls_error(error: &io::Error) -> &rustls::Error {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .expect("TLS verifier error retained by tokio-rustls")
}
