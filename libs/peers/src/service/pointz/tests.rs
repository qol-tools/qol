use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use tokio::{net::UdpSocket, sync::mpsc};

use super::{
    envelope, pairing::test_client::TestClient, LegacyPointz, PointzAdapter, PointzOptions,
    PointzSink, Seed, MAX_DEVICES,
};
use crate::pointz::{PointzDeviceId, PointzImportSource};
use crate::service::{AuthorityError, PeerAuthority};

const SEED: [u8; 32] = [9; 32];
const PHONE: [u8; 16] = [1; 16];
const PHONE_KEY: [u8; 32] = [2; 32];

struct Capture(mpsc::UnboundedSender<serde_json::Value>);

impl PointzSink for Capture {
    fn deliver(&self, command: serde_json::Value) -> bool {
        self.0.send(command).is_ok()
    }
}

fn authority() -> PeerAuthority {
    PeerAuthority::session("desktop".into(), SystemTime::now()).unwrap()
}

fn legacy(devices: &[([u8; 16], [u8; 32], &str, u64)]) -> LegacyPointz {
    let devices: Vec<_> = devices
        .iter()
        .map(|(id, key, name, paired_at_ms)| {
            serde_json::json!({
                "device_id": URL_SAFE_NO_PAD.encode(id),
                "key": URL_SAFE_NO_PAD.encode(key),
                "name": name,
                "paired_at_ms": paired_at_ms,
            })
        })
        .collect();
    LegacyPointz::decode(
        Some(URL_SAFE_NO_PAD.encode(SEED).as_bytes()),
        Some(&serde_json::to_vec(&serde_json::json!({ "devices": devices })).unwrap()),
    )
    .unwrap()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn command(key: &[u8; 32], device: [u8; 16], nonce: [u8; 16], sent_at_ms: u64) -> Vec<u8> {
    envelope::seal(
        br#"{"type":"MouseClick","button":1}"#,
        key,
        device,
        sent_at_ms,
        nonce,
    )
}

struct Harness {
    adapter: PointzAdapter,
    delivered: mpsc::UnboundedReceiver<serde_json::Value>,
    phone: UdpSocket,
}

impl Harness {
    async fn start(authority: &PeerAuthority) -> Self {
        let (sender, delivered) = mpsc::unbounded_channel();
        let options = PointzOptions {
            bind: Ipv4Addr::LOCALHOST,
            discovery_port: 0,
            command_port: 0,
            hostname: "desktop-host".into(),
        };
        let adapter = PointzAdapter::start(
            &tokio::runtime::Handle::current(),
            authority,
            options,
            Arc::new(Capture(sender)),
        )
        .unwrap_or_else(|_| panic!("adapter must bind loopback ports"));
        let phone = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        Self {
            adapter,
            delivered,
            phone,
        }
    }

    fn discovery(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.adapter.ports().0))
    }

    fn commands(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.adapter.ports().1))
    }

    async fn ask(&self, datagram: &[u8]) -> serde_json::Value {
        self.phone
            .send_to(datagram, self.discovery())
            .await
            .unwrap();
        let mut buffer = vec![0; 4096];
        let (size, _) =
            tokio::time::timeout(Duration::from_secs(5), self.phone.recv_from(&mut buffer))
                .await
                .expect("the adapter replies")
                .unwrap();
        serde_json::from_slice(&buffer[..size]).unwrap()
    }

    async fn send_command(&self, packet: &[u8]) {
        self.phone.send_to(packet, self.commands()).await.unwrap();
    }

    async fn next_delivery(&mut self) -> Option<serde_json::Value> {
        tokio::time::timeout(Duration::from_millis(500), self.delivered.recv())
            .await
            .ok()
            .flatten()
    }
}

#[test]
fn import_keeps_the_server_id_keys_and_names_and_runs_once() {
    let authority = authority();
    authority
        .import_pointz(legacy(&[(PHONE, PHONE_KEY, "  Pixel\u{7}  ", 10)]))
        .unwrap();

    let projection = authority.pointz().unwrap().unwrap();
    assert_eq!(projection.server_id, Seed::from_bytes(SEED).server_id());
    assert_eq!(projection.import.source, PointzImportSource::Legacy);
    assert_eq!(projection.devices.len(), 1);
    assert_eq!(projection.devices[0].name, "Pixel");
    assert_eq!(
        *authority
            .pointz_key(&PointzDeviceId::from_bytes(PHONE))
            .unwrap(),
        PHONE_KEY
    );
    assert_eq!(
        authority.import_pointz(legacy(&[([5; 16], [6; 32], "Other", 11)])),
        Err(AuthorityError::PointzMigrated)
    );
    assert_eq!(authority.pointz().unwrap().unwrap().devices.len(), 1);
    assert_eq!(authority.initialize_pointz(), Ok(None));
}

#[test]
fn a_fresh_section_is_created_once_and_names_never_arrive_empty() {
    let authority = authority();
    assert_eq!(authority.pointz().unwrap(), None);
    assert!(authority.initialize_pointz().unwrap().is_some());
    let server_id = authority.pointz().unwrap().unwrap().server_id;
    assert_eq!(authority.initialize_pointz(), Ok(None));
    assert_eq!(authority.pointz().unwrap().unwrap().server_id, server_id);

    authority
        .pair_pointz(
            PointzDeviceId::from_bytes(PHONE),
            zeroize::Zeroizing::new(PHONE_KEY),
            "\u{1b} ",
        )
        .unwrap();
    assert_eq!(
        authority.pointz().unwrap().unwrap().devices[0].name,
        "Phone"
    );
}

#[test]
fn removal_needs_the_current_revision_and_a_paired_device() {
    let authority = authority();
    authority
        .import_pointz(legacy(&[(PHONE, PHONE_KEY, "Pixel", 10)]))
        .unwrap();
    let revision = authority.projection().unwrap().revision;
    let device = PointzDeviceId::from_bytes(PHONE);

    assert!(matches!(
        authority.remove_pointz_device(crate::StoreRevision::INITIAL, device),
        Err(AuthorityError::StaleRevision { .. })
    ));
    authority.remove_pointz_device(revision, device).unwrap();
    assert_eq!(authority.pointz_key(&device), None);
    assert_eq!(
        authority.remove_pointz_device(authority.projection().unwrap().revision, device),
        Err(AuthorityError::UnknownDevice)
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn imported_phones_survive_a_restart_and_removal_is_durable() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("authority");
    let authority =
        PeerAuthority::create_persistent(&root, "desktop".into(), SystemTime::now()).unwrap();
    authority
        .import_pointz(legacy(&[
            (PHONE, PHONE_KEY, "Pixel", 10),
            ([3; 16], [4; 32], "Tablet", 11),
        ]))
        .unwrap();
    let revision = authority.projection().unwrap().revision;
    authority
        .remove_pointz_device(revision, PointzDeviceId::from_bytes([3; 16]))
        .unwrap();
    drop(authority);

    let reopened = PeerAuthority::open_persistent(&root, SystemTime::now()).unwrap();
    let projection = reopened.pointz().unwrap().unwrap();
    assert_eq!(projection.server_id, Seed::from_bytes(SEED).server_id());
    assert_eq!(projection.devices.len(), 1);
    assert_eq!(
        *reopened
            .pointz_key(&PointzDeviceId::from_bytes(PHONE))
            .unwrap(),
        PHONE_KEY
    );
    assert_eq!(
        reopened.import_pointz(legacy(&[([3; 16], [4; 32], "Tablet", 11)])),
        Err(AuthorityError::PointzMigrated)
    );
}

#[tokio::test]
async fn discovery_answers_with_the_imported_server_id() {
    let authority = authority();
    authority.import_pointz(legacy(&[])).unwrap();
    let harness = Harness::start(&authority).await;

    let reply = harness.ask(b"DISCOVER").await;

    assert_eq!(
        reply,
        serde_json::json!({
            "hostname": "desktop-host",
            "server_id": Seed::from_bytes(SEED).server_id(),
            "authentication": "pair-x25519-v1",
            "pairing_open": false,
        })
    );
}

#[tokio::test]
async fn a_phone_pairs_through_the_adapter_and_its_commands_arrive() {
    let authority = authority();
    authority.initialize_pointz().unwrap();
    let mut harness = Harness::start(&authority).await;
    let code = harness.adapter.begin_pairing().unwrap().code.unwrap();
    let server_id = authority.pointz().unwrap().unwrap().server_id;
    let client = TestClient::new(PHONE);

    assert_eq!(harness.ask(b"DISCOVER").await["pairing_open"], true);
    let offer = harness.ask(&client.hello()).await;
    let (k_auth, k_wrap) = client.keys(&offer, &code);
    let result = harness.ask(&client.confirm(&k_auth)).await;
    let key = client
        .open_key(&result, &k_auth, &k_wrap, &server_id)
        .expect("the phone receives a key it can open");

    assert!(!harness.adapter.pairing().open);
    assert_eq!(
        authority.pointz().unwrap().unwrap().devices[0].name,
        "TestPhone"
    );
    harness
        .send_command(&command(&key, PHONE, [1; 16], now_ms()))
        .await;
    assert_eq!(
        harness.next_delivery().await,
        Some(serde_json::json!({"type": "MouseClick", "button": 1}))
    );
}

#[tokio::test]
async fn a_pairing_that_cannot_be_stored_hands_out_no_key() {
    let authority = authority();
    let full: Vec<_> = (0..MAX_DEVICES as u8)
        .map(|index| ([index + 10; 16], [index; 32], "Phone", u64::from(index)))
        .collect();
    authority.import_pointz(legacy(&full)).unwrap();
    let harness = Harness::start(&authority).await;
    let code = harness.adapter.begin_pairing().unwrap().code.unwrap();
    let client = TestClient::new(PHONE);

    let offer = harness.ask(&client.hello()).await;
    let (k_auth, _) = client.keys(&offer, &code);
    let result = harness.ask(&client.confirm(&k_auth)).await;

    assert_eq!(result["type"], "PairError");
    assert_eq!(result["reason"], "unavailable");
    assert!(result.get("sealed").is_none());
    assert_eq!(
        authority.pointz_key(&PointzDeviceId::from_bytes(PHONE)),
        None
    );
}

#[tokio::test]
async fn replays_stale_commands_and_removed_phones_are_refused() {
    let authority = authority();
    authority
        .import_pointz(legacy(&[(PHONE, PHONE_KEY, "Pixel", 10)]))
        .unwrap();
    let mut harness = Harness::start(&authority).await;
    let packet = command(&PHONE_KEY, PHONE, [1; 16], now_ms());

    harness.send_command(&packet).await;
    assert!(harness.next_delivery().await.is_some());
    harness.send_command(&packet).await;
    assert_eq!(harness.next_delivery().await, None);
    harness
        .send_command(&command(&PHONE_KEY, PHONE, [2; 16], now_ms() - 31_000))
        .await;
    assert_eq!(harness.next_delivery().await, None);

    let revision = authority.projection().unwrap().revision;
    authority
        .remove_pointz_device(revision, PointzDeviceId::from_bytes(PHONE))
        .unwrap();
    harness
        .send_command(&command(&PHONE_KEY, PHONE, [3; 16], now_ms()))
        .await;
    assert_eq!(harness.next_delivery().await, None);
}
