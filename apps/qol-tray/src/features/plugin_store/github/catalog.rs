use super::super::source::PluginSource;
use crate::version::normalize_semver_tag;
use anyhow::Result;

pub(super) async fn manifest_from_response(
    plugin_id: &str,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return manifest_fetch_error(plugin_id, status, response).await;
    }

    let content = response.text().await?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content)?;
    manifest.validate()?;
    Ok(Some(manifest))
}

async fn manifest_fetch_error(
    plugin_id: &str,
    status: reqwest::StatusCode,
    response: reqwest::Response,
) -> Result<Option<crate::plugins::PluginManifest>> {
    let body = response.text().await.unwrap_or_default();
    anyhow::bail!(
        "Failed to fetch plugin.toml for {}: {} {}",
        plugin_id,
        status,
        body
    )
}

pub(super) fn normalized_release_tag(tag_name: &str) -> Result<String> {
    normalize_semver_tag(tag_name)
        .ok_or_else(|| anyhow::anyhow!("latest release tag '{}' is not valid semver", tag_name))
}

pub(super) fn build_plugin_metadata(
    plugin_id: &str,
    source: &PluginSource,
    manifest: crate::plugins::PluginManifest,
    version: String,
) -> PluginMetadata {
    PluginMetadata {
        id: plugin_id.to_string(),
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version,
        repo_url: source.plugin_subdir_html_url(plugin_id),
        platforms: manifest.plugin.platforms,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
    pub platforms: Option<Vec<String>>,
}

impl PluginMetadata {
    pub(crate) fn supports_current_platform(&self) -> bool {
        crate::plugins::manifest::supports_current_platform(&self.platforms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::plugin_store::github::cache::CachedPlugin;
    use crate::plugins::manifest::{
        BuildInfo, Capabilities, MenuConfig, PluginInfo, PluginManifest,
    };

    fn make_manifest(name: &str, version: &str) -> PluginManifest {
        PluginManifest {
            manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
            plugin: PluginInfo {
                id: Some("test-plugin".into()),
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
            capabilities: Capabilities::default(),
            build: BuildInfo::default(),
            traits: None,
            config: crate::plugins::manifest::ConfigDeclarations::default(),
        }
    }

    fn core_source() -> PluginSource {
        PluginSource::new("core", "qol-tools/qol", "main")
    }

    #[test]
    fn build_plugin_metadata_uses_provided_version() {
        let manifest = make_manifest("Test", "1.0.0");
        let metadata =
            build_plugin_metadata("plugin-test", &core_source(), manifest, "1.0.0".to_string());
        assert_eq!(metadata.version, "1.0.0");
    }

    #[test]
    fn build_plugin_metadata_extracts_all_fields_and_uses_source_subdir_url() {
        let manifest = make_manifest("Example Plugin", "2.5.0");
        let metadata = build_plugin_metadata(
            "plugin-example",
            &core_source(),
            manifest,
            "3.0.0".to_string(),
        );

        assert_eq!(metadata.id, "plugin-example");
        assert_eq!(metadata.name, "Example Plugin");
        assert_eq!(metadata.description, "Test plugin");
        assert_eq!(metadata.version, "3.0.0");
        assert_eq!(
            metadata.repo_url, "https://github.com/qol-tools/qol/tree/main/plugins/plugin-example",
            "repo_url must point at the subdir within the source repo, not a per-plugin repo"
        );
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
            repo_url: "https://github.com/qol-tools/qol/tree/main/plugins/plugin-test".to_string(),
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
