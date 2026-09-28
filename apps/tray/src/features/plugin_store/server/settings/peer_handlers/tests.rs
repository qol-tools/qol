use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::post,
    Router,
};
use qol_runtime::{local_ipc::MAX_MESSAGE_BYTES, PlatformStateClient};
use tower::ServiceExt;

fn app(client: PlatformStateClient) -> Router {
    Router::new().route(
        "/peers/admin",
        post(move |request| super::admin_at(request, client.clone())),
    )
}

fn request(body: impl Into<Body>) -> Request<Body> {
    Request::post("/peers/admin")
        .header("content-type", "application/json")
        .body(body.into())
        .unwrap()
}

#[tokio::test]
async fn malformed_and_oversized_requests_are_bounded_and_never_echoed() {
    for (body, status) in [
        (
            "qol-link:private-fixture".to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "x".repeat(MAX_MESSAGE_BYTES + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let client = PlatformStateClient::new("/unused-peer-settings-fixture".into());
        let response = app(client).oneshot(request(body)).await.unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = to_bytes(response.into_body(), MAX_MESSAGE_BYTES)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("private-fixture"));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn adapter_reaches_an_attached_authority_and_preserves_lost_mutation_uncertainty() {
    use crate::features::linked_devices::tests::{attach, authority, host_at};
    use qol_peers::admin::Request as PeerRequest;
    use qol_runtime::protocol::RuntimeRequest;
    use std::io::{BufReader, Write};
    let temporary = tempfile::tempdir().unwrap();
    let host = host_at(&temporary.path().join("peers"), false);
    let shared = std::sync::Arc::new(attach(&host));
    for (index, lose_reply, expected_result) in [
        (0, false, "status"),
        (1, false, "error"),
        (2, true, "outcome_unknown"),
        (3, false, "status"),
    ] {
        let query = match index {
            0 | 1 => PeerRequest::StartSession {
                name: "fixture".into(),
            },
            2 => PeerRequest::Rename {
                expected: authority(&shared).expected(),
                name: "changed-once".into(),
            },
            _ => PeerRequest::Status,
        };
        let body = serde_json::to_vec(&query).unwrap();
        let path = temporary.path().join(format!("runtime-{index}.sock"));
        let listener = qol_runtime::local_ipc::bind_listener(&path).unwrap();
        let attached = shared.clone();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            qol_runtime::local_ipc::authorize_peer(&stream).unwrap();
            let bytes = qol_runtime::local_ipc::read_secret_line(&mut BufReader::new(&stream))
                .unwrap()
                .unwrap();
            let RuntimeRequest::PeerAdmin { request } = serde_json::from_str(&bytes).unwrap()
            else {
                panic!("peer admin")
            };
            let response = attached.peer_admin(request);
            if lose_reply {
                return;
            }
            let bytes = qol_runtime::local_ipc::encode_secret_json(&response).unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let response = app(PlatformStateClient::new(path))
            .oneshot(request(body))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if lose_reply {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = to_bytes(response.into_body(), MAX_MESSAGE_BYTES)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value[if lose_reply { "kind" } else { "result" }],
            expected_result
        );
        if index == 1 {
            assert_eq!(value["error"]["code"], "already_active");
        }
        if index == 3 {
            assert_eq!(value["status"]["authority"]["name"], "changed-once");
        }
        worker.join().unwrap();
    }
    host.shutdown();
}

#[tokio::test]
async fn invitation_validation_is_read_only_bounded_and_redacts_invalid_codes() {
    for body in [
        r#""qol-link:private-fixture""#.to_string(),
        serde_json::to_string(&format!("qol-link:{}", "x".repeat(4096))).unwrap(),
    ] {
        let response = super::routes()
            .with_state(super::CatalogState(std::sync::Arc::new(
                std::sync::Mutex::new(crate::plugins::PluginManager::new()),
            )))
            .oneshot(
                Request::post("/peers/invitation")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = to_bytes(response.into_body(), MAX_MESSAGE_BYTES)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("qol-link:"));
        assert!(!String::from_utf8_lossy(&bytes).contains("private-fixture"));
    }
}

#[tokio::test]
async fn catalog_route_projects_the_live_canonical_contract_without_caching() {
    use crate::plugins::{Plugin, PluginId, PluginManager, PluginManifest};
    use std::sync::{Arc, Mutex};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("fixture-bin"), "fixture").unwrap();
    let manifest: PluginManifest = toml::from_str(
        r#"
[plugin]
id = "fixture"
uid = "fixture-uid"
name = "Fixture"
description = ""
version = "1.0.0"
[menu]
label = "Fixture"
items = []
[runtime]
command = "fixture-bin"
[action.allowed]
label = "Allowed"
args = ["allowed"]
peer = { replay = "never" }
[action.local]
label = "Local"
args = ["local"]
"#,
    )
    .unwrap();
    let mut manager = PluginManager::new();
    manager.insert_plugin_for_test(Plugin::new(
        PluginId::new("fixture"),
        manifest,
        root.path().into(),
    ));
    let app = super::routes().with_state(super::CatalogState(Arc::new(Mutex::new(manager))));
    for available in [true, false] {
        if !available {
            std::fs::remove_file(root.path().join("fixture-bin")).unwrap();
        }
        let response = app
            .clone()
            .oneshot(Request::get("/peers/catalog").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = to_bytes(response.into_body(), MAX_MESSAGE_BYTES)
            .await
            .unwrap();
        let operations: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if !available {
            assert_eq!(operations, serde_json::json!([]));
            continue;
        }
        assert_eq!(
            operations,
            serde_json::json!([{
                "key": {"identity": {"scope": "stable", "value": "fixture-uid"}, "kind": "action", "name": "allowed"},
                "plugin_id": "fixture", "description": "Allowed"
            }])
        );
    }
    std::fs::write(
        root.path().join("qol-runtime.toml"),
        "private-invalid-contract",
    )
    .unwrap();
    let response = app
        .oneshot(Request::get("/peers/catalog").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), MAX_MESSAGE_BYTES)
        .await
        .unwrap();
    let failure: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(failure["kind"], "transport");
    assert!(!String::from_utf8_lossy(&bytes).contains("private-invalid-contract"));
}

#[tokio::test]
async fn unavailable_catalog_is_an_error_instead_of_an_empty_success() {
    let manager = std::sync::Arc::new(std::sync::Mutex::new(crate::plugins::PluginManager::new()));
    let poison = manager.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poison.lock().unwrap();
        panic!("fixture poison");
    })
    .join();
    let response = super::routes()
        .with_state(super::CatalogState(manager))
        .oneshot(Request::get("/peers/catalog").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), MAX_MESSAGE_BYTES)
        .await
        .unwrap();
    let failure: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(failure["kind"], "transport");
}
