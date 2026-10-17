use super::document::fixtures::{blob, document, version, Signer};
use super::host::fixtures::host;
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
    blobs: HashMap<String, Vec<u8>>,
    manifests: HashMap<String, Value>,
    tokens_issued: usize,
    indexes_served: usize,
    manifests_served: usize,
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
            .route("/plugins/index.signed.json", get(serve_index))
            .route("/token", get(serve_token))
            .route("/v2/qol-tools/plugins/blobs/{digest}", get(serve_blob))
            .route(
                "/v2/qol-tools/plugins/manifests/{reference}",
                get(serve_manifest),
            )
            .with_state(server.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        server
    }

    fn location(&self, signer: &Signer) -> IndexLocation {
        IndexLocation {
            url: format!("{}/plugins/index.signed.json", self.base),
            public_key: signer.public_key(),
            registry: None,
        }
    }

    fn registry_location(&self, signer: &Signer) -> IndexLocation {
        IndexLocation {
            registry: Some(qol_plugin_index::RegistryLocation {
                url: self.base.clone(),
                repository: "qol-tools/plugins".to_string(),
            }),
            ..self.location(signer)
        }
    }

    fn push_index(&self, signed: &[u8]) {
        let mut published = self.published.lock().unwrap();
        let layer = json!({
            "mediaType": qol_plugin_index::SIGNED_INDEX_MEDIA_TYPE,
            "digest": digest(signed),
            "size": signed.len(),
        });
        let manifest = json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": "application/vnd.qol.plugin-index.v1",
            "layers": [layer],
        });
        published.blobs.insert(digest(signed), signed.to_vec());
        published
            .manifests
            .insert(qol_plugin_index::INDEX_TAG.to_string(), manifest);
    }

    fn publish_host(&self, signer: &Signer, serial: u64, asset_name: &str, bytes: &[u8]) {
        let mut published = self.published.lock().unwrap();
        let assets = json!({ asset_name: blob(&digest(bytes), bytes.len() as u64) });
        let mut body = document(serial, &self.base, json!({}));
        body["host"] = json!({ "linux": host("3.2.0", &[("3.2.0", "r2", assets)]) });
        published.blobs.insert(digest(bytes), bytes.to_vec());
        published.index = signer.sign_index(&serde_json::to_vec(&body).unwrap());
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
        published.index = signer.sign_index(&body);
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
    let mut published = server.published.lock().unwrap();
    published.indexes_served += 1;
    published.index.clone()
}

async fn serve_token(State(server): State<Server>) -> Json<Value> {
    server.published.lock().unwrap().tokens_issued += 1;
    Json(json!({ "token": TOKEN }))
}

fn unauthorized(server: &Server, headers: &HeaderMap) -> Option<Response> {
    let authorized = headers
        .get(header::AUTHORIZATION)
        .is_some_and(|value| value == format!("Bearer {TOKEN}").as_str());
    if authorized {
        return None;
    }
    let challenge = format!(
        r#"Bearer realm="{}/token",service="test",scope="repository:qol-tools/plugins:pull""#,
        server.base
    );
    Some(
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, challenge)],
        )
            .into_response(),
    )
}

async fn serve_blob(
    State(server): State<Server>,
    UrlPath(digest): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(refused) = unauthorized(&server, &headers) {
        return refused;
    }
    match server.published.lock().unwrap().blobs.get(&digest) {
        Some(bytes) => bytes.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_manifest(
    State(server): State<Server>,
    UrlPath(reference): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(refused) = unauthorized(&server, &headers) {
        return refused;
    }
    let mut published = server.published.lock().unwrap();
    published.manifests_served += 1;
    match published.manifests.get(&reference) {
        Some(manifest) => Json(manifest.clone()).into_response(),
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
    let signer = Signer::generate();
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
    assert_eq!(server.published.lock().unwrap().tokens_issued, 1);
    assert_eq!(server.published.lock().unwrap().indexes_served, 3);

    server.publish(&signer, 1, &[("1.0.0", b"binary one")]);
    installer.update(&source, PLUGIN_ID).await.unwrap();
    assert_eq!(installed_version(&plugins_dir), "1.1.0");
    assert_eq!(installed_binary(&plugins_dir), b"binary two");
}

#[tokio::test]
async fn install_refuses_files_that_do_not_match_the_index() {
    let _env = crate::test_support::env_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(tmp.path());
    let plugins_dir = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins_dir).unwrap();
    let server = Server::start().await;
    let signer = Signer::generate();
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

    let impostor = Signer::generate();
    let wrong_key = PluginSource::new("core", "qol-tools/qol", "main")
        .with_signed_index(server.location(&impostor));
    let unsigned = installer.install(&wrong_key, PLUGIN_ID).await.unwrap_err();
    assert!(
        format!("{unsigned:#}").contains("signature"),
        "{unsigned:#}"
    );
    assert!(!plugins_dir.join(PLUGIN_ID).exists());
}

#[tokio::test]
async fn an_index_or_blob_that_cannot_be_reached_names_what_failed() {
    let _env = crate::test_support::env_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(tmp.path());
    let server = Server::start().await;
    let signer = Signer::generate();
    server.publish(&signer, 1, &[("1.0.0", b"binary one")]);
    let missing_index = IndexLocation {
        url: format!("{}/missing/index.signed.json", server.base),
        public_key: signer.public_key(),
        registry: None,
    };

    let staging = tmp.path().join("staging");
    let not_served = super::stage_release(&missing_index, PLUGIN_ID, None, &staging)
        .await
        .unwrap_err();
    let not_listed = super::stage_release(&server.location(&signer), "qol-missing", None, &staging)
        .await
        .unwrap_err();
    server.published.lock().unwrap().blobs.clear();
    let blob_gone = super::stage_release(&server.location(&signer), PLUGIN_ID, None, &staging)
        .await
        .unwrap_err();

    for (error, cause) in [
        (not_served, "answered 404"),
        (not_listed, "does not list qol-missing"),
        (blob_gone, "answered 404"),
    ] {
        assert!(format!("{error:#}").contains(cause), "{error:#}");
    }
}

#[tokio::test]
async fn the_registry_index_is_read_first_and_pages_answers_when_it_fails() {
    let _env = crate::test_support::env_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(tmp.path());
    let server = Server::start().await;
    let signer = Signer::generate();
    server.publish(&signer, 1, &[("1.0.0", b"binary one")]);
    let pages_copy = server.published.lock().unwrap().index.clone();
    let signed: qol_plugin_index::SignedIndex = serde_json::from_slice(&pages_copy).unwrap();
    let impostor_copy = Signer::generate().sign_index(signed.index.as_bytes());
    type Case<'a> = (&'a str, Option<&'a [u8]>, Option<&'a [u8]>, &'a str);
    let cases: &[Case] = &[
        (
            "registry serves the index",
            Some(pages_copy.as_slice()),
            None,
            "registry",
        ),
        ("registry has no index tag", None, None, "pages"),
        (
            "registry blob does not match its descriptor",
            Some(pages_copy.as_slice()),
            Some(b"{}".as_slice()),
            "pages",
        ),
        (
            "registry index is signed by another key",
            Some(impostor_copy.as_slice()),
            None,
            "pages",
        ),
    ];
    for (case, pushed, served_instead, expected) in cases {
        {
            let mut published = server.published.lock().unwrap();
            published.manifests.clear();
            published.indexes_served = 0;
            published.manifests_served = 0;
        }
        if let Some(pushed) = pushed {
            server.push_index(pushed);
            if let Some(served) = served_instead {
                server.replace_blob(pushed, served);
            }
        }
        let document = super::fetch::load(&server.registry_location(&signer))
            .await
            .unwrap();
        assert_eq!(document.serial, 1, "{case}");
        assert!(document.plugins.contains_key(PLUGIN_ID), "{case}");
        let published = server.published.lock().unwrap();
        let read_from = if published.indexes_served == 0 {
            "registry"
        } else {
            "pages"
        };
        assert_eq!(read_from, *expected, "{case}");
        assert_eq!(published.manifests_served, 1, "{case}");
    }
}

#[tokio::test]
async fn a_host_asset_is_pulled_from_the_registry_and_checked_against_the_index() {
    let _env = crate::test_support::env_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(tmp.path());
    let server = Server::start().await;
    let signer = Signer::generate();
    let asset_name = "qol-tray-linux-x86_64.tar.gz";
    server.publish_host(&signer, 1, asset_name, b"host build");
    let release = super::host::load_host_release(&server.location(&signer), "linux")
        .await
        .unwrap()
        .expect("the index lists a linux host");
    assert_eq!(
        (release.version.as_str(), release.revision.as_str()),
        ("3.2.0", "r2")
    );
    let asset = release.asset(asset_name).unwrap();
    let path = tmp.path().join(asset_name);

    std::fs::write(&path, asset.open().await.unwrap().bytes().await.unwrap()).unwrap();
    asset.verify_file(&path).unwrap();

    server.replace_blob(b"host build", b"host bui1d");
    std::fs::write(&path, asset.open().await.unwrap().bytes().await.unwrap()).unwrap();
    let tampered = asset.verify_file(&path).unwrap_err();
    assert!(
        format!("{tampered:#}").contains("SHA-256 mismatch"),
        "{tampered:#}"
    );
    assert!(
        super::host::load_host_release(&server.location(&signer), "windows")
            .await
            .unwrap()
            .is_none()
    );
}
