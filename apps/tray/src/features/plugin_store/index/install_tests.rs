use super::document::fixtures::{blob, document, version, Signer};
use super::stage::fixtures::tree;
use super::IndexLocation;
use crate::features::plugin_store::installer::PluginInstaller;
use crate::features::plugin_store::platform::dependency_binary_output_path;
use crate::features::plugin_store::release_assets::{resolve_asset_pattern, PlatformTarget};
use crate::features::plugin_store::source::PluginSource;
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

const PLUGIN_ID: &str = "qol-shot";
const TOKEN: &str = "anonymous-pull";

#[derive(Default)]
struct Published {
    index: Vec<u8>,
    signature: String,
    blobs: HashMap<String, Vec<u8>>,
}

#[derive(Clone)]
struct Server {
    base: String,
    published: Arc<Mutex<Published>>,
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

impl Server {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = Self {
            base,
            published: Arc::default(),
        };
        let app = Router::new()
            .route("/plugins/index.json", get(serve_index))
            .route("/plugins/index.json.minisig", get(serve_signature))
            .route("/token", get(serve_token))
            .route("/v2/qol-tools/plugins/blobs/{digest}", get(serve_blob))
            .with_state(server.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        server
    }

    fn location(&self, signer: &Signer) -> IndexLocation {
        IndexLocation {
            url: format!("{}/plugins/index.json", self.base),
            public_key: signer.public_key(),
        }
    }

    fn publish(&self, signer: &Signer, serial: u64, releases: &[(&str, &[u8])]) {
        let mut published = self.published.lock().unwrap();
        let mut versions = serde_json::Map::new();
        for (release, binary) in releases {
            let entry = version(PLUGIN_ID, release, json!({}), json!({}));
            let plugin_toml = entry["plugin_toml"].as_str().unwrap().as_bytes().to_vec();
            let tree_bytes = tree(&[
                ("plugin.toml", &plugin_toml, 0o644),
                ("README.md", b"shot", 0o644),
            ]);
            let tree_digest = digest(&tree_bytes);
            let binary_digest = digest(binary);
            let assets = platform_asset_names()
                .into_iter()
                .map(|name| (name, blob(&binary_digest, binary.len() as u64)))
                .collect::<serde_json::Map<_, _>>();
            let entry = version(
                PLUGIN_ID,
                release,
                blob(&tree_digest, tree_bytes.len() as u64),
                Value::Object(assets),
            );
            published.blobs.insert(tree_digest, tree_bytes);
            published.blobs.insert(binary_digest, binary.to_vec());
            versions.insert(release.to_string(), entry);
        }
        let latest = releases.last().unwrap().0;
        let plugins = json!({ PLUGIN_ID: { "latest": latest, "versions": versions } });
        let body = serde_json::to_vec(&document(serial, &self.base, plugins)).unwrap();
        published.signature = signer.sign(&body);
        published.index = body;
    }

    fn replace_blob(&self, original: &[u8], served: &[u8]) {
        let mut published = self.published.lock().unwrap();
        published.blobs.insert(digest(original), served.to_vec());
    }
}

fn platform_asset_names() -> Vec<String> {
    vec![resolve_asset_pattern(
        &format!("{PLUGIN_ID}-{{os}}-{{arch}}"),
        PlatformTarget::current().unwrap(),
    )]
}

async fn serve_index(State(server): State<Server>) -> Vec<u8> {
    server.published.lock().unwrap().index.clone()
}

async fn serve_signature(State(server): State<Server>) -> String {
    server.published.lock().unwrap().signature.clone()
}

async fn serve_token() -> Json<Value> {
    Json(json!({ "token": TOKEN }))
}

async fn serve_blob(
    State(server): State<Server>,
    UrlPath(digest): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let authorized = headers
        .get(header::AUTHORIZATION)
        .is_some_and(|value| value == format!("Bearer {TOKEN}").as_str());
    if !authorized {
        let challenge = format!(
            r#"Bearer realm="{}/token",service="test",scope="repository:qol-tools/plugins:pull""#,
            server.base
        );
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, challenge)],
        )
            .into_response();
    }
    match server.published.lock().unwrap().blobs.get(&digest) {
        Some(bytes) => bytes.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn installed_binary(plugins_dir: &Path) -> Vec<u8> {
    std::fs::read(dependency_binary_output_path(
        &plugins_dir.join(PLUGIN_ID),
        PLUGIN_ID,
    ))
    .unwrap()
}

fn installed_version(plugins_dir: &Path) -> String {
    crate::plugins::PluginManifest::read_from_dir(plugins_dir.join(PLUGIN_ID))
        .unwrap()
        .plugin
        .version
}

#[tokio::test]
async fn install_update_and_pinned_update_follow_the_signed_index() {
    let _env = crate::test_support::env_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(tmp.path());
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let server = Server::start().await;
    let signer = Signer::new();
    let source = PluginSource::new("core", "qol-tools/qol", "main")
        .with_signed_index(server.location(&signer));
    let installer = PluginInstaller::new(plugins_dir.clone());

    server.publish(&signer, 1, &[("1.0.0", b"binary one")]);
    installer.install(&source, PLUGIN_ID).await.unwrap();
    assert_eq!(installed_version(&plugins_dir), "1.0.0");
    assert_eq!(installed_binary(&plugins_dir), b"binary one");
    assert_eq!(
        std::fs::read(plugins_dir.join(PLUGIN_ID).join("README.md")).unwrap(),
        b"shot"
    );

    server.publish(
        &signer,
        2,
        &[("1.0.0", b"binary one"), ("1.1.0", b"binary two")],
    );
    installer.update(&source, PLUGIN_ID).await.unwrap();
    assert_eq!(installed_version(&plugins_dir), "1.1.0");
    assert_eq!(installed_binary(&plugins_dir), b"binary two");

    installer
        .update_exact(&source, PLUGIN_ID, "1.0.0")
        .await
        .unwrap();
    assert_eq!(installed_version(&plugins_dir), "1.0.0");
    assert_eq!(installed_binary(&plugins_dir), b"binary one");

    server.publish(&signer, 1, &[("1.0.0", b"binary one")]);
    let replayed = installer.update(&source, PLUGIN_ID).await.unwrap_err();
    assert!(format!("{replayed:#}").contains("older"), "{replayed:#}");
}

#[tokio::test]
async fn install_refuses_files_that_do_not_match_the_index() {
    let _env = crate::test_support::env_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(tmp.path());
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let server = Server::start().await;
    let signer = Signer::new();
    let installer = PluginInstaller::new(plugins_dir.clone());

    server.publish(&signer, 1, &[("1.0.0", b"binary one")]);
    server.replace_blob(b"binary one", b"binary bad");
    let source = PluginSource::new("core", "qol-tools/qol", "main")
        .with_signed_index(server.location(&signer));
    let tampered = installer.install(&source, PLUGIN_ID).await.unwrap_err();
    assert!(
        format!("{tampered:#}").contains("SHA-256 mismatch"),
        "{tampered:#}"
    );
    assert!(!plugins_dir.join(PLUGIN_ID).exists());

    let impostor = Signer::new();
    let wrong_key = PluginSource::new("core", "qol-tools/qol", "main")
        .with_signed_index(server.location(&impostor));
    let unsigned = installer.install(&wrong_key, PLUGIN_ID).await.unwrap_err();
    assert!(
        format!("{unsigned:#}").contains("signature"),
        "{unsigned:#}"
    );
    assert!(!plugins_dir.join(PLUGIN_ID).exists());
}
