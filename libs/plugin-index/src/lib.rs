mod challenge;
#[cfg(any(test, feature = "test-signer"))]
pub mod test_signer;

pub use challenge::BearerChallenge;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;
pub const HOST_OSES: [&str; 3] = ["linux", "macos", "windows"];
pub const HOST_KEEP: usize = 2;
pub const INDEX_TAG: &str = "index";
pub const SIGNED_INDEX_MEDIA_TYPE: &str = "application/vnd.qol.plugin-index.signed.v1+json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDocument {
    pub schema: u32,
    pub serial: u64,
    pub registry: RegistryLocation,
    pub plugins: BTreeMap<String, IndexedPlugin>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host: BTreeMap<String, IndexedHost>,
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
pub struct IndexedHost {
    pub latest: String,
    pub versions: BTreeMap<String, IndexedHostVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedHostVersion {
    pub manifest_digest: String,
    pub revision: String,
    pub assets: BTreeMap<String, Blob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedIndex {
    pub signature: String,
    pub index: String,
}

pub fn host_tag(os: &str, version: &str) -> String {
    format!("qol-tray-{os}-v{version}")
}

pub fn public_key_line(minisign_pub_file: &str) -> &str {
    minisign_pub_file.lines().last().unwrap_or_default().trim()
}

pub fn verify_signed(signed: &[u8], public_key: &str) -> Result<IndexDocument> {
    let signed: SignedIndex =
        serde_json::from_slice(signed).context("the signed plugin index is unreadable")?;
    verify(signed.index.as_bytes(), &signed.signature, public_key)
}

fn verify(body: &[u8], signature: &str, public_key: &str) -> Result<IndexDocument> {
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
            host: BTreeMap::new(),
        }
    }

    fn with_host(mut document: IndexDocument) -> IndexDocument {
        let version = IndexedHostVersion {
            manifest_digest: "sha256:h".to_string(),
            revision: "0123456789abcdef0123456789abcdef01234567".to_string(),
            assets: BTreeMap::from([(
                "qol-tray-linux-x86_64.tar.gz".to_string(),
                Blob {
                    digest: "sha256:a".to_string(),
                    size: 2,
                },
            )]),
        };
        document.host.insert(
            "linux".to_string(),
            IndexedHost {
                latest: "3.101.4".to_string(),
                versions: BTreeMap::from([("3.101.4".to_string(), version)]),
            },
        );
        document
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
    fn verify_signed_reads_the_index_and_signature_from_one_file() {
        let signer = TestSigner::generate();
        let body = serde_json::to_vec(&index(3)).unwrap();
        let key = signer.public_key();
        let from_jq = format!(
            "{{\n  \"signature\": {},\n  \"index\": {}\n}}\n",
            serde_json::to_string(&signer.sign(&body)).unwrap(),
            serde_json::to_string(std::str::from_utf8(&body).unwrap()).unwrap(),
        );
        let mut swapped: SignedIndex = serde_json::from_slice(&signer.sign_index(&body)).unwrap();
        swapped.index = serde_json::to_string(&index(4)).unwrap();

        assert_eq!(
            verify_signed(&signer.sign_index(&body), &key).unwrap(),
            index(3)
        );
        assert_eq!(verify_signed(from_jq.as_bytes(), &key).unwrap(), index(3));
        let refused: &[(&str, Vec<u8>, &str)] = &[
            (
                "index swapped under the signature",
                serde_json::to_vec(&swapped).unwrap(),
                "does not match",
            ),
            ("bare index", body.clone(), "unreadable"),
        ];
        for (case, bytes, cause) in refused {
            let error = verify_signed(bytes, &key).unwrap_err();
            assert!(format!("{error:#}").contains(cause), "{case}: {error:#}");
        }
    }

    #[test]
    fn host_entries_are_optional_and_absent_ones_leave_the_bytes_unchanged() {
        let plugins_only = concat!(
            r#"{"schema":1,"serial":7,"registry":{"url":"https://ghcr.io","repository":"qol-tools/plugins"},"#,
            r#""plugins":{"qol-shot":{"latest":"1.0.0","versions":{"1.0.0":{"manifest_digest":"sha256:m","#,
            r#""dir":"shot","plugin_toml":"[plugin]\nid = \"qol-shot\"\n","tree":{"digest":"sha256:t","size":1},"#,
            r#""assets":{}}}}}}"#
        );
        assert_eq!(serde_json::to_string(&index(7)).unwrap(), plugins_only);
        let cases: &[(&str, IndexDocument)] = &[
            ("plugins only", index(7)),
            ("with a host entry", with_host(index(7))),
        ];
        for (case, document) in cases {
            let bytes = serde_json::to_vec(document).unwrap();
            let read: IndexDocument = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(&read, document, "{case}");
        }
        let written = serde_json::to_string(&with_host(index(7))).unwrap();
        assert!(
            written.contains(r#""host":{"linux":{"latest":"3.101.4""#),
            "{written}"
        );
    }

    #[test]
    fn verify_signed_accepts_host_entries() {
        let signer = TestSigner::generate();
        let body = serde_json::to_vec(&with_host(index(5))).unwrap();
        assert_eq!(
            verify_signed(&signer.sign_index(&body), &signer.public_key()).unwrap(),
            with_host(index(5))
        );
    }

    #[test]
    fn host_tags_name_the_os_and_version() {
        let cases: &[(&str, &str, &str)] = &[
            ("linux", "3.101.4", "qol-tray-linux-v3.101.4"),
            ("macos", "0.1.0", "qol-tray-macos-v0.1.0"),
            ("windows", "10.0.2", "qol-tray-windows-v10.0.2"),
        ];
        for (os, version, tag) in cases {
            assert_eq!(host_tag(os, version), *tag);
        }
    }

    #[test]
    fn public_key_line_skips_the_comment() {
        let file = "untrusted comment: minisign public key ABC\nRWQkey\n";
        assert_eq!(public_key_line(file), "RWQkey");
    }
}
