use crate::oci::{Manifest, INDEX_ARTIFACT_TYPE};
use crate::registry::{read_body, send, Registry};
use crate::release::{is_host_tag, newest_per_plugin};
use anyhow::{Context, Result};
use qol_plugin_index::RegistryLocation;
use serde::Deserialize;
use std::collections::BTreeSet;

const GITHUB_API: &str = "https://api.github.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageVersion {
    pub id: u64,
    pub name: String,
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
struct ApiVersion {
    id: u64,
    name: String,
    metadata: ApiMetadata,
}

#[derive(Deserialize)]
struct ApiMetadata {
    container: ApiContainer,
}

#[derive(Deserialize)]
struct ApiContainer {
    #[serde(default)]
    tags: Vec<String>,
}

pub struct Packages {
    agent: ureq::Agent,
    versions_url: String,
    token: String,
}

impl Packages {
    pub fn new(location: &RegistryLocation, token: String) -> Result<Self> {
        Ok(Self {
            agent: crate::registry::agent(),
            versions_url: versions_url(location)?,
            token,
        })
    }

    pub fn versions(&self) -> Result<Vec<PackageVersion>> {
        let mut versions = Vec::new();
        for page in 1.. {
            let url = format!("{}?per_page=100&page={page}", self.versions_url);
            let response = send(self.request("GET", &url), None, &url)?;
            if response.status() != 200 {
                anyhow::bail!("{url} answered {}", response.status());
            }
            let batch: Vec<ApiVersion> = serde_json::from_slice(&read_body(response, &url)?)
                .with_context(|| format!("{url} answered an unreadable version list"))?;
            if batch.is_empty() {
                break;
            }
            versions.extend(batch.into_iter().map(|version| PackageVersion {
                id: version.id,
                name: version.name,
                tags: version.metadata.container.tags,
            }));
        }
        Ok(versions)
    }

    pub fn delete(&self, version: &PackageVersion) -> Result<()> {
        let url = format!("{}/{}", self.versions_url, version.id);
        let response = send(self.request("DELETE", &url), None, &url)?;
        if response.status() != 204 {
            anyhow::bail!("{url} answered {}", response.status());
        }
        Ok(())
    }

    fn request(&self, method: &str, url: &str) -> ureq::Request {
        self.agent
            .request(method, url)
            .set("Accept", "application/vnd.github+json")
            .set("Authorization", &format!("Bearer {}", self.token))
    }
}

pub fn doomed(
    versions: &[PackageVersion],
    keep_host: usize,
    is_index_copy: impl Fn(&PackageVersion) -> bool,
) -> Vec<&PackageVersion> {
    let host_tags = versions
        .iter()
        .flat_map(|version| &version.tags)
        .map(String::as_str)
        .filter(|tag| is_host_tag(tag));
    let kept: BTreeSet<String> = newest_per_plugin(host_tags, keep_host)
        .into_iter()
        .map(|release| release.tag)
        .collect();
    versions
        .iter()
        .filter(|version| {
            if version.tags.is_empty() {
                return is_index_copy(version);
            }
            version
                .tags
                .iter()
                .all(|tag| is_host_tag(tag) && !kept.contains(tag))
        })
        .collect()
}

pub fn is_index_copy(registry: &Registry, version: &PackageVersion) -> bool {
    let Ok(Some(fetched)) = registry.manifest(&version.name) else {
        return false;
    };
    serde_json::from_slice::<Manifest>(&fetched.bytes)
        .is_ok_and(|manifest| manifest.artifact_type == INDEX_ARTIFACT_TYPE)
}

pub fn describe(version: &PackageVersion) -> String {
    if version.tags.is_empty() {
        return format!("{} (untagged)", version.name);
    }
    format!("{} ({})", version.name, version.tags.join(", "))
}

fn versions_url(location: &RegistryLocation) -> Result<String> {
    let (owner, package) = location.repository.split_once('/').with_context(|| {
        format!(
            "registry repository {} must be OWNER/PACKAGE",
            location.repository
        )
    })?;
    Ok(format!(
        "{GITHUB_API}/orgs/{owner}/packages/container/{}/versions",
        package.replace('/', "%2F")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(id: u64, tags: &[&str]) -> PackageVersion {
        PackageVersion {
            id,
            name: format!("sha256:{id}"),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }

    #[test]
    fn doomed_keeps_every_non_host_tag_the_newest_hosts_per_os_and_untagged_non_index_versions() {
        let versions = [
            version(1, &[]),
            version(14, &[]),
            version(2, &["qol-shot-v1.0.0"]),
            version(3, &["index"]),
            version(4, &["qol-tray-linux-v3.101.4"]),
            version(5, &["qol-tray-linux-v3.101.3"]),
            version(6, &["qol-tray-linux-v3.101.2"]),
            version(7, &["qol-tray-linux-v3.9.0", "qol-shot-v0.9.0"]),
            version(8, &["qol-tray-windows-v3.101.4"]),
            version(9, &["qol-tray-freebsd-v1.0.0"]),
            version(10, &["sha256-abc.sig"]),
            version(11, &["qol-tray-macos-v3.101.4"]),
            version(12, &["qol-tray-macos-v3.101.10"]),
            version(13, &["qol-tray-macos-v3.101.9"]),
        ];
        let cases: &[(usize, &[u64])] = &[(2, &[1, 6, 11]), (1, &[1, 5, 6, 11, 13])];
        for (keep_host, expected) in cases {
            let ids: Vec<u64> = doomed(&versions, *keep_host, |version| version.id != 14)
                .into_iter()
                .map(|version| version.id)
                .collect();
            assert_eq!(ids, *expected, "keep {keep_host}");
        }
    }

    #[test]
    fn versions_url_names_the_org_package() {
        let location = |repository: &str| RegistryLocation {
            url: "https://ghcr.io".to_string(),
            repository: repository.to_string(),
        };
        let cases: &[(&str, Option<&str>)] = &[
            (
                "qol-tools/plugins",
                Some("https://api.github.com/orgs/qol-tools/packages/container/plugins/versions"),
            ),
            (
                "qol-tools/a/b",
                Some("https://api.github.com/orgs/qol-tools/packages/container/a%2Fb/versions"),
            ),
            ("plugins", None),
        ];
        for (repository, expected) in cases {
            assert_eq!(
                versions_url(&location(repository)).ok().as_deref(),
                *expected,
                "{repository}"
            );
        }
    }

    #[test]
    fn describe_lists_the_tags() {
        assert_eq!(describe(&version(1, &[])), "sha256:1 (untagged)");
        assert_eq!(describe(&version(2, &["a", "b"])), "sha256:2 (a, b)");
    }
}
