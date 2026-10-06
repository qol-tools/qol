use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const ARTIFACT_TYPE: &str = "application/vnd.qol.plugin.v1";
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.qol.plugin.manifest.v1+toml";
pub const TREE_MEDIA_TYPE: &str = "application/vnd.qol.plugin.tree.v1.tar+gzip";
pub const ASSET_MEDIA_TYPE: &str = "application/octet-stream";
pub const TITLE_ANNOTATION: &str = "org.opencontainers.image.title";
pub const DIR_ANNOTATION: &str = "dev.qol.plugin.dir";
pub const SOURCE_ANNOTATION: &str = "org.opencontainers.image.source";
pub const REVISION_ANNOTATION: &str = "org.opencontainers.image.revision";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub media_type: String,
    pub artifact_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

impl Descriptor {
    pub fn of(media_type: &str, bytes: &[u8]) -> Self {
        Self {
            media_type: media_type.to_string(),
            digest: sha256_digest(bytes),
            size: bytes.len() as u64,
            annotations: BTreeMap::new(),
        }
    }

    pub fn titled(mut self, title: &str) -> Self {
        self.annotations
            .insert(TITLE_ANNOTATION.to_string(), title.to_string());
        self
    }

    pub fn title(&self) -> Option<&str> {
        self.annotations.get(TITLE_ANNOTATION).map(String::as_str)
    }
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn is_sha256_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_digests_are_lowercase_hex_of_64_chars() {
        let digest = sha256_digest(b"abc");
        assert_eq!(
            digest,
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let cases: &[(&str, bool)] = &[
            (digest.as_str(), true),
            ("sha256:t", false),
            (&digest.to_uppercase(), false),
            ("sha512:00", false),
        ];
        for (candidate, valid) in cases {
            assert_eq!(is_sha256_digest(candidate), *valid, "{candidate}");
        }
    }
}
