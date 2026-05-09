#![cfg(unix)]
#![cfg(debug_assertions)]

use std::path::PathBuf;
use std::sync::OnceLock;

use qol_tray::features::profile::sync::{SyncConnectRequest, SyncService};
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const TEST_PATH_ROOT_ENV: &str = "QOL_TRAY_TEST_PATH_ROOT";

fn shared_path_root() -> &'static TempDir {
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let dir = TempDir::new().expect("path-root tempdir");
        std::env::set_var(TEST_PATH_ROOT_ENV, dir.path());
        dir
    })
}

async fn lock_tests() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

fn plugins_dir_under_root() -> PathBuf {
    let dir = shared_path_root()
        .path()
        .join("data")
        .join("qol-tray")
        .join("plugins");
    std::fs::create_dir_all(&dir).expect("create plugins dir");
    dir
}

async fn fresh_service() -> (SyncService, TempDir) {
    let plugins_dir = plugins_dir_under_root();
    let folder = TempDir::new().expect("folder tempdir");
    let service = SyncService::new(plugins_dir).expect("SyncService::new");
    let _ = service.disconnect().await;
    (service, folder)
}

fn folder_request(folder: &TempDir, file_name: &str) -> SyncConnectRequest {
    SyncConnectRequest::Folder {
        folder_path: folder.path().display().to_string(),
        path: file_name.to_string(),
        pull_on_launch: true,
        push_on_change: true,
    }
}

#[tokio::test]
async fn connect_with_empty_remote_writes_initial_document_to_folder_target() {
    let _serial = lock_tests().await;
    let (service, folder) = fresh_service().await;

    let target_name = "profile-initial.json";
    service
        .connect(folder_request(&folder, target_name))
        .await
        .expect("connect must succeed against an empty folder target");

    let target = folder.path().join(target_name);
    assert!(
        target.exists(),
        "folder provider must create the remote document at {}",
        target.display(),
    );
    let body = std::fs::read_to_string(&target).expect("read target");
    let parsed: serde_json::Value =
        serde_json::from_str(&body).expect("written document must be valid JSON");
    assert!(
        parsed.is_object(),
        "expected the export bundle to serialise as a JSON object, got: {parsed}",
    );

    service
        .disconnect()
        .await
        .expect("disconnect must succeed after a clean connect");
}

#[tokio::test]
async fn auto_push_after_connect_is_a_noop_because_local_matches_remote_hash() {
    let _serial = lock_tests().await;
    let (service, folder) = fresh_service().await;
    service
        .connect(folder_request(&folder, "profile-noop.json"))
        .await
        .expect("connect");

    let result = service
        .auto_push_if_dirty()
        .await
        .expect("auto_push_if_dirty must not error on a freshly-synced state");

    assert!(
        !result.applied_remote,
        "auto_push must not re-apply when state already matches: {result:?}",
    );
    let lower = result.message.to_lowercase();
    assert!(
        lower.contains("up to date") || lower.contains("disabled"),
        "expected an up-to-date or disabled message, got: {:?}",
        result.message,
    );

    service.disconnect().await.expect("disconnect");
}

#[tokio::test]
async fn disconnect_after_connect_resets_state_to_unconfigured() {
    let _serial = lock_tests().await;
    let (service, folder) = fresh_service().await;
    service
        .connect(folder_request(&folder, "profile-disconnect.json"))
        .await
        .expect("connect");

    let before = service.status();
    assert!(
        before.configured,
        "status must be configured after a successful connect: {before:?}",
    );

    service.disconnect().await.expect("disconnect");

    let after = service.status();
    assert!(
        !after.configured,
        "status must report unconfigured after disconnect: {after:?}",
    );
    assert!(
        after.provider.is_none(),
        "provider must be cleared on disconnect: {after:?}",
    );
}
