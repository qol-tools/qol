use crate::git;
use crate::oci::{
    sha256_digest, Descriptor, Manifest, ARTIFACT_TYPE, ASSET_MEDIA_TYPE, CONFIG_MEDIA_TYPE,
    DIR_ANNOTATION, MANIFEST_MEDIA_TYPE, REVISION_ANNOTATION, SOURCE_ANNOTATION, TREE_MEDIA_TYPE,
};
use crate::parallel::parallel_map;
use crate::registry::Registry;
use crate::release::ReleaseTag;
use anyhow::{Context, Result};
use flate2::{Compression, GzBuilder};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

const UPLOAD_WORKERS: usize = 4;

pub struct Release {
    pub tag: String,
    pub dir: String,
    pub commit: String,
    pub config: Vec<u8>,
    pub tree: Vec<u8>,
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Pushed {
    New,
    AlreadyThere,
}

pub fn read_assets(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut assets = BTreeMap::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("{} has no UTF-8 name", path.display()))?
            .to_string();
        assets.insert(name, std::fs::read(&path)?);
    }
    Ok(assets)
}

pub fn release_from_git(
    tag: &str,
    revision: &str,
    assets: BTreeMap<String, Vec<u8>>,
) -> Result<Release> {
    let parsed = ReleaseTag::parse(tag)
        .with_context(|| format!("tag {tag:?} must match <plugin-id>-vX.Y.Z"))?;
    let plugin = git::plugin_at(revision, &parsed.plugin_id)?;
    if crate::release::declared_version(&plugin.plugin_toml).as_deref()
        != Some(parsed.version.as_str())
    {
        anyhow::bail!(
            "{revision}: plugins/{}/plugin.toml is not version {}",
            plugin.dir,
            parsed.version
        );
    }
    let tree = gzip(&git::tree_tar(&plugin.commit, &plugin.dir)?)?;
    Ok(Release {
        tag: tag.to_string(),
        dir: plugin.dir,
        commit: plugin.commit,
        config: plugin.plugin_toml.into_bytes(),
        tree,
        assets,
    })
}

pub fn manifest(release: &Release, source: &str) -> Manifest {
    let mut layers = vec![Descriptor::of(TREE_MEDIA_TYPE, &release.tree)];
    layers.extend(
        release
            .assets
            .iter()
            .map(|(name, bytes)| Descriptor::of(ASSET_MEDIA_TYPE, bytes).titled(name)),
    );
    Manifest {
        schema_version: 2,
        media_type: MANIFEST_MEDIA_TYPE.to_string(),
        artifact_type: ARTIFACT_TYPE.to_string(),
        config: Descriptor::of(CONFIG_MEDIA_TYPE, &release.config),
        layers,
        annotations: BTreeMap::from([
            (DIR_ANNOTATION.to_string(), release.dir.clone()),
            (REVISION_ANNOTATION.to_string(), release.commit.clone()),
            (SOURCE_ANNOTATION.to_string(), source.to_string()),
        ]),
    }
}

pub fn push(registry: &Registry, release: &Release, source: &str) -> Result<Pushed> {
    let manifest = manifest(release, source);
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    if let Some(published) = registry.manifest(&release.tag)? {
        if published.digest == sha256_digest(&manifest_bytes) {
            return Ok(Pushed::AlreadyThere);
        }
        anyhow::bail!(
            "{} already holds different content; a published release is never overwritten",
            release.tag
        );
    }
    let blobs = [release.config.as_slice(), release.tree.as_slice()]
        .into_iter()
        .chain(release.assets.values().map(Vec::as_slice));
    let uploads: Vec<(&str, &[u8])> = std::iter::once(&manifest.config)
        .chain(&manifest.layers)
        .map(|descriptor| descriptor.digest.as_str())
        .zip(blobs)
        .collect();
    parallel_map(&uploads, UPLOAD_WORKERS, |(digest, bytes)| {
        if !registry.has_blob(digest)? {
            registry.upload_blob(digest, bytes)?;
        }
        Ok(())
    })?;
    registry.put_manifest(&release.tag, &manifest_bytes)?;
    Ok(Pushed::New)
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::TITLE_ANNOTATION;

    fn release() -> Release {
        Release {
            tag: "qol-shot-v1.0.0".to_string(),
            dir: "shot".to_string(),
            commit: "abc123".to_string(),
            config: b"[plugin]\nid = \"qol-shot\"\n".to_vec(),
            tree: gzip(b"tree").unwrap(),
            assets: BTreeMap::from([
                ("qol-shot-macos-aarch64".to_string(), b"mac".to_vec()),
                ("IBM-Plex-OFL.txt".to_string(), b"ofl".to_vec()),
                ("qol-shot-linux-x86_64".to_string(), b"linux".to_vec()),
            ]),
        }
    }

    #[test]
    fn manifest_leads_with_the_tree_and_titles_assets_in_name_order() {
        let bytes = serde_json::to_vec(&manifest(&release(), "https://github.com/o/qol")).unwrap();
        let parsed: Manifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.artifact_type, ARTIFACT_TYPE);
        assert_eq!(parsed.config.media_type, CONFIG_MEDIA_TYPE);
        assert_eq!(parsed.annotations[DIR_ANNOTATION], "shot");
        assert_eq!(parsed.annotations[REVISION_ANNOTATION], "abc123");
        assert_eq!(
            parsed.annotations[SOURCE_ANNOTATION],
            "https://github.com/o/qol"
        );
        let layers: Vec<(&str, Option<&str>)> = parsed
            .layers
            .iter()
            .map(|layer| {
                (
                    layer.media_type.as_str(),
                    layer.annotations.get(TITLE_ANNOTATION).map(String::as_str),
                )
            })
            .collect();
        assert_eq!(
            layers,
            [
                (TREE_MEDIA_TYPE, None),
                (ASSET_MEDIA_TYPE, Some("IBM-Plex-OFL.txt")),
                (ASSET_MEDIA_TYPE, Some("qol-shot-linux-x86_64")),
                (ASSET_MEDIA_TYPE, Some("qol-shot-macos-aarch64")),
            ]
        );
    }

    #[test]
    fn the_same_release_always_yields_the_same_manifest_bytes() {
        let bytes = |release: &Release| serde_json::to_vec(&manifest(release, "s")).unwrap();
        let first = bytes(&release());
        let mut rebuilt = release();
        rebuilt.tree = gzip(b"tree").unwrap();
        assert_eq!(first, bytes(&rebuilt));
        let mut other = release();
        other
            .assets
            .insert("qol-shot-linux-x86_64".to_string(), b"other".to_vec());
        assert_ne!(first, bytes(&other));
    }
}
