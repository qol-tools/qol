use super::document::{Blob, IndexDocument, RegistryLocation};
use super::{core_index, fetch, registry, IndexLocation};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedHostRelease {
    pub(crate) version: String,
    pub(crate) revision: String,
    registry: RegistryLocation,
    assets: BTreeMap<String, Blob>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedHostAsset {
    name: String,
    registry: RegistryLocation,
    blob: Blob,
}

pub(crate) async fn latest_host_release(os: &str) -> Result<Option<IndexedHostRelease>> {
    load_host_release(&core_index(), os).await
}

pub(super) async fn load_host_release(
    location: &IndexLocation,
    os: &str,
) -> Result<Option<IndexedHostRelease>> {
    Ok(host_release(&fetch::load(location).await?, os))
}

fn host_release(document: &IndexDocument, os: &str) -> Option<IndexedHostRelease> {
    let host = document.host.get(os)?;
    let Some(version) = host.versions.get(&host.latest) else {
        log::warn!(
            "The plugin index names qol-tray {} for {os} but does not list it",
            host.latest
        );
        return None;
    };
    Some(IndexedHostRelease {
        version: host.latest.clone(),
        revision: version.revision.clone(),
        registry: document.registry.clone(),
        assets: version.assets.clone(),
    })
}

impl IndexedHostRelease {
    pub(crate) fn asset(&self, name: &str) -> Result<IndexedHostAsset> {
        let blob = self.assets.get(name).with_context(|| {
            format!(
                "the plugin index lists no {name} for qol-tray {}",
                self.version
            )
        })?;
        Ok(IndexedHostAsset {
            name: name.to_string(),
            registry: self.registry.clone(),
            blob: blob.clone(),
        })
    }
}

impl IndexedHostAsset {
    pub(crate) fn location(&self) -> String {
        registry::blob_url(&self.registry, &self.blob.digest)
    }

    pub(crate) async fn open(&self) -> Result<reqwest::Response> {
        registry::open_blob(&self.registry, &self.blob).await
    }

    pub(crate) fn verify_file(&self, path: &Path) -> Result<()> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("failed to read downloaded asset {}", path.display()))?;
        registry::verify_blob(&self.blob, &bytes, &self.name)
    }
}

#[cfg(test)]
impl IndexedHostRelease {
    pub(crate) fn test_fixture(version: &str, assets: &[(&str, &[u8])]) -> Self {
        use sha2::{Digest, Sha256};
        Self {
            version: version.to_string(),
            revision: "a".repeat(40),
            registry: RegistryLocation {
                url: "https://ghcr.io".to_string(),
                repository: "qol-tools/plugins".to_string(),
            },
            assets: assets
                .iter()
                .map(|(name, bytes)| {
                    let blob = Blob {
                        digest: format!("sha256:{:x}", Sha256::digest(bytes)),
                        size: bytes.len() as u64,
                    };
                    (name.to_string(), blob)
                })
                .collect(),
        }
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{json, Value};

    pub(crate) fn host(latest: &str, versions: &[(&str, &str, Value)]) -> Value {
        let versions = versions
            .iter()
            .map(|(version, revision, assets)| {
                let entry = json!({
                    "manifest_digest": "sha256:manifest",
                    "revision": revision,
                    "assets": assets,
                });
                (version.to_string(), entry)
            })
            .collect::<serde_json::Map<_, _>>();
        json!({ "latest": latest, "versions": versions })
    }
}

#[cfg(test)]
mod tests {
    use super::super::document::fixtures::{blob, document};
    use super::fixtures::host;
    use super::*;
    use serde_json::json;

    fn indexed(hosts: serde_json::Value) -> IndexDocument {
        let mut value = document(1, "https://ghcr.io", json!({}));
        value["host"] = hosts;
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn the_host_release_is_the_latest_listed_version_for_the_os() {
        let assets = json!({ "qol-tray-linux-x86_64.tar.gz": blob("sha256:l", 3) });
        let document = indexed(json!({
            "linux": host("3.2.0", &[("3.1.0", "r1", json!({})), ("3.2.0", "r2", assets)]),
            "macos": host("3.3.0", &[("3.1.0", "r1", json!({}))]),
        }));
        let cases: &[(&str, Option<(&str, &str)>)] = &[
            ("linux", Some(("3.2.0", "r2"))),
            ("macos", None),
            ("windows", None),
        ];
        for (os, expected) in cases {
            let release = host_release(&document, os);
            assert_eq!(
                release
                    .as_ref()
                    .map(|release| (release.version.as_str(), release.revision.as_str())),
                *expected,
                "{os}"
            );
        }
        assert!(host_release(&indexed(json!({})), "linux").is_none());
    }

    #[test]
    fn a_host_asset_is_resolved_by_name_and_verified_by_size_and_digest() {
        let release = IndexedHostRelease::test_fixture("3.2.0", &[("qol-tray.exe", b"host")]);
        let asset = release.asset("qol-tray.exe").unwrap();
        assert!(asset
            .location()
            .starts_with("https://ghcr.io/v2/qol-tools/plugins/blobs/sha256:"));
        let missing = release.asset("qol-tray-install.exe").unwrap_err();
        assert!(format!("{missing:#}").contains("lists no qol-tray-install.exe"));

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("download");
        let cases: &[(&[u8], Option<&str>)] = &[
            (b"host", None),
            (b"hosts", Some("is 5 bytes but the plugin index says 4")),
            (b"tost", Some("SHA-256 mismatch")),
        ];
        for (written, failure) in cases {
            std::fs::write(&path, written).unwrap();
            let result = asset.verify_file(&path);
            match failure {
                None => result.unwrap(),
                Some(cause) => assert!(
                    format!("{:#}", result.unwrap_err()).contains(cause),
                    "{written:?}"
                ),
            }
        }
    }
}
