use super::CachedPlugin;
use super::*;
use crate::features::plugin_store::source::PluginSource;
use crate::plugins::manifest::{BuildInfo, Capabilities, MenuConfig, PluginInfo, PluginManifest};

fn make_manifest(name: &str, version: &str) -> PluginManifest {
    PluginManifest {
        manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
        plugin: PluginInfo {
            id: Some("test-plugin".into()),
            uid: None,
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
        actions: Default::default(),
        capabilities: Capabilities::default(),
        build: BuildInfo::default(),
        traits: None,
        shortcuts: Vec::new(),
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
    let current_os = super::super::super::platform::current_manifest_token();
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
