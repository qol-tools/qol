use std::time::{Duration, SystemTime};

use tokio::{
    io::{duplex, AsyncReadExt, AsyncWriteExt},
    time::timeout,
};

use super::{grant, revision, PeerAuthority};
use crate::service::{Identity, NormalClientConfig, PeerError};

#[tokio::test]
async fn existing_tls_configuration_reads_live_authority_after_revocation() {
    let now = SystemTime::now() - Duration::from_secs(60);
    let authority = PeerAuthority::session("server".into(), now).unwrap();
    let client_identity = Identity::generate(now).unwrap();
    let client_id = client_identity.pin().peer_id();
    authority
        .insert_link(
            revision(&authority),
            client_identity.pin().clone(),
            "client".into(),
        )
        .unwrap();
    authority
        .set_grants(revision(&authority), client_id, vec![grant("run")])
        .unwrap();
    let client_config =
        NormalClientConfig::new(&client_identity, authority.local_pin().unwrap()).unwrap();
    let server_config = authority.server_config().unwrap();
    let (client_io, server_io) = duplex(4096);
    let (client, server) = timeout(Duration::from_secs(5), async {
        tokio::join!(
            client_config.connect(client_io),
            server_config.accept(server_io)
        )
    })
    .await
    .unwrap();
    let mut client = client.unwrap();
    let mut server = server.unwrap();
    assert_eq!(server.remote_identity().peer_id(), client_id);
    timeout(Duration::from_secs(5), async {
        tokio::join!(
            async {
                client.write_all(b"proof").await.unwrap();
                client.flush().await.unwrap();
            },
            async {
                let mut data = [0; 5];
                server.read_exact(&mut data).await.unwrap();
                assert_eq!(&data, b"proof");
            }
        );
    })
    .await
    .unwrap();
    authority.revoke(revision(&authority), client_id).unwrap();
    assert!(!authority.has_grant(client_id, &grant("run")));
    let (client_io, server_io) = duplex(4096);
    let (_, rejected) = timeout(Duration::from_secs(5), async {
        tokio::join!(
            client_config.connect(client_io),
            server_config.accept(server_io)
        )
    })
    .await
    .unwrap();
    assert!(matches!(rejected, Err(PeerError::UntrustedPeer)));
}
