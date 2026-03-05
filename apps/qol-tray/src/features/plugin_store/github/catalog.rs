use super::super::release_assets::{resolve_asset_pattern, PlatformTarget};
use crate::version::normalize_semver_tag;
use anyhow::Result;
use serde::Deserialize;

const PLUGIN_PREFIX: &str = "plugin-";

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GitHubRepo {
    pub(super) name: String,
    pub(super) html_url: String,
}

#[derive(Debug, Clone)]
pub(super) struct RequiredReleaseAsset {
    pub(super) repo: String,
    pub(super) name: String,
}

pub(super) fn manifest_url(org: &str, repo_name: &str, branch: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{}/{}/{}/plugin.toml",
        org, repo_name, branch
    )
}

pub(super) async fn manifest_from_response(
    repo_name: &str,
    branch: &str,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return manifest_fetch_error(repo_name, branch, status, response).await;
    }

    let content = response.text().await?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

async fn manifest_fetch_error(
    repo_name: &str,
    branch: &str,
    status: reqwest::StatusCode,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let body = response.text().await.unwrap_or_default();
    anyhow::bail!(
        "Failed to fetch plugin.toml for {} on {}: {} {}",
        repo_name,
        branch,
        status,
        body
    )
}

pub(super) fn normalized_release_tag(tag_name: &str) -> Result<String> {
    normalize_semver_tag(tag_name)
        .ok_or_else(|| anyhow::anyhow!("latest release tag '{}' is not valid semver", tag_name))
}

fn is_plugin_repo(name: &str) -> bool {
    if name.trim() != name || name == "plugin-template" {
        return false;
    }
    let Some(suffix) = name.strip_prefix(PLUGIN_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && !suffix.starts_with('-')
        && !suffix.ends_with('-')
        && suffix
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

pub(super) fn filter_plugin_repos(repos: &[GitHubRepo]) -> Vec<&GitHubRepo> {
    repos
        .iter()
        .filter(|repo| is_plugin_repo(&repo.name))
        .collect()
}

pub(super) fn build_plugin_metadata(
    repo: &GitHubRepo,
    manifest: crate::plugins::PluginManifest,
    version: String,
) -> PluginMetadata {
    PluginMetadata {
        id: repo.name.clone(),
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version,
        repo_url: repo.html_url.clone(),
        platforms: manifest.plugin.platforms,
    }
}

pub(super) fn required_release_assets(
    manifest: &crate::plugins::PluginManifest,
    target: PlatformTarget,
) -> Result<Vec<RequiredReleaseAsset>> {
    let dependencies = manifest
        .dependencies
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("manifest is missing dependencies.binaries"))?;
    if dependencies.binaries.is_empty() {
        anyhow::bail!("manifest has empty dependencies.binaries");
    }

    Ok(dependencies
        .binaries
        .iter()
        .map(|binary| RequiredReleaseAsset {
            repo: binary.repo.clone(),
            name: resolve_asset_pattern(&binary.pattern, target),
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
    pub platforms: Option<Vec<String>>,
}

impl PluginMetadata {
    pub fn supports_current_platform(&self) -> bool {
        crate::plugins::manifest::supports_current_platform(&self.platforms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::plugin_store::github::cache::CachedPlugin;
    use crate::plugins::manifest::{MenuConfig, PluginInfo, PluginManifest};

    fn make_repo(name: &str) -> GitHubRepo {
        GitHubRepo {
            name: name.to_string(),
            html_url: format!("https://github.com/test/{}", name),
        }
    }

    fn make_manifest(name: &str, version: &str) -> PluginManifest {
        PluginManifest {
            manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
            plugin: PluginInfo {
                name: name.to_string(),
                description: "Test plugin".to_string(),
                version: version.to_string(),
                author: None,
                platforms: None,
            },
            menu: MenuConfig {
                label: "Test".to_string(),
                icon: None,
                items: vec![],
            },
            daemon: None,
            dependencies: None,
            runtime: None,
        }
    }

    #[test]
    fn is_plugin_repo_filtering() {
        let cases = [
            ("plugin-screen-recorder", true),
            ("plugin-notes", true),
            ("plugin-a", true),
            ("plugin-123", true),
            ("plugin-caps", true),
            ("screen-recorder", false),
            ("my-plugin", false),
            ("pluginstore", false),
            ("plugin-template", false),
            ("", false),
            ("plugin-", false),
            ("plugin", false),
            ("PLUGIN-foo", false),
            ("Plugin-foo", false),
            (" plugin-foo", false),
            ("plugin-foo ", false),
            ("plugin--double", false),
        ];

        for (name, expected) in cases {
            assert_eq!(is_plugin_repo(name), expected, "name: {:?}", name);
        }
    }

    #[test]
    fn filter_plugin_repos_selects_valid_plugins() {
        let cases = [
            (
                vec!["plugin-recorder", "some-tool", "plugin-notes", "pluginish"],
                vec!["plugin-recorder", "plugin-notes"],
            ),
            (vec!["tool-one", "tool-two"], vec![]),
        ];

        for (input_names, expected_names) in cases {
            let repos: Vec<_> = input_names.iter().map(|name| make_repo(name)).collect();
            let filtered = filter_plugin_repos(&repos);
            let names: Vec<_> = filtered.iter().map(|repo| repo.name.as_str()).collect();
            assert_eq!(names, expected_names, "input: {:?}", input_names);
        }
    }

    #[test]
    fn build_plugin_metadata_uses_provided_version() {
        let repo = make_repo("plugin-test");
        let manifest = make_manifest("Test", "1.0.0");
        let metadata = build_plugin_metadata(&repo, manifest, "1.0.0".to_string());
        assert_eq!(metadata.version, "1.0.0");
    }

    #[test]
    fn build_plugin_metadata_extracts_all_fields() {
        let repo = make_repo("plugin-example");
        let manifest = make_manifest("Example Plugin", "2.5.0");
        let metadata = build_plugin_metadata(&repo, manifest, "3.0.0".to_string());

        assert_eq!(metadata.id, "plugin-example");
        assert_eq!(metadata.name, "Example Plugin");
        assert_eq!(metadata.description, "Test plugin");
        assert_eq!(metadata.version, "3.0.0");
        assert_eq!(metadata.repo_url, "https://github.com/test/plugin-example");
    }

    fn make_metadata(platforms: Option<Vec<&str>>) -> PluginMetadata {
        PluginMetadata {
            id: "test".to_string(),
            name: "Test".to_string(),
            description: "Test".to_string(),
            version: "1.0.0".to_string(),
            repo_url: "https://example.com".to_string(),
            platforms: platforms.map(|platforms| platforms.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn plugin_metadata_supports_current_platform_cases() {
        let current_os = std::env::consts::OS;
        let cases: &[(Option<Vec<&str>>, bool)] = &[
            (None, true),
            (Some(vec![]), false),
            (Some(vec![current_os]), true),
            (Some(vec!["not-a-real-os"]), false),
            (Some(vec!["linux", "windows", "macos"]), true),
            (Some(vec!["fake1", "fake2"]), false),
            (Some(vec!["LINUX"]), false),
        ];

        for (platforms, expected) in cases {
            let metadata = make_metadata(platforms.clone());
            assert_eq!(
                metadata.supports_current_platform(),
                *expected,
                "platforms: {:?}",
                platforms
            );
        }
    }

    #[test]
    fn cached_plugin_roundtrip() {
        let metadata = PluginMetadata {
            id: "plugin-test".to_string(),
            name: "Test Plugin".to_string(),
            description: "A test".to_string(),
            version: "1.2.3".to_string(),
            repo_url: "https://github.com/test/plugin-test".to_string(),
            platforms: Some(vec!["linux".to_string()]),
        };

        let cached: CachedPlugin = metadata.clone().into();
        let back: PluginMetadata = cached.into();

        assert_eq!(back.id, metadata.id);
        assert_eq!(back.name, metadata.name);
        assert_eq!(back.description, metadata.description);
        assert_eq!(back.version, metadata.version);
        assert_eq!(back.repo_url, metadata.repo_url);
        assert_eq!(back.platforms, metadata.platforms);
    }
}
