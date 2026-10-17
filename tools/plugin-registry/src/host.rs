use crate::artifact::{push_once, Pushed};
use crate::oci::{
    is_lower_hex, Descriptor, Manifest, ASSET_MEDIA_TYPE, HOST_ARTIFACT_TYPE,
    HOST_CONFIG_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, REVISION_ANNOTATION, SOURCE_ANNOTATION,
};
use crate::registry::Registry;
use crate::release::ReleaseTag;
use anyhow::Result;
use qol_plugin_index::host_tag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConfig {
    pub os: String,
    pub version: String,
    pub revision: String,
}

impl HostConfig {
    pub fn tag(&self) -> Result<String> {
        let tag = host_tag(&self.os, &self.version);
        let names_a_build = ReleaseTag::parse(&tag).is_some_and(|release| {
            release.host_system() == Some(self.os.as_str()) && release.version == self.version
        });
        if !names_a_build {
            anyhow::bail!(
                "os {:?} and version {:?} do not name a qol-tray build",
                self.os,
                self.version
            );
        }
        if !is_lower_hex(&self.revision, 40) {
            anyhow::bail!("revision {:?} is not a full commit id", self.revision);
        }
        Ok(tag)
    }
}

pub struct HostBuild {
    pub tag: String,
    pub revision: String,
    pub config: Vec<u8>,
    pub assets: BTreeMap<String, Vec<u8>>,
}

pub fn host_build(config: HostConfig, assets: BTreeMap<String, Vec<u8>>) -> Result<HostBuild> {
    let tag = config.tag()?;
    if assets.is_empty() {
        anyhow::bail!("{tag} has no files to push");
    }
    Ok(HostBuild {
        tag,
        config: serde_json::to_vec(&config)?,
        revision: config.revision,
        assets,
    })
}

pub fn manifest(build: &HostBuild, source: &str) -> Manifest {
    Manifest {
        schema_version: 2,
        media_type: MANIFEST_MEDIA_TYPE.to_string(),
        artifact_type: HOST_ARTIFACT_TYPE.to_string(),
        config: Descriptor::of(HOST_CONFIG_MEDIA_TYPE, &build.config),
        layers: build
            .assets
            .iter()
            .map(|(name, bytes)| Descriptor::of(ASSET_MEDIA_TYPE, bytes).titled(name))
            .collect(),
        annotations: BTreeMap::from([
            (REVISION_ANNOTATION.to_string(), build.revision.clone()),
            (SOURCE_ANNOTATION.to_string(), source.to_string()),
        ]),
    }
}

pub fn push(registry: &Registry, build: &HostBuild, source: &str) -> Result<Pushed> {
    let blobs: Vec<&[u8]> = std::iter::once(build.config.as_slice())
        .chain(build.assets.values().map(Vec::as_slice))
        .collect();
    push_once(registry, &build.tag, &manifest(build, source), &blobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_platform::Os;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn config(os: &str, version: &str, revision: &str) -> HostConfig {
        HostConfig {
            os: os.to_string(),
            version: version.to_string(),
            revision: revision.to_string(),
        }
    }

    #[test]
    fn host_config_names_one_build_of_a_known_os() {
        let cases: &[(HostConfig, Option<&str>)] = &[
            (
                config(Os::Linux.name(), "3.101.4", COMMIT),
                Some("qol-tray-linux-v3.101.4"),
            ),
            (
                config(Os::Windows.name(), "3.0.0", COMMIT),
                Some("qol-tray-windows-v3.0.0"),
            ),
            (config("freebsd", "3.101.4", COMMIT), None),
            (config(Os::Linux.name(), "3.101", COMMIT), None),
            (config(Os::Linux.name(), "3.101.4-v1.0.0", COMMIT), None),
            (config(Os::Linux.name(), "3.101.4", "abc123"), None),
            (
                config(Os::Linux.name(), "3.101.4", &COMMIT.to_uppercase()),
                None,
            ),
        ];
        for (config, tag) in cases {
            assert_eq!(config.tag().ok().as_deref(), *tag, "{config:?}");
        }
    }

    #[test]
    fn host_build_needs_files() {
        let error = host_build(config(Os::Linux.name(), "1.0.0", COMMIT), BTreeMap::new())
            .err()
            .unwrap();
        assert!(error.to_string().contains("no files"), "{error:#}");
    }

    #[test]
    fn manifest_titles_every_file_and_carries_the_config_body() {
        let build = host_build(
            config(Os::Windows.name(), "3.101.4", COMMIT),
            BTreeMap::from([
                ("qol-tray-windows-x86_64.exe".to_string(), b"tray".to_vec()),
                (
                    "qol-tray-install-windows-x86_64.exe".to_string(),
                    b"installer".to_vec(),
                ),
            ]),
        )
        .unwrap();
        assert_eq!(build.tag, "qol-tray-windows-v3.101.4");
        assert_eq!(
            String::from_utf8(build.config.clone()).unwrap(),
            format!(r#"{{"os":"windows","version":"3.101.4","revision":"{COMMIT}"}}"#)
        );
        let parsed: Manifest =
            serde_json::from_slice(&serde_json::to_vec(&manifest(&build, "s")).unwrap()).unwrap();
        assert_eq!(parsed.artifact_type, HOST_ARTIFACT_TYPE);
        assert_eq!(
            parsed.config,
            Descriptor::of(HOST_CONFIG_MEDIA_TYPE, &build.config)
        );
        assert_eq!(parsed.annotations[REVISION_ANNOTATION], COMMIT);
        assert_eq!(parsed.annotations[SOURCE_ANNOTATION], "s");
        let layers: Vec<(&str, Option<&str>)> = parsed
            .layers
            .iter()
            .map(|layer| (layer.media_type.as_str(), layer.title()))
            .collect();
        assert_eq!(
            layers,
            [
                (
                    ASSET_MEDIA_TYPE,
                    Some("qol-tray-install-windows-x86_64.exe")
                ),
                (ASSET_MEDIA_TYPE, Some("qol-tray-windows-x86_64.exe")),
            ]
        );
    }
}
