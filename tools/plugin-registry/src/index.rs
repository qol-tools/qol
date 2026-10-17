use crate::host::HostConfig;
use crate::oci::{
    is_sha256_digest, Descriptor, Manifest, ASSET_MEDIA_TYPE, CONFIG_MEDIA_TYPE, DIR_ANNOTATION,
    HOST_CONFIG_MEDIA_TYPE, TREE_MEDIA_TYPE,
};
use crate::parallel::parallel_map;
use crate::provenance::{verify_assets, Provenance};
use crate::registry::Registry;
use crate::release::{declared_id, declared_version};
use crate::release::{newest_per_plugin, ReleaseTag};
use anyhow::{Context, Result};
use qol_plugin_index::{
    Blob, IndexDocument, IndexedHost, IndexedHostVersion, IndexedPlugin, IndexedVersion,
    RegistryLocation, HOST_KEEP, SCHEMA,
};
use std::collections::{BTreeMap, BTreeSet};

const FETCH_WORKERS: usize = 8;

pub struct Built {
    pub document: IndexDocument,
    pub reused: usize,
    pub fetched: usize,
    pub warnings: Vec<String>,
}

struct Gathered<T> {
    versions: Vec<(ReleaseTag, T)>,
    reused: usize,
    fetched: usize,
}

type HostOutcome = std::result::Result<IndexedHostVersion, String>;

pub fn build(
    registry: &Registry,
    previous: Option<&IndexDocument>,
    keep: usize,
    now: u64,
    provenance: Option<&dyn Provenance>,
) -> Result<Built> {
    let tags = registry.tags()?;
    let (plugin_tags, host_tags) = split_tags(&tags, keep);
    let reusable = previous.filter(|previous| previous.registry == *registry.location());
    let plugins = gather(
        plugin_tags,
        |release| {
            reusable?
                .plugins
                .get(&release.plugin_id)?
                .versions
                .get(&release.version)
                .cloned()
        },
        |release| fetch_version(registry, release),
    )?;
    let hosts = gather(
        host_tags,
        |release| {
            let entry = reusable?
                .host
                .get(release.host_system()?)?
                .versions
                .get(&release.version)?;
            Some(Ok(entry.clone()))
        },
        |release| {
            Ok(fetch_host_version(registry, release, provenance)
                .map_err(|error| format!("{} is left out of the index: {error:#}", release.tag)))
        },
    )?;
    let (host, warnings) = assemble_host(hosts.versions);
    let document = IndexDocument {
        host,
        ..assemble(
            plugins.versions.into_iter(),
            registry.location().clone(),
            next_serial(previous, now),
        )
    };
    Ok(Built {
        document,
        reused: plugins.reused + hosts.reused,
        fetched: plugins.fetched + hosts.fetched,
        warnings,
    })
}

pub fn changes(previous: Option<&IndexDocument>, index: &IndexDocument) -> Vec<String> {
    let listed = |document: &IndexDocument| -> BTreeSet<(String, String)> {
        let plugins = document.plugins.iter().flat_map(|(plugin_id, plugin)| {
            plugin
                .versions
                .keys()
                .map(|version| (plugin_id.clone(), version.clone()))
        });
        let hosts = document.host.iter().flat_map(|(os, host)| {
            host.versions
                .keys()
                .map(move |version| (format!("host {os}"), version.clone()))
        });
        plugins.chain(hosts).collect()
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

fn split_tags(tags: &[String], keep: usize) -> (Vec<ReleaseTag>, Vec<ReleaseTag>) {
    let (host_tags, plugin_tags): (Vec<&str>, Vec<&str>) = tags
        .iter()
        .map(String::as_str)
        .partition(|tag| ReleaseTag::parse(tag).is_some_and(|release| release.is_host()));
    let hosts = newest_per_plugin(host_tags, HOST_KEEP)
        .into_iter()
        .filter(|release| release.host_system().is_some())
        .collect();
    (newest_per_plugin(plugin_tags, keep), hosts)
}

fn gather<T: Send>(
    retained: Vec<ReleaseTag>,
    listed: impl Fn(&ReleaseTag) -> Option<T>,
    fetch: impl Fn(&ReleaseTag) -> Result<T> + Sync,
) -> Result<Gathered<T>> {
    let (reused, missing): (Vec<_>, Vec<_>) = retained
        .into_iter()
        .map(|release| {
            let entry = listed(&release);
            (release, entry)
        })
        .partition(|(_, entry)| entry.is_some());
    let missing: Vec<ReleaseTag> = missing.into_iter().map(|(release, _)| release).collect();
    let fetched = parallel_map(&missing, FETCH_WORKERS, fetch)?;
    Ok(Gathered {
        reused: reused.len(),
        fetched: fetched.len(),
        versions: reused
            .into_iter()
            .filter_map(|(release, entry)| Some((release, entry?)))
            .chain(missing.into_iter().zip(fetched))
            .collect(),
    })
}

fn next_serial(previous: Option<&IndexDocument>, now: u64) -> u64 {
    now.max(previous.map_or(0, |previous| previous.serial + 1))
}

fn grouped<T>(
    versions: impl Iterator<Item = (ReleaseTag, T)>,
    key: impl Fn(&ReleaseTag) -> String,
) -> BTreeMap<String, (String, BTreeMap<String, T>)> {
    let mut groups: BTreeMap<String, (ReleaseTag, BTreeMap<String, T>)> = BTreeMap::new();
    for (release, entry) in versions {
        let (newest, group) = groups
            .entry(key(&release))
            .or_insert_with(|| (release.clone(), BTreeMap::new()));
        if release.number > newest.number {
            *newest = release.clone();
        }
        group.insert(release.version, entry);
    }
    groups
        .into_iter()
        .map(|(key, (newest, versions))| (key, (newest.version, versions)))
        .collect()
}

fn assemble(
    versions: impl Iterator<Item = (ReleaseTag, IndexedVersion)>,
    registry: RegistryLocation,
    serial: u64,
) -> IndexDocument {
    IndexDocument {
        schema: SCHEMA,
        serial,
        registry,
        plugins: grouped(versions, |release| release.plugin_id.clone())
            .into_iter()
            .map(|(plugin_id, (latest, versions))| (plugin_id, IndexedPlugin { latest, versions }))
            .collect(),
        host: BTreeMap::new(),
    }
}

fn assemble_host(
    outcomes: Vec<(ReleaseTag, HostOutcome)>,
) -> (BTreeMap<String, IndexedHost>, Vec<String>) {
    let mut versions = Vec::new();
    let mut warnings = Vec::new();
    for (release, outcome) in outcomes {
        match outcome {
            Ok(entry) => versions.push((release, entry)),
            Err(warning) => warnings.push(warning),
        }
    }
    let host = grouped(versions.into_iter(), |release| {
        release.host_system().unwrap_or_default().to_string()
    })
    .into_iter()
    .map(|(os, (latest, versions))| (os, IndexedHost { latest, versions }))
    .collect();
    (host, warnings)
}

fn fetch_manifest(
    registry: &Registry,
    release: &ReleaseTag,
    config_media_type: &str,
) -> Result<(String, Manifest, Vec<u8>)> {
    let fetched = registry
        .manifest(&release.tag)?
        .with_context(|| format!("{} is tagged but its manifest is missing", release.tag))?;
    let manifest: Manifest = serde_json::from_slice(&fetched.bytes)
        .with_context(|| format!("{} has an unreadable manifest", release.tag))?;
    if manifest.config.media_type != config_media_type {
        anyhow::bail!(
            "{}: config media type is {:?}",
            release.tag,
            manifest.config.media_type
        );
    }
    let config = registry.blob(&manifest.config.digest, manifest.config.size)?;
    Ok((fetched.digest, manifest, config))
}

fn fetch_version(registry: &Registry, release: &ReleaseTag) -> Result<IndexedVersion> {
    let (manifest_digest, manifest, config) = fetch_manifest(registry, release, CONFIG_MEDIA_TYPE)?;
    let plugin_toml = String::from_utf8(config)
        .with_context(|| format!("{}: plugin.toml is not UTF-8", release.tag))?;
    if declared_id(&plugin_toml).as_deref() != Some(release.plugin_id.as_str())
        || declared_version(&plugin_toml).as_deref() != Some(release.version.as_str())
    {
        anyhow::bail!("{}: plugin.toml names another release", release.tag);
    }
    version_entry(&manifest_digest, &manifest, plugin_toml)
}

fn fetch_host_version(
    registry: &Registry,
    release: &ReleaseTag,
    provenance: Option<&dyn Provenance>,
) -> Result<IndexedHostVersion> {
    let (manifest_digest, manifest, config) =
        fetch_manifest(registry, release, HOST_CONFIG_MEDIA_TYPE)?;
    let entry = host_version_entry(release, &manifest_digest, &manifest, &config)?;
    if let Some(provenance) = provenance {
        verify_assets(
            &entry,
            |blob| registry.blob(&blob.digest, blob.size),
            provenance,
        )?;
    }
    Ok(entry)
}

fn version_entry(
    manifest_digest: &str,
    manifest: &Manifest,
    plugin_toml: String,
) -> Result<IndexedVersion> {
    let (trees, assets): (Vec<&Descriptor>, Vec<&Descriptor>) = manifest
        .layers
        .iter()
        .partition(|layer| layer.media_type == TREE_MEDIA_TYPE);
    let trees = trees
        .into_iter()
        .map(|layer| layer_blob(manifest_digest, layer))
        .collect::<Result<Vec<_>>>()?;
    let assets = titled_assets(manifest_digest, assets)?;
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

fn host_version_entry(
    release: &ReleaseTag,
    manifest_digest: &str,
    manifest: &Manifest,
    config: &[u8],
) -> Result<IndexedHostVersion> {
    let config: HostConfig = serde_json::from_slice(config)
        .with_context(|| format!("{}: the host config is unreadable", release.tag))?;
    let tag = config
        .tag()
        .with_context(|| format!("{}: the host config is invalid", release.tag))?;
    if tag != release.tag {
        anyhow::bail!("{}: the host config names {tag}", release.tag);
    }
    if let Some(layer) = manifest
        .layers
        .iter()
        .find(|layer| layer.media_type != ASSET_MEDIA_TYPE)
    {
        anyhow::bail!(
            "{manifest_digest}: unexpected layer media type {:?}",
            layer.media_type
        );
    }
    let assets = titled_assets(manifest_digest, &manifest.layers)?;
    if assets.is_empty() {
        anyhow::bail!("{manifest_digest}: a host build has no files");
    }
    Ok(IndexedHostVersion {
        manifest_digest: manifest_digest.to_string(),
        revision: config.revision,
        assets,
    })
}

fn layer_blob(manifest_digest: &str, layer: &Descriptor) -> Result<Blob> {
    if !is_sha256_digest(&layer.digest) {
        anyhow::bail!("{manifest_digest}: invalid layer digest {:?}", layer.digest);
    }
    Ok(Blob {
        digest: layer.digest.clone(),
        size: layer.size,
    })
}

fn titled_assets<'a>(
    manifest_digest: &str,
    layers: impl IntoIterator<Item = &'a Descriptor>,
) -> Result<BTreeMap<String, Blob>> {
    let mut assets = BTreeMap::new();
    for layer in layers {
        let blob = layer_blob(manifest_digest, layer)?;
        let title = layer
            .title()
            .filter(|title| !title.is_empty())
            .with_context(|| format!("{manifest_digest}: an asset layer has no title"))?;
        if assets.insert(title.to_string(), blob).is_some() {
            anyhow::bail!("{manifest_digest}: asset {title:?} appears twice");
        }
    }
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::sha256_digest;
    use qol_platform::Os;

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

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn host_entry(label: &str) -> IndexedHostVersion {
        IndexedHostVersion {
            manifest_digest: sha(label),
            revision: COMMIT.to_string(),
            assets: BTreeMap::new(),
        }
    }

    fn host_config(os: &str, version: &str, revision: &str) -> Vec<u8> {
        serde_json::to_vec(&HostConfig {
            os: os.to_string(),
            version: version.to_string(),
            revision: revision.to_string(),
        })
        .unwrap()
    }

    fn host_manifest(layers: Vec<Descriptor>) -> Manifest {
        Manifest {
            config: layer(HOST_CONFIG_MEDIA_TYPE, &sha("config"), None),
            annotations: BTreeMap::new(),
            ..manifest(layers, "")
        }
    }

    #[test]
    fn split_tags_routes_host_tags_and_ignores_the_index_tag() {
        let tags: Vec<String> = [
            "qol-shot-v1.0.0",
            "qol-shot-v0.9.0",
            "qol-tray-linux-v3.101.4",
            "qol-tray-linux-v3.100.0",
            "qol-tray-linux-v3.101.3",
            "qol-tray-macos-v3.101.4",
            "qol-tray-freebsd-v1.0.0",
            "index",
            "sha256-abc.sig",
        ]
        .map(str::to_string)
        .into();
        let (plugins, hosts) = split_tags(&tags, 1);
        let tags_of = |releases: Vec<ReleaseTag>| -> Vec<String> {
            releases.into_iter().map(|release| release.tag).collect()
        };
        assert_eq!(tags_of(plugins), ["qol-shot-v1.0.0"]);
        assert_eq!(
            tags_of(hosts),
            [
                "qol-tray-linux-v3.101.4",
                "qol-tray-linux-v3.101.3",
                "qol-tray-macos-v3.101.4"
            ]
        );
    }

    #[test]
    fn host_version_entry_matches_the_config_to_the_tag() {
        let release = ReleaseTag::parse("qol-tray-linux-v3.101.4").unwrap();
        let asset = || {
            layer(
                ASSET_MEDIA_TYPE,
                &sha("a"),
                Some("qol-tray-linux-x86_64.tar.gz"),
            )
        };
        let good = host_config(Os::Linux.name(), "3.101.4", COMMIT);
        let entry =
            host_version_entry(&release, &sha("m"), &host_manifest(vec![asset()]), &good).unwrap();
        assert_eq!(entry.manifest_digest, sha("m"));
        assert_eq!(entry.revision, COMMIT);
        assert_eq!(
            entry.assets["qol-tray-linux-x86_64.tar.gz"].digest,
            sha("a")
        );

        let refused: Vec<(&str, Vec<u8>, Vec<Descriptor>)> = vec![
            ("unreadable config", b"not json".to_vec(), vec![asset()]),
            (
                "other os",
                host_config(Os::Macos.name(), "3.101.4", COMMIT),
                vec![asset()],
            ),
            (
                "other version",
                host_config(Os::Linux.name(), "3.101.3", COMMIT),
                vec![asset()],
            ),
            (
                "short revision",
                host_config(Os::Linux.name(), "3.101.4", "abc"),
                vec![asset()],
            ),
            (
                "tree layer",
                good.clone(),
                vec![layer(TREE_MEDIA_TYPE, &sha("t"), None), asset()],
            ),
            ("no files", good.clone(), Vec::new()),
            (
                "untitled file",
                good.clone(),
                vec![layer(ASSET_MEDIA_TYPE, &sha("a"), None)],
            ),
        ];
        for (case, config, layers) in refused {
            assert!(
                host_version_entry(&release, &sha("m"), &host_manifest(layers), &config).is_err(),
                "{case}"
            );
        }
    }

    #[test]
    fn assemble_host_leaves_failed_versions_out_with_a_warning() {
        let outcomes = |cases: &[(&str, bool)]| -> Vec<(ReleaseTag, HostOutcome)> {
            cases
                .iter()
                .map(|(tag, verified)| {
                    let outcome = if *verified {
                        Ok(host_entry(tag))
                    } else {
                        Err(format!("{tag} is left out"))
                    };
                    (ReleaseTag::parse(tag).unwrap(), outcome)
                })
                .collect()
        };
        let (host, warnings) = assemble_host(outcomes(&[
            ("qol-tray-linux-v3.101.4", false),
            ("qol-tray-linux-v3.101.3", true),
            ("qol-tray-macos-v3.101.9", true),
            ("qol-tray-macos-v3.101.10", true),
            ("qol-tray-windows-v3.101.10", false),
        ]));
        let latest: Vec<(&str, &str, usize)> = host
            .iter()
            .map(|(os, host)| (os.as_str(), host.latest.as_str(), host.versions.len()))
            .collect();
        assert_eq!(
            latest,
            [
                (Os::Linux.name(), "3.101.3", 1),
                (Os::Macos.name(), "3.101.10", 2)
            ]
        );
        assert_eq!(
            warnings,
            [
                "qol-tray-linux-v3.101.4 is left out",
                "qol-tray-windows-v3.101.10 is left out"
            ]
        );
        assert!(assemble_host(Vec::new()).0.is_empty());
    }

    #[test]
    fn gather_fetches_only_versions_the_previous_index_lacks() {
        let retained: Vec<ReleaseTag> = ["qol-shot-v2.0.0", "qol-shot-v1.0.0"]
            .map(|tag| ReleaseTag::parse(tag).unwrap())
            .into();
        let gathered = gather(
            retained.clone(),
            |release| (release.version == "1.0.0").then_some("listed"),
            |release| {
                assert_ne!(release.version, "1.0.0", "a listed version was fetched");
                Ok("fetched")
            },
        )
        .unwrap();
        assert_eq!((gathered.reused, gathered.fetched), (1, 1));
        let versions: Vec<(&str, &str)> = gathered
            .versions
            .iter()
            .map(|(release, entry)| (release.version.as_str(), *entry))
            .collect();
        assert_eq!(versions, [("1.0.0", "listed"), ("2.0.0", "fetched")]);
        assert!(gather(retained, |_| None::<&str>, |_| anyhow::bail!("down")).is_err());
    }

    #[test]
    fn changes_cover_host_versions() {
        let location = RegistryLocation {
            url: "u".to_string(),
            repository: "r".to_string(),
        };
        let build = |tags: &[&str]| IndexDocument {
            host: assemble_host(
                tags.iter()
                    .map(|tag| (ReleaseTag::parse(tag).unwrap(), Ok(host_entry(tag))))
                    .collect(),
            )
            .0,
            ..assemble(std::iter::empty(), location.clone(), 1)
        };
        let previous = build(&["qol-tray-linux-v3.101.3", "qol-tray-linux-v3.101.4"]);
        let index = build(&[
            "qol-tray-linux-v3.101.4",
            "qol-tray-linux-v3.101.5",
            "qol-tray-windows-v3.101.5",
        ]);
        assert_eq!(
            changes(Some(&previous), &index),
            [
                "+ host linux 3.101.5",
                "+ host windows 3.101.5",
                "- host linux 3.101.3"
            ]
        );
    }
}
