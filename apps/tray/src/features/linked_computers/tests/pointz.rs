use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use qol_peers::admin::{Error, Lifecycle, PointzRequest, PointzStatus, Request, Response};
use qol_peers::pointz::{PointzImportSource, PointzPlugin, PointzTransport};
use qol_peers::AuthorityLifetime;
use sha2::{Digest, Sha256};

use super::{attach, authority, status};
use crate::features::linked_computers::pointz::PointzHostOptions;
use crate::features::linked_computers::{legacy_pointz_allowed, PeerHostHandle};
use crate::plugins::{Plugin, PluginId, PluginManager, PluginManifest};
use crate::runtime::SharedState;

static CUTOVER_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const SEED: [u8; 32] = [9; 32];

fn manifest(compatible: bool) -> PluginManifest {
    let capability = if compatible {
        "[capabilities]\npeer_trust = \"pointz-v1\"\n"
    } else {
        ""
    };
    toml::from_str(&format!(
        r#"
[plugin]
id = "qol-pointz"
uid = "9cb88d65-1d43-4fde-95b6-105761f0a14b"
name = "PointZerver"
description = ""
version = "1.45.0"
[menu]
label = "PointZ"
items = []
[daemon]
enabled = true
command = "qol-pointz-test-missing"
socket = "qol-pointz-test.sock"
{capability}"#
    ))
    .unwrap()
}

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    plugins: Arc<Mutex<PluginManager>>,
    host: PeerHostHandle,
    shared: SharedState,
}

impl Fixture {
    fn new(compatible: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("peers");
        let data_root = temporary.path().join("data");
        let data = data_root.join("plugins").join("qol-pointz");
        std::fs::create_dir_all(&data).unwrap();
        let plugins = Arc::new(Mutex::new(PluginManager::new()));
        let host = PeerHostHandle::new(Ok(root.clone()), plugins.clone(), false);
        host.initialize();
        host.attach_pointz(
            tokio::runtime::Handle::current(),
            PointzHostOptions {
                bind: Ipv4Addr::LOCALHOST,
                discovery_port: 0,
                command_port: 0,
                data_root: Some(data_root),
            },
        );
        let fixture = Self {
            _temporary: temporary,
            root,
            data,
            plugins,
            shared: attach(&host),
            host,
        };
        fixture.install(compatible);
        fixture
    }

    fn install(&self, compatible: bool) {
        self.plugins
            .lock()
            .unwrap()
            .insert_plugin_for_test(Plugin::new(
                PluginId::new("qol-pointz"),
                manifest(compatible),
                self.data.clone(),
            ));
    }

    fn write_legacy(&self, device: [u8; 16]) {
        std::fs::write(
            self.data.join("pairing-secret"),
            URL_SAFE_NO_PAD.encode(SEED),
        )
        .unwrap();
        let devices = serde_json::json!({"devices": [{
            "device_id": URL_SAFE_NO_PAD.encode(device),
            "key": URL_SAFE_NO_PAD.encode([2; 32]),
            "name": "Pixel",
            "paired_at_ms": 10,
        }]});
        std::fs::write(
            self.data.join("devices.json"),
            serde_json::to_vec(&devices).unwrap(),
        )
        .unwrap();
    }

    fn pointz(&self, request: PointzRequest) -> Response {
        self.shared.peer_admin(Request::Pointz { request })
    }

    fn pointz_status(&self) -> PointzStatus {
        let Response::PointzStatus { status } = self.pointz(PointzRequest::Status {}) else {
            panic!("PointZ status response required");
        };
        status
    }

    async fn reconcile(&self) {
        let host = self.host.clone();
        tokio::task::spawn_blocking(move || host.reconcile_pointz())
            .await
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.shutdown();
    }
}

fn server_id(seed: [u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(&Sha256::digest(seed)[..12])
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[tokio::test]
async fn legacy_phones_move_into_a_created_persistent_authority_exactly_once() {
    let _serial = CUTOVER_TESTS.lock().await;
    let fixture = Fixture::new(true);
    fixture.write_legacy([1; 16]);

    fixture.reconcile().await;

    let summary = authority(&fixture.shared);
    assert_eq!(summary.lifetime, AuthorityLifetime::Persistent);
    assert!(fixture.root.join("state.json").is_file());
    let pointz = fixture.pointz_status();
    let migration = pointz.migration.unwrap();
    assert_eq!(migration.source, PointzImportSource::Legacy);
    assert_eq!(migration.imported, 1);
    assert!(!migration.phones_must_pair_again());
    assert_eq!(pointz.server_id, Some(server_id(SEED)));
    assert_eq!(pointz.device_count, 1);
    assert_eq!(pointz.plugin, PointzPlugin::Compatible);
    assert!(matches!(pointz.transport, PointzTransport::Running { .. }));
    assert!(!exists(&fixture.data.join("pairing-secret")));
    assert!(exists(&fixture.data.join("pairing-secret.migrated")));
    assert!(exists(&fixture.data.join("devices.json.migrated")));
    assert!(!legacy_pointz_allowed());

    let Response::Changed { .. } = fixture.pointz(PointzRequest::Remove {
        expected: authority(&fixture.shared).expected(),
        device_id: qol_peers::pointz::PointzDeviceId::from_bytes([1; 16]),
    }) else {
        panic!("removal must commit");
    };
    fixture.write_legacy([1; 16]);
    fixture.reconcile().await;

    assert_eq!(fixture.pointz_status().device_count, 0);
    assert!(exists(&fixture.data.join("devices.json")));
}

#[tokio::test]
async fn a_legacy_pointz_keeps_its_own_pairing_until_cutover_and_then_cannot_start() {
    let _serial = CUTOVER_TESTS.lock().await;
    let fixture = Fixture::new(false);
    fixture.write_legacy([1; 16]);

    fixture.reconcile().await;

    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Inactive);
    assert!(exists(&fixture.data.join("devices.json")));
    assert!(legacy_pointz_allowed());
    assert_eq!(fixture.pointz_status().plugin, PointzPlugin::Legacy);
    assert_eq!(
        fixture.pointz(PointzRequest::BeginPairing {}),
        Response::Error {
            error: Error::PointzUnavailable
        }
    );

    fixture.install(true);
    fixture.reconcile().await;
    assert!(!legacy_pointz_allowed());

    let mut legacy = Plugin::new(
        PluginId::new("qol-pointz"),
        manifest(false),
        fixture.data.clone(),
    );
    let error = legacy.start_daemon().unwrap_err();
    assert!(error.to_string().contains("must be updated"));
}

#[tokio::test]
async fn pairing_on_a_fresh_computer_creates_the_authority_and_opens_a_window() {
    let _serial = CUTOVER_TESTS.lock().await;
    let fixture = Fixture::new(true);

    fixture.reconcile().await;
    assert_eq!(status(&fixture.shared).lifecycle, Lifecycle::Inactive);

    let Response::PointzPairing { pairing } = fixture.pointz(PointzRequest::BeginPairing {}) else {
        panic!("pairing response required");
    };
    assert!(pairing.open);
    let code = pairing.code.unwrap();
    assert_eq!(code.len(), 6);
    assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
    assert_eq!(
        authority(&fixture.shared).lifetime,
        AuthorityLifetime::Persistent
    );
    let pointz = fixture.pointz_status();
    assert_eq!(pointz.migration.unwrap().source, PointzImportSource::Fresh);
    assert!(pointz.pairing.open);

    fixture.pointz(PointzRequest::CancelPairing {});
    assert!(!fixture.pointz_status().pairing.open);
}
