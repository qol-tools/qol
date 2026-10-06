use super::document::{IndexedPlugin, IndexedVersion};
use super::{fetch, IndexLocation};
use crate::features::plugin_store::github::{build_plugin_metadata, PluginMetadata};
use crate::features::plugin_store::release_assets::PlatformTarget;
use crate::features::plugin_store::source::{required_release_asset_names, PluginSource};
use crate::plugins::PluginManifest;
use anyhow::{Context, Result};

pub(crate) async fn list_plugins(
    source: &PluginSource,
    location: &IndexLocation,
) -> Result<Vec<PluginMetadata>> {
    let document = fetch::load(location).await?;
    let target = PlatformTarget::current()?;
    Ok(document
        .plugins
        .iter()
        .filter_map(|(plugin_id, plugin)| {
            plugin_metadata(source, plugin_id, plugin, target)
                .inspect_err(|error| {
                    log::warn!("Skipping {plugin_id} from the plugin index: {error:#}")
                })
                .ok()
        })
        .collect())
}

fn plugin_metadata(
    source: &PluginSource,
    plugin_id: &str,
    plugin: &IndexedPlugin,
    target: PlatformTarget,
) -> Result<PluginMetadata> {
    let version = plugin
        .versions
        .get(&plugin.latest)
        .with_context(|| format!("latest version {} is not listed", plugin.latest))?;
    let manifest = PluginManifest::parse_and_validate(&version.plugin_toml)?;
    let declared = manifest.plugin.require_declared_id()?.as_str().to_string();
    if declared != plugin_id {
        anyhow::bail!("its plugin.toml declares {declared}");
    }
    require_platform_assets(&manifest, version, target)?;
    build_plugin_metadata(&version.dir, source, manifest, plugin.latest.clone())
}

fn require_platform_assets(
    manifest: &PluginManifest,
    version: &IndexedVersion,
    target: PlatformTarget,
) -> Result<()> {
    let binaries = manifest
        .dependencies
        .as_ref()
        .map(|dependencies| dependencies.binaries.as_slice())
        .unwrap_or_default();
    if binaries.is_empty() {
        anyhow::bail!("its plugin.toml has no dependencies.binaries");
    }
    for asset_name in required_release_asset_names(binaries, target) {
        if !version.assets.contains_key(&asset_name) {
            anyhow::bail!("the release has no {asset_name}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::document::fixtures::{blob, document, version};
    use super::super::document::IndexDocument;
    use super::*;
    use serde_json::json;

    fn plugin(plugin_id: &str, assets: serde_json::Value) -> IndexedPlugin {
        let value = document(
            1,
            "https://ghcr.io",
            json!({
                plugin_id: {
                    "latest": "1.0.0",
                    "versions": { "1.0.0": version(plugin_id, "1.0.0", blob("sha256:t", 1), assets) }
                }
            }),
        );
        let parsed: IndexDocument = serde_json::from_value(value).unwrap();
        parsed.plugins[plugin_id].clone()
    }

    fn all_platform_assets(plugin_id: &str) -> serde_json::Value {
        let mut assets = serde_json::Map::new();
        for suffix in [
            "linux-x86_64",
            "linux-aarch64",
            "macos-aarch64",
            "macos-x86_64",
            "windows-x86_64",
        ] {
            assets.insert(format!("{plugin_id}-{suffix}"), blob("sha256:b", 1));
        }
        serde_json::Value::Object(assets)
    }

    #[test]
    fn lists_a_plugin_with_this_platforms_binary_from_its_released_manifest() {
        let source = PluginSource::new("core", "qol-tools/qol", "main");
        let target = PlatformTarget::current().unwrap();
        let entry = plugin("qol-shot", all_platform_assets("qol-shot"));
        let metadata = plugin_metadata(&source, "qol-shot", &entry, target).unwrap();
        assert_eq!(metadata.id, "qol-shot");
        assert_eq!(metadata.version, "1.0.0");
        assert_eq!(
            metadata.repo_url,
            "https://github.com/qol-tools/qol/tree/main/plugins/shot"
        );
    }

    #[test]
    fn skips_plugins_the_index_cannot_install_here() {
        let source = PluginSource::new("core", "qol-tools/qol", "main");
        let target = PlatformTarget::current().unwrap();
        let cases: &[(&str, &str, IndexedPlugin)] = &[
            (
                "no binary for this platform",
                "qol-shot",
                plugin("qol-shot", json!({})),
            ),
            (
                "id differs from manifest",
                "qol-other",
                plugin("qol-shot", all_platform_assets("qol-shot")),
            ),
        ];
        for (case, plugin_id, entry) in cases {
            assert!(
                plugin_metadata(&source, plugin_id, entry, target).is_err(),
                "{case}"
            );
        }
        let mut missing_latest = plugin("qol-shot", all_platform_assets("qol-shot"));
        missing_latest.latest = "2.0.0".to_string();
        assert!(plugin_metadata(&source, "qol-shot", &missing_latest, target).is_err());
    }
}
