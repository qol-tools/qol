use anyhow::{anyhow, Result};

use crate::features::profile::core::PluginLockEntry;
use crate::paths::is_safe_path_component;
use crate::plugins::manifest::{ConfigScope, PluginManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfigResolution {
    pub scope: ConfigScope,
    pub os_bucket: Option<String>,
}

pub fn resolve_plugin_config(
    lock_entry: Option<&PluginLockEntry>,
    manifest: Option<&PluginManifest>,
    current_os: &str,
) -> Result<PluginConfigResolution> {
    if !is_safe_path_component(current_os) {
        return Err(anyhow!("invalid current_os: {current_os}"));
    }

    let declared_single = single_declared_platform(lock_entry, manifest);
    let declared_default = manifest.and_then(|m| m.config.default_scope);

    let (scope, os_bucket) = match declared_default {
        Some(ConfigScope::Core) => (ConfigScope::Core, None),
        Some(ConfigScope::Device) => (ConfigScope::Device, None),
        Some(ConfigScope::Os) => {
            let bucket = declared_single.unwrap_or_else(|| current_os.to_string());
            validate_bucket(&bucket)?;
            (ConfigScope::Os, Some(bucket))
        }
        None => match declared_single {
            Some(bucket) => {
                validate_bucket(&bucket)?;
                (ConfigScope::Os, Some(bucket))
            }
            None => (ConfigScope::Core, None),
        },
    };

    Ok(PluginConfigResolution { scope, os_bucket })
}

pub fn classify_os_bucket(
    lock_entry: Option<&PluginLockEntry>,
    manifest: Option<&PluginManifest>,
    current_os: &str,
) -> Result<String> {
    if !is_safe_path_component(current_os) {
        return Err(anyhow!("invalid current_os: {current_os}"));
    }
    let bucket =
        single_declared_platform(lock_entry, manifest).unwrap_or_else(|| current_os.to_string());
    validate_bucket(&bucket)?;
    Ok(bucket)
}

fn single_declared_platform(
    lock_entry: Option<&PluginLockEntry>,
    manifest: Option<&PluginManifest>,
) -> Option<String> {
    lock_entry
        .and_then(|e| single_platform(e.platforms.as_deref()))
        .or_else(|| manifest.and_then(|m| single_platform(m.plugin.platforms.as_deref())))
}

fn single_platform(platforms: Option<&[String]>) -> Option<String> {
    match platforms {
        Some([only]) => Some(only.clone()),
        _ => None,
    }
}

fn validate_bucket(bucket: &str) -> Result<()> {
    if !is_safe_path_component(bucket) {
        return Err(anyhow!("invalid os_bucket: {bucket}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{
        Capabilities, ConfigDeclarations, MenuConfig, PluginInfo, PluginManifest,
    };

    fn entry(id: &str, platforms: Option<Vec<&str>>) -> PluginLockEntry {
        PluginLockEntry {
            id: id.to_string(),
            repo_url: "https://example/repo.git".to_string(),
            version: "1.0.0".to_string(),
            platforms: platforms.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        }
    }

    fn manifest(
        platforms: Option<Vec<&str>>,
        default_scope: Option<ConfigScope>,
    ) -> PluginManifest {
        let mut config = ConfigDeclarations::default();
        config.default_scope = default_scope;
        PluginManifest {
            manifest_version: 1,
            plugin: PluginInfo {
                name: "p".to_string(),
                description: String::new(),
                version: "1.0.0".to_string(),
                author: None,
                platforms: platforms.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            },
            menu: MenuConfig {
                label: "p".to_string(),
                icon: None,
                items: vec![],
            },
            daemon: None,
            dependencies: None,
            runtime: None,
            capabilities: Capabilities::default(),
            build: Default::default(),
            traits: None,
            config,
        }
    }

    struct Case {
        name: &'static str,
        lock: Option<PluginLockEntry>,
        manifest: Option<PluginManifest>,
        current_os: &'static str,
        expected_scope: ConfigScope,
        expected_os_bucket: Option<&'static str>,
    }

    fn run(case: &Case) {
        let got =
            resolve_plugin_config(case.lock.as_ref(), case.manifest.as_ref(), case.current_os)
                .unwrap_or_else(|e| panic!("[{}] resolver errored: {e:#}", case.name));
        assert_eq!(got.scope, case.expected_scope, "[{}] scope", case.name);
        assert_eq!(
            got.os_bucket.as_deref(),
            case.expected_os_bucket,
            "[{}] os_bucket",
            case.name
        );
    }

    #[test]
    fn fallthrough_with_no_signals_lands_in_core() {
        run(&Case {
            name: "no lock, no manifest, no scope",
            lock: None,
            manifest: None,
            current_os: "linux",
            expected_scope: ConfigScope::Core,
            expected_os_bucket: None,
        });
    }

    #[test]
    fn lock_single_platform_routes_to_that_os_bucket_not_current_os() {
        run(&Case {
            name: "lock platforms=[macos] while running on linux must still target os/macos so a sync push from linux preserves the Mac slot",
            lock: Some(entry("plugin-keyremap", Some(vec!["macos"]))),
            manifest: None,
            current_os: "linux",
            expected_scope: ConfigScope::Os,
            expected_os_bucket: Some("macos"),
        });
    }

    #[test]
    fn manifest_single_platform_routes_when_lock_is_absent() {
        run(&Case {
            name: "no lock entry, manifest platforms=[windows]",
            lock: None,
            manifest: Some(manifest(Some(vec!["windows"]), None)),
            current_os: "linux",
            expected_scope: ConfigScope::Os,
            expected_os_bucket: Some("windows"),
        });
    }

    #[test]
    fn lock_multi_platform_without_default_scope_falls_through_to_core() {
        run(&Case {
            name: "lock platforms=[linux, macos], no default_scope",
            lock: Some(entry("plugin-cross", Some(vec!["linux", "macos"]))),
            manifest: None,
            current_os: "linux",
            expected_scope: ConfigScope::Core,
            expected_os_bucket: None,
        });
    }

    #[test]
    fn default_scope_core_wins_over_single_platform_lock() {
        run(&Case {
            name: "lock says macos but plugin author opted into Core",
            lock: Some(entry("plugin-portable", Some(vec!["macos"]))),
            manifest: Some(manifest(Some(vec!["macos"]), Some(ConfigScope::Core))),
            current_os: "macos",
            expected_scope: ConfigScope::Core,
            expected_os_bucket: None,
        });
    }

    #[test]
    fn default_scope_device_routes_to_device_regardless_of_platform_signals() {
        run(&Case {
            name: "default_scope=device beats lock single platform",
            lock: Some(entry("plugin-secrets", Some(vec!["macos"]))),
            manifest: Some(manifest(Some(vec!["macos"]), Some(ConfigScope::Device))),
            current_os: "macos",
            expected_scope: ConfigScope::Device,
            expected_os_bucket: None,
        });
    }

    #[test]
    fn default_scope_os_on_cross_platform_plugin_uses_current_os() {
        run(&Case {
            name: "cross-platform plugin opted into per-OS scope routes to current os bucket",
            lock: Some(entry("plugin-per-os", Some(vec!["linux", "macos"]))),
            manifest: Some(manifest(
                Some(vec!["linux", "macos"]),
                Some(ConfigScope::Os),
            )),
            current_os: "linux",
            expected_scope: ConfigScope::Os,
            expected_os_bucket: Some("linux"),
        });
    }

    #[test]
    fn default_scope_os_on_single_platform_plugin_prefers_declared_platform_over_current_os() {
        run(&Case {
            name: "Mac-only plugin written from a Linux machine still routes to os/macos",
            lock: Some(entry("plugin-keyremap", Some(vec!["macos"]))),
            manifest: Some(manifest(Some(vec!["macos"]), Some(ConfigScope::Os))),
            current_os: "linux",
            expected_scope: ConfigScope::Os,
            expected_os_bucket: Some("macos"),
        });
    }

    #[test]
    fn default_scope_os_with_no_platform_signal_falls_back_to_current_os() {
        run(&Case {
            name: "no lock, no manifest platforms, default_scope=os",
            lock: None,
            manifest: Some(manifest(None, Some(ConfigScope::Os))),
            current_os: "macos",
            expected_scope: ConfigScope::Os,
            expected_os_bucket: Some("macos"),
        });
    }

    #[test]
    fn lock_wins_over_manifest_when_both_single_platform_but_disagree() {
        run(&Case {
            name: "lock claims macos, manifest says linux - lock is the cross-machine truth",
            lock: Some(entry("plugin-divergent", Some(vec!["macos"]))),
            manifest: Some(manifest(Some(vec!["linux"]), None)),
            current_os: "linux",
            expected_scope: ConfigScope::Os,
            expected_os_bucket: Some("macos"),
        });
    }

    #[test]
    fn empty_platforms_array_in_lock_treated_as_unknown_not_single_platform() {
        run(&Case {
            name: "platforms=[] is unknown, must not route to os/<empty>",
            lock: Some(entry("plugin-y", Some(vec![]))),
            manifest: None,
            current_os: "linux",
            expected_scope: ConfigScope::Core,
            expected_os_bucket: None,
        });
    }

    #[test]
    fn lock_with_no_platforms_field_treated_as_unknown() {
        run(&Case {
            name: "platforms = None means we cannot classify - leave in core, not os/<current>",
            lock: Some(entry("plugin-z", None)),
            manifest: None,
            current_os: "linux",
            expected_scope: ConfigScope::Core,
            expected_os_bucket: None,
        });
    }

    #[test]
    fn rejects_invalid_current_os_string() {
        let err = resolve_plugin_config(None, None, "../linux").unwrap_err();
        assert!(
            format!("{err:#}").contains("invalid current_os"),
            "expected invalid-os rejection, got: {err:#}"
        );
    }

    #[test]
    fn classify_os_bucket_with_no_platform_signal_returns_current_os() {
        assert_eq!(classify_os_bucket(None, None, "linux").unwrap(), "linux");
    }

    #[test]
    fn classify_os_bucket_uses_lock_single_platform_even_from_another_os() {
        let lock = entry("plugin-keyremap", Some(vec!["macos"]));
        assert_eq!(
            classify_os_bucket(Some(&lock), None, "linux").unwrap(),
            "macos",
            "Linux machine must still target Mac-only plugin's os bucket"
        );
    }

    #[test]
    fn classify_os_bucket_uses_manifest_single_platform_when_lock_absent() {
        let m = manifest(Some(vec!["windows"]), None);
        assert_eq!(
            classify_os_bucket(None, Some(&m), "linux").unwrap(),
            "windows"
        );
    }

    #[test]
    fn classify_os_bucket_prefers_lock_over_manifest_when_both_single_platform() {
        let lock = entry("p", Some(vec!["macos"]));
        let m = manifest(Some(vec!["linux"]), None);
        assert_eq!(
            classify_os_bucket(Some(&lock), Some(&m), "linux").unwrap(),
            "macos",
            "lock is the cross-machine truth; manifest is only the local view"
        );
    }

    #[test]
    fn classify_os_bucket_with_multi_platform_lock_falls_back_to_current_os() {
        let lock = entry("p", Some(vec!["linux", "macos"]));
        assert_eq!(
            classify_os_bucket(Some(&lock), None, "linux").unwrap(),
            "linux"
        );
    }

    #[test]
    fn classify_os_bucket_rejects_unsafe_bucket_strings() {
        let lock = entry("p", Some(vec!["../etc"]));
        let err = classify_os_bucket(Some(&lock), None, "linux").unwrap_err();
        assert!(format!("{err:#}").contains("invalid os_bucket"));
    }
}
