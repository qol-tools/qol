mod challenge;
#[cfg(any(test, feature = "test-signer"))]
pub mod test_signer;

pub use challenge::BearerChallenge;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDocument {
    pub schema: u32,
    pub serial: u64,
    pub registry: RegistryLocation,
    pub plugins: BTreeMap<String, IndexedPlugin>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryLocation {
    pub url: String,
    pub repository: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedPlugin {
    pub latest: String,
    pub versions: BTreeMap<String, IndexedVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedVersion {
    pub manifest_digest: String,
    pub dir: String,
    pub plugin_toml: String,
    pub tree: Blob,
    pub assets: BTreeMap<String, Blob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub digest: String,
    pub size: u64,
}

pub fn public_key_line(minisign_pub_file: &str) -> &str {
    minisign_pub_file.lines().last().unwrap_or_default().trim()
}

pub fn verify(body: &[u8], signature: &str, public_key: &str) -> Result<IndexDocument> {
    let key = minisign_verify::PublicKey::from_base64(public_key)
        .context("the plugin index public key is invalid")?;
    let signature = minisign_verify::Signature::decode(signature)
        .context("the plugin index signature is unreadable")?;
    let allow_legacy_unhashed_signatures = false;
    key.verify(body, &signature, allow_legacy_unhashed_signatures)
        .context("the plugin index signature does not match")?;
    let document: IndexDocument =
        serde_json::from_slice(body).context("the plugin index is unreadable")?;
    if document.schema != SCHEMA {
        anyhow::bail!("plugin index schema {} is not supported", document.schema);
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_signer::TestSigner;

    fn index(serial: u64) -> IndexDocument {
        let version = IndexedVersion {
            manifest_digest: "sha256:m".to_string(),
            dir: "shot".to_string(),
            plugin_toml: "[plugin]\nid = \"qol-shot\"\n".to_string(),
            tree: Blob {
                digest: "sha256:t".to_string(),
                size: 1,
            },
            assets: BTreeMap::new(),
        };
        IndexDocument {
            schema: SCHEMA,
            serial,
            registry: RegistryLocation {
                url: "https://ghcr.io".to_string(),
                repository: "qol-tools/plugins".to_string(),
            },
            plugins: BTreeMap::from([(
                "qol-shot".to_string(),
                IndexedPlugin {
                    latest: "1.0.0".to_string(),
                    versions: BTreeMap::from([("1.0.0".to_string(), version)]),
                },
            )]),
        }
    }

    #[test]
    fn verify_accepts_only_the_signed_bytes_under_the_pinned_key() {
        let signer = TestSigner::generate();
        let other = TestSigner::generate();
        let body = serde_json::to_vec(&index(7)).unwrap();
        let signature = signer.sign(&body);
        let mut tampered = body.clone();
        tampered[body.len() - 2] = b' ';
        let key = signer.public_key();

        assert_eq!(verify(&body, &signature, &key).unwrap(), index(7));
        let refused: &[(&str, &[u8], String, String)] = &[
            ("tampered body", &tampered, signature.clone(), key.clone()),
            ("other key", &body, signature.clone(), other.public_key()),
            ("other signature", &body, other.sign(&body), key.clone()),
            (
                "garbage signature",
                &body,
                "not a signature".to_string(),
                key.clone(),
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
        let signer = TestSigner::generate();
        let mut document = index(1);
        document.schema = 2;
        let body = serde_json::to_vec(&document).unwrap();
        let error = verify(&body, &signer.sign(&body), &signer.public_key()).unwrap_err();
        assert!(error.to_string().contains("schema 2"), "{error:#}");
    }

    #[test]
    fn public_key_line_skips_the_comment() {
        let file = "untrusted comment: minisign public key ABC\nRWQkey\n";
        assert_eq!(public_key_line(file), "RWQkey");
    }
}
