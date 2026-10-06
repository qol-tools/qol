use crate::oci::{is_sha256_digest, Manifest, CONFIG_MEDIA_TYPE, DIR_ANNOTATION, TREE_MEDIA_TYPE};
use crate::parallel::parallel_map;
use crate::registry::Registry;
use crate::release::{declared_id, declared_version};
use crate::release::{newest_per_plugin, ReleaseTag};
use anyhow::{Context, Result};
use qol_plugin_index::{
    Blob, IndexDocument, IndexedPlugin, IndexedVersion, RegistryLocation, SCHEMA,
};
use std::collections::{BTreeMap, BTreeSet};

const FETCH_WORKERS: usize = 8;

pub struct Built {
    pub document: IndexDocument,
    pub reused: usize,
    pub fetched: usize,
}

pub fn build(
    registry: &Registry,
    previous: Option<&IndexDocument>,
    keep: usize,
    now: u64,
) -> Result<Built> {
    let tags = registry.tags()?;
    let retained = newest_per_plugin(tags.iter().map(String::as_str), keep);
    let reusable = previous.filter(|previous| previous.registry == *registry.location());
    let listed = |release: &ReleaseTag| {
        reusable?
            .plugins
            .get(&release.plugin_id)?
            .versions
            .get(&release.version)
            .cloned()
    };
    let (reused, missing): (Vec<_>, Vec<_>) = retained
        .into_iter()
        .map(|release| {
            let entry = listed(&release);
            (release, entry)
        })
        .partition(|(_, entry)| entry.is_some());
    let missing: Vec<ReleaseTag> = missing.into_iter().map(|(release, _)| release).collect();
    let fetched = parallel_map(&missing, FETCH_WORKERS, |release| {
        fetch_version(registry, release)
    })?;
    let counts = (reused.len(), fetched.len());
    let versions = reused
        .into_iter()
        .filter_map(|(release, entry)| Some((release, entry?)))
        .chain(missing.into_iter().zip(fetched));
    let document = assemble(
        versions,
        registry.location().clone(),
        next_serial(previous, now),
    );
    Ok(Built {
        document,
        reused: counts.0,
        fetched: counts.1,
    })
}

pub fn changes(previous: Option<&IndexDocument>, index: &IndexDocument) -> Vec<String> {
    let listed = |document: &IndexDocument| -> BTreeSet<(String, String)> {
        document
            .plugins
            .iter()
            .flat_map(|(plugin_id, plugin)| {
                plugin
                    .versions
                    .keys()
                    .map(|version| (plugin_id.clone(), version.clone()))
            })
            .collect()
    };
    let before = previous.map(listed).unwrap_or_default();
    let after = listed(index);
    let added = after
        .difference(&before)
        .map(|(plugin_id, version)| format!("+ {plugin_id} {version}"));
    let removed = before
        .difference(&after)
        .map(|(plugin_id, version)| format!("- {plugin_id} {version}"));
    added.chain(removed).collect()
}

fn next_serial(previous: Option<&IndexDocument>, now: u64) -> u64 {
    now.max(previous.map_or(0, |previous| previous.serial + 1))
}

fn assemble(
    versions: impl Iterator<Item = (ReleaseTag, IndexedVersion)>,
    registry: RegistryLocation,
    serial: u64,
) -> IndexDocument {
    let mut plugins: BTreeMap<String, (ReleaseTag, IndexedPlugin)> = BTreeMap::new();
    for (release, entry) in versions {
        let (newest, plugin) = plugins.entry(release.plugin_id.clone()).or_insert_with(|| {
            let plugin = IndexedPlugin {
                latest: release.version.clone(),
                versions: BTreeMap::new(),
            };
            (release.clone(), plugin)
        });
        if release.number > newest.number {
            plugin.latest = release.version.clone();
            *newest = release.clone();
        }
        plugin.versions.insert(release.version, entry);
    }
    IndexDocument {
        schema: SCHEMA,
        serial,
        registry,
        plugins: plugins
            .into_iter()
            .map(|(plugin_id, (_, plugin))| (plugin_id, plugin))
            .collect(),
    }
}

fn fetch_version(registry: &Registry, release: &ReleaseTag) -> Result<IndexedVersion> {
    let fetched = registry
        .manifest(&release.tag)?
        .with_context(|| format!("{} is tagged but its manifest is missing", release.tag))?;
    let manifest: Manifest = serde_json::from_slice(&fetched.bytes)
        .with_context(|| format!("{} has an unreadable manifest", release.tag))?;
    if manifest.config.media_type != CONFIG_MEDIA_TYPE {
        anyhow::bail!(
            "{}: config media type is {:?}",
            release.tag,
            manifest.config.media_type
        );
    }
    let config = registry.blob(&manifest.config.digest, manifest.config.size)?;
    let plugin_toml = String::from_utf8(config)
        .with_context(|| format!("{}: plugin.toml is not UTF-8", release.tag))?;
    if declared_id(&plugin_toml).as_deref() != Some(release.plugin_id.as_str())
        || declared_version(&plugin_toml).as_deref() != Some(release.version.as_str())
    {
        anyhow::bail!("{}: plugin.toml names another release", release.tag);
    }
    version_entry(&fetched.digest, &manifest, plugin_toml)
}

fn version_entry(
    manifest_digest: &str,
    manifest: &Manifest,
    plugin_toml: String,
) -> Result<IndexedVersion> {
    let mut trees = Vec::new();
    let mut assets = BTreeMap::new();
    for layer in &manifest.layers {
        if !is_sha256_digest(&layer.digest) {
            anyhow::bail!("{manifest_digest}: invalid layer digest {:?}", layer.digest);
        }
        let blob = Blob {
            digest: layer.digest.clone(),
            size: layer.size,
        };
        if layer.media_type == TREE_MEDIA_TYPE {
            trees.push(blob);
            continue;
        }
        let title = layer
            .title()
            .filter(|title| !title.is_empty())
            .with_context(|| format!("{manifest_digest}: an asset layer has no title"))?;
        if assets.insert(title.to_string(), blob).is_some() {
            anyhow::bail!("{manifest_digest}: asset {title:?} appears twice");
        }
    }
    let [tree] = <[Blob; 1]>::try_from(trees).map_err(|trees| {
        anyhow::anyhow!(
            "{manifest_digest}: expected one tree layer, found {}",
            trees.len()
        )
    })?;
    let dir = manifest
        .annotations
        .get(DIR_ANNOTATION)
        .filter(|dir| !dir.is_empty() && !dir.contains('/') && !dir.starts_with('.'))
        .with_context(|| format!("{manifest_digest}: invalid {DIR_ANNOTATION} annotation"))?;
    Ok(IndexedVersion {
        manifest_digest: manifest_digest.to_string(),
        dir: dir.clone(),
        plugin_toml,
        tree,
        assets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::{sha256_digest, Descriptor, ASSET_MEDIA_TYPE};

    fn sha(label: &str) -> String {
        sha256_digest(label.as_bytes())
    }

    fn layer(media_type: &str, digest: &str, title: Option<&str>) -> Descriptor {
        let descriptor = Descriptor {
            media_type: media_type.to_string(),
            digest: digest.to_string(),
            size: 10,
            annotations: BTreeMap::new(),
        };
        match title {
            Some(title) => descriptor.titled(title),
            None => descriptor,
        }
    }

    fn manifest(layers: Vec<Descriptor>, dir: &str) -> Manifest {
        Manifest {
            schema_version: 2,
            media_type: String::new(),
            artifact_type: String::new(),
            config: layer(CONFIG_MEDIA_TYPE, &sha("config"), None),
            layers,
            annotations: BTreeMap::from([(DIR_ANNOTATION.to_string(), dir.to_string())]),
        }
    }

    fn entry(label: &str) -> IndexedVersion {
        version_entry(
            &sha(label),
            &manifest(vec![layer(TREE_MEDIA_TYPE, &sha("t"), None)], "shot"),
            String::new(),
        )
        .unwrap()
    }

    #[test]
    fn splits_the_tree_from_titled_assets() {
        let entry = version_entry(
            &sha("m"),
            &manifest(
                vec![
                    layer(TREE_MEDIA_TYPE, &sha("t"), None),
                    layer(
                        ASSET_MEDIA_TYPE,
                        &sha("a"),
                        Some("qol-alt-tab-linux-x86_64"),
                    ),
                    layer(ASSET_MEDIA_TYPE, &sha("o"), Some("IBM-Plex-OFL.txt")),
                ],
                "alt-tab",
            ),
            "toml".to_string(),
        )
        .unwrap();
        assert_eq!(entry.manifest_digest, sha("m"));
        assert_eq!(entry.dir, "alt-tab");
        assert_eq!(entry.tree.digest, sha("t"));
        assert_eq!(
            entry.assets.keys().collect::<Vec<_>>(),
            ["IBM-Plex-OFL.txt", "qol-alt-tab-linux-x86_64"]
        );
    }

    #[test]
    fn refuses_malformed_manifests() {
        let tree = || layer(TREE_MEDIA_TYPE, &sha("t"), None);
        let cases: Vec<(&str, Manifest)> = vec![
            (
                "no tree",
                manifest(vec![layer(ASSET_MEDIA_TYPE, &sha("a"), Some("a"))], "shot"),
            ),
            (
                "two trees",
                manifest(
                    vec![tree(), layer(TREE_MEDIA_TYPE, &sha("t2"), None)],
                    "shot",
                ),
            ),
            (
                "short digest",
                manifest(vec![layer(TREE_MEDIA_TYPE, "sha256:t", None)], "shot"),
            ),
            (
                "untitled asset",
                manifest(
                    vec![tree(), layer(ASSET_MEDIA_TYPE, &sha("a"), None)],
                    "shot",
                ),
            ),
            (
                "repeated title",
                manifest(
                    vec![
                        tree(),
                        layer(ASSET_MEDIA_TYPE, &sha("a"), Some("bin")),
                        layer(ASSET_MEDIA_TYPE, &sha("b"), Some("bin")),
                    ],
                    "shot",
                ),
            ),
            ("empty dir", manifest(vec![tree()], "")),
            ("nested dir", manifest(vec![tree()], "plugins/shot")),
            ("parent dir", manifest(vec![tree()], "..")),
            ("hidden dir", manifest(vec![tree()], ".hidden")),
        ];
        for (case, manifest) in cases {
            assert!(
                version_entry(&sha("m"), &manifest, String::new()).is_err(),
                "{case}"
            );
        }
    }

    #[test]
    fn assembles_newest_by_number_as_latest() {
        let versions = [
            "qol-alt-tab-v0.9.0",
            "qol-alt-tab-v0.10.0",
            "qol-shot-v1.0.0",
        ]
        .into_iter()
        .map(|tag| (ReleaseTag::parse(tag).unwrap(), entry(tag)));
        let location = RegistryLocation {
            url: "https://ghcr.io".to_string(),
            repository: "qol-tools/plugins".to_string(),
        };
        let index = assemble(versions, location.clone(), 7);
        assert_eq!(index.schema, SCHEMA);
        assert_eq!(index.serial, 7);
        assert_eq!(index.registry, location);
        assert_eq!(index.plugins["qol-alt-tab"].latest, "0.10.0");
        assert_eq!(index.plugins["qol-alt-tab"].versions.len(), 2);
        assert_eq!(index.plugins["qol-shot"].latest, "1.0.0");
        assert!(assemble(std::iter::empty(), location, 1).plugins.is_empty());
    }

    #[test]
    fn serial_always_moves_forward() {
        let location = RegistryLocation {
            url: "u".to_string(),
            repository: "r".to_string(),
        };
        let previous = assemble(std::iter::empty(), location, 500);
        let cases: &[(Option<&IndexDocument>, u64, u64)] = &[
            (None, 100, 100),
            (Some(&previous), 100, 501),
            (Some(&previous), 900, 900),
        ];
        for (previous, now, expected) in cases {
            assert_eq!(next_serial(*previous, *now), *expected);
        }
    }

    #[test]
    fn changes_list_added_then_removed_versions() {
        let location = RegistryLocation {
            url: "u".to_string(),
            repository: "r".to_string(),
        };
        let build = |tags: &[&str]| {
            assemble(
                tags.iter()
                    .map(|tag| (ReleaseTag::parse(tag).unwrap(), entry(tag))),
                location.clone(),
                1,
            )
        };
        let previous = build(&["qol-shot-v0.9.0", "qol-shot-v1.0.0"]);
        let index = build(&["qol-shot-v1.0.0", "qol-launcher-v2.0.0"]);
        assert_eq!(
            changes(Some(&previous), &index),
            ["+ qol-launcher 2.0.0", "- qol-shot 0.9.0"]
        );
    }
}
