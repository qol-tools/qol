use crate::artifact::upload;
use crate::oci::{
    Descriptor, Manifest, EMPTY_CONFIG, EMPTY_MEDIA_TYPE, INDEX_ARTIFACT_TYPE, MANIFEST_MEDIA_TYPE,
};
use crate::registry::{read_body, send, Registry};
use anyhow::{Context, Result};
use qol_plugin_index::{IndexDocument, SignedIndex, INDEX_TAG, SIGNED_INDEX_MEDIA_TYPE};
use std::collections::BTreeMap;

pub fn fetch(agent: &ureq::Agent, url: &str, public_key: &str) -> Result<Option<IndexDocument>> {
    let response = send(agent.get(url), None, url)?;
    match response.status() {
        404 => return Ok(None),
        200 => {}
        status => anyhow::bail!("{url} answered {status}"),
    }
    let signed = read_body(response, url)?;
    let document = qol_plugin_index::verify_signed(&signed, public_key)
        .with_context(|| format!("the deployed index at {url} does not verify"))?;
    Ok(Some(document))
}

pub fn push(registry: &Registry, signed: &[u8]) -> Result<u64> {
    let file: SignedIndex =
        serde_json::from_slice(signed).context("the signed index file is unreadable")?;
    let document: IndexDocument =
        serde_json::from_str(&file.index).context("the signed index holds an unreadable index")?;
    let manifest = manifest(signed);
    upload(registry, &manifest, &[EMPTY_CONFIG, signed])?;
    registry.put_manifest(INDEX_TAG, &serde_json::to_vec(&manifest)?)?;
    Ok(document.serial)
}

fn manifest(signed: &[u8]) -> Manifest {
    Manifest {
        schema_version: 2,
        media_type: MANIFEST_MEDIA_TYPE.to_string(),
        artifact_type: INDEX_ARTIFACT_TYPE.to_string(),
        config: Descriptor::of(EMPTY_MEDIA_TYPE, EMPTY_CONFIG),
        layers: vec![Descriptor::of(SIGNED_INDEX_MEDIA_TYPE, signed)],
        annotations: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::sha256_digest;

    #[test]
    fn index_manifest_holds_the_signed_file_under_the_empty_config() {
        let signed = br#"{"signature":"s","index":"{}"}"#;
        let manifest = manifest(signed);
        assert_eq!(manifest.artifact_type, INDEX_ARTIFACT_TYPE);
        assert_eq!(manifest.config.media_type, EMPTY_MEDIA_TYPE);
        assert_eq!(
            manifest.config.digest,
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
        assert_eq!(manifest.config.size, 2);
        assert_eq!(
            manifest.layers,
            [Descriptor {
                media_type: SIGNED_INDEX_MEDIA_TYPE.to_string(),
                digest: sha256_digest(signed),
                size: signed.len() as u64,
                annotations: BTreeMap::new(),
            }]
        );
    }
}
