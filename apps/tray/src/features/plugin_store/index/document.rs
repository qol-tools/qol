use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IndexDocument {
    pub(super) schema: u32,
    pub(super) serial: u64,
    pub(super) registry: RegistryLocation,
    pub(super) plugins: BTreeMap<String, IndexedPlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(super) struct RegistryLocation {
    pub(super) url: String,
    pub(super) repository: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IndexedPlugin {
    pub(super) latest: String,
    pub(super) versions: BTreeMap<String, IndexedVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IndexedVersion {
    pub(super) dir: String,
    pub(super) plugin_toml: String,
    pub(super) tree: Blob,
    pub(super) assets: BTreeMap<String, Blob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(super) struct Blob {
    pub(super) digest: String,
    pub(super) size: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedRelease {
    pub(super) plugin_id: String,
    pub(super) version: String,
    pub(super) registry: RegistryLocation,
    pub(super) files: IndexedVersion,
}

pub(super) fn verify(body: &[u8], signature: &str, public_key: &str) -> Result<IndexDocument> {
    let key = minisign_verify::PublicKey::from_base64(public_key)
        .context("the plugin index public key is invalid")?;
    let signature = minisign_verify::Signature::decode(signature)
        .context("the plugin index signature is unreadable")?;
    key.verify(body, &signature, false)
        .context("the plugin index signature does not match")?;
    let document: IndexDocument =
        serde_json::from_slice(body).context("the plugin index is unreadable")?;
    if document.schema != SCHEMA {
        anyhow::bail!("plugin index schema {} is not supported", document.schema);
    }
    Ok(document)
}

impl IndexDocument {
    pub(super) fn release(&self, plugin_id: &str, version: Option<&str>) -> Result<IndexedRelease> {
        let plugin = self
            .plugins
            .get(plugin_id)
            .with_context(|| format!("the plugin index does not list {plugin_id}"))?;
        let version = version.unwrap_or(&plugin.latest);
        let files = plugin
            .versions
            .get(version)
            .with_context(|| format!("the plugin index does not list {plugin_id} {version}"))?;
        Ok(IndexedRelease {
            plugin_id: plugin_id.to_string(),
            version: version.to_string(),
            registry: self.registry.clone(),
            files: files.clone(),
        })
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    use serde_json::{json, Value};

    pub(crate) struct Signer {
        keypair: minisign::KeyPair,
    }

    impl Signer {
        pub(crate) fn new() -> Self {
            Self {
                keypair: minisign::KeyPair::generate_unencrypted_keypair().expect("test keypair"),
            }
        }

        pub(crate) fn public_key(&self) -> String {
            self.keypair.pk.to_base64()
        }

        pub(crate) fn sign(&self, body: &[u8]) -> String {
            minisign::sign(
                Some(&self.keypair.pk),
                &self.keypair.sk,
                body,
                Some("qol plugin index"),
                None,
            )
            .expect("test signature")
            .into_string()
        }
    }

    pub(crate) fn blob(digest: &str, size: u64) -> Value {
        json!({ "digest": digest, "size": size })
    }

    pub(crate) fn version(plugin_id: &str, version: &str, tree: Value, assets: Value) -> Value {
        let plugin_toml = format!(
            "[plugin]\nid = \"{plugin_id}\"\nname = \"{plugin_id}\"\ndescription = \"test\"\nversion = \"{version}\"\nplatforms = [\"linux\", \"macos\", \"windows\"]\n\n[menu]\nlabel = \"{plugin_id}\"\nitems = []\n\n[runtime]\ncommand = \"{plugin_id}\"\n\n[[dependencies.binaries]]\nname = \"{plugin_id}\"\nrepo = \"qol-tools/{plugin_id}\"\npattern = \"{plugin_id}-{{os}}-{{arch}}\"\n"
        );
        json!({
            "manifest_digest": "sha256:manifest",
            "dir": plugin_id.trim_start_matches("qol-"),
            "plugin_toml": plugin_toml,
            "tree": tree,
            "assets": assets,
        })
    }

    pub(crate) fn document(serial: u64, registry_url: &str, plugins: Value) -> Value {
        json!({
            "schema": 1,
            "serial": serial,
            "registry": { "url": registry_url, "repository": "qol-tools/plugins" },
            "plugins": plugins,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{blob, document, version, Signer};
    use super::*;
    use serde_json::json;

    fn two_version_index(serial: u64) -> Vec<u8> {
        let plugins = json!({
            "qol-shot": {
                "latest": "1.2.0",
                "versions": {
                    "1.1.0": version("qol-shot", "1.1.0", blob("sha256:t1", 1), json!({})),
                    "1.2.0": version("qol-shot", "1.2.0", blob("sha256:t2", 2), json!({})),
                }
            }
        });
        serde_json::to_vec(&document(serial, "https://ghcr.io", plugins)).unwrap()
    }

    #[test]
    fn verify_accepts_only_the_signed_bytes_under_the_pinned_key() {
        let signer = Signer::new();
        let other = Signer::new();
        let body = two_version_index(7);
        let signature = signer.sign(&body);
        let mut tampered = body.clone();
        tampered[body.len() - 2] = b' ';

        let document = verify(&body, &signature, &signer.public_key()).expect("signed index");
        assert_eq!(document.serial, 7);
        assert_eq!(document.registry.url, "https://ghcr.io");

        let refused: &[(&str, &[u8], String, String)] = &[
            (
                "tampered body",
                &tampered,
                signature.clone(),
                signer.public_key(),
            ),
            ("other key", &body, signature.clone(), other.public_key()),
            (
                "other signature",
                &body,
                other.sign(&body),
                signer.public_key(),
            ),
            (
                "garbage signature",
                &body,
                "not a signature".to_string(),
                signer.public_key(),
            ),
        ];
        for (case, bytes, signature, key) in refused {
            assert!(
                verify(bytes, signature, key).is_err(),
                "{case} must be refused"
            );
        }
    }

    #[test]
    fn verify_refuses_a_signed_index_with_an_unknown_schema() {
        let signer = Signer::new();
        let mut value = document(1, "https://ghcr.io", json!({}));
        value["schema"] = json!(2);
        let body = serde_json::to_vec(&value).unwrap();
        let error = verify(&body, &signer.sign(&body), &signer.public_key()).unwrap_err();
        assert!(error.to_string().contains("schema 2"), "{error:#}");
    }

    #[test]
    fn release_picks_latest_or_the_requested_version() {
        let signer = Signer::new();
        let body = two_version_index(1);
        let document = verify(&body, &signer.sign(&body), &signer.public_key()).unwrap();
        let cases: &[(Option<&str>, &str, &str)] = &[
            (None, "1.2.0", "sha256:t2"),
            (Some("1.1.0"), "1.1.0", "sha256:t1"),
        ];
        for (requested, expected, tree) in cases {
            let release = document.release("qol-shot", *requested).unwrap();
            assert_eq!(release.version, *expected);
            assert_eq!(release.files.tree.digest, *tree);
            assert_eq!(release.files.dir, "shot");
        }
        assert!(document.release("qol-shot", Some("9.9.9")).is_err());
        assert!(document.release("qol-missing", None).is_err());
    }
}
