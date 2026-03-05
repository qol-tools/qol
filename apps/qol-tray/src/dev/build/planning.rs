use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::fingerprint::fingerprint_plugin;
use super::types::PluginBuildPlan;

pub fn plan_linked_plugin_builds(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
) -> Vec<PluginBuildPlan> {
    sorted_links(dev_links)
        .into_iter()
        .map(|(plugin_id, path)| plan_plugin_build(plugin_id, path, known_fingerprints))
        .collect()
}

fn sorted_links(dev_links: &HashMap<String, PathBuf>) -> Vec<(&String, &PathBuf)> {
    let mut links: Vec<_> = dev_links.iter().collect();
    links.sort_by(|(left, _), (right, _)| left.cmp(right));
    links
}

fn plan_plugin_build(
    plugin_id: &String,
    path: &PathBuf,
    known_fingerprints: &HashMap<String, String>,
) -> PluginBuildPlan {
    let has_cargo = path.join("Cargo.toml").is_file();
    let last_built_fingerprint = known_fingerprints.get(plugin_id).cloned();
    let (supports_platform, platform_reason) = check_plugin_platform(path);

    if !has_cargo {
        return missing_cargo_plan(plugin_id, path, last_built_fingerprint);
    }

    if !supports_platform {
        return unsupported_platform_plan(plugin_id, path, last_built_fingerprint, platform_reason);
    }

    match fingerprint_plugin(path) {
        Ok(current_fingerprint) => {
            fingerprinted_plan(plugin_id, path, last_built_fingerprint, current_fingerprint)
        }
        Err(error) => fingerprint_unavailable_plan(plugin_id, path, last_built_fingerprint, error),
    }
}

fn missing_cargo_plan(
    plugin_id: &String,
    path: &PathBuf,
    last_built_fingerprint: Option<String>,
) -> PluginBuildPlan {
    PluginBuildPlan {
        plugin_id: plugin_id.clone(),
        path: path.clone(),
        has_cargo: false,
        supports_platform: true,
        needs_rebuild: false,
        current_fingerprint: None,
        last_built_fingerprint,
        reason: "Cargo.toml missing".to_string(),
    }
}

fn unsupported_platform_plan(
    plugin_id: &String,
    path: &PathBuf,
    last_built_fingerprint: Option<String>,
    reason: String,
) -> PluginBuildPlan {
    PluginBuildPlan {
        plugin_id: plugin_id.clone(),
        path: path.clone(),
        has_cargo: true,
        supports_platform: false,
        needs_rebuild: false,
        current_fingerprint: None,
        last_built_fingerprint,
        reason,
    }
}

fn fingerprinted_plan(
    plugin_id: &String,
    path: &PathBuf,
    last_built_fingerprint: Option<String>,
    current_fingerprint: String,
) -> PluginBuildPlan {
    let needs_rebuild = build_needed(&last_built_fingerprint, &current_fingerprint);
    PluginBuildPlan {
        plugin_id: plugin_id.clone(),
        path: path.clone(),
        has_cargo: true,
        supports_platform: true,
        needs_rebuild,
        current_fingerprint: Some(current_fingerprint),
        last_built_fingerprint: last_built_fingerprint.clone(),
        reason: build_reason(last_built_fingerprint.as_ref(), needs_rebuild),
    }
}

fn fingerprint_unavailable_plan(
    plugin_id: &String,
    path: &PathBuf,
    last_built_fingerprint: Option<String>,
    error: String,
) -> PluginBuildPlan {
    PluginBuildPlan {
        plugin_id: plugin_id.clone(),
        path: path.clone(),
        has_cargo: true,
        supports_platform: true,
        needs_rebuild: true,
        current_fingerprint: None,
        last_built_fingerprint,
        reason: format!("Fingerprint unavailable: {}", error),
    }
}

fn build_needed(last_built_fingerprint: &Option<String>, current_fingerprint: &str) -> bool {
    last_built_fingerprint
        .as_ref()
        .map(|known| known != current_fingerprint)
        .unwrap_or(true)
}

fn build_reason(last_built_fingerprint: Option<&String>, needs_rebuild: bool) -> String {
    if !needs_rebuild {
        return "Up to date".to_string();
    }
    if last_built_fingerprint.is_some() {
        return "Source changed".to_string();
    }
    "No successful build recorded".to_string()
}

fn check_plugin_platform(path: &Path) -> (bool, String) {
    let toml_path = path.join("plugin.toml");
    let Ok(content) = std::fs::read_to_string(&toml_path) else {
        return (true, String::new());
    };
    let Ok(manifest) = toml::from_str::<crate::plugins::PluginManifest>(&content) else {
        return (true, String::new());
    };
    if manifest.plugin.supports_current_platform() {
        return (true, String::new());
    }
    let declared = manifest
        .plugin
        .platforms
        .as_ref()
        .map(|platforms| platforms.join(", "))
        .unwrap_or_else(|| "none".to_string());
    (
        false,
        format!(
            "Not supported on {} (requires {})",
            std::env::consts::OS,
            declared
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::build::{
        build_linked_plugins_with_progress, load_build_fingerprints, save_build_fingerprints,
    };
    use crate::dev::core::BuildStatus;
    use std::fs;
    use tempfile::TempDir;

    fn write_basic_plugin(root: &Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    #[test]
    fn plan_marks_new_plugin_for_rebuild() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "No successful build recorded");
    }

    #[test]
    fn plan_marks_unchanged_plugin_as_up_to_date() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir.clone())]);
        let fingerprint = fingerprint_plugin(&plugin_dir).unwrap();
        let known = HashMap::from([("plugin-a".to_string(), fingerprint)]);
        let plans = plan_linked_plugin_builds(&links, &known);

        assert_eq!(plans.len(), 1);
        assert!(!plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "Up to date");
    }

    #[test]
    fn plan_skips_plugin_without_cargo_manifest() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("src.rs"), "fn main() {}\n").unwrap();

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(!plans[0].has_cargo);
        assert!(!plans[0].needs_rebuild);
        assert_eq!(plans[0].reason, "Cargo.toml missing");
    }

    #[test]
    fn fingerprint_state_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let data = HashMap::from([("plugin-a".to_string(), "abc".to_string())]);

        save_build_fingerprints(tmp.path(), &data).unwrap();
        let loaded = load_build_fingerprints(tmp.path());

        assert_eq!(loaded, data);
    }

    #[test]
    fn plan_skips_plugin_with_unsupported_platform() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);
        let unsupported = if cfg!(target_os = "linux") {
            "windows"
        } else {
            "linux"
        };
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"Test\"\ndescription = \"\"\nversion = \"1.0.0\"\nplatforms = [\"{unsupported}\"]\n\n[menu]\nlabel = \"Test\"\nitems = []\n"
            ),
        )
        .unwrap();

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(plans[0].has_cargo);
        assert!(!plans[0].supports_platform);
        assert!(!plans[0].needs_rebuild);
        assert!(plans[0].reason.contains("Not supported on"));
        assert!(plans[0].reason.contains(unsupported));
    }

    #[test]
    fn plan_builds_plugin_with_matching_platform() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nname = \"Test\"\ndescription = \"\"\nversion = \"1.0.0\"\nplatforms = [\"{}\"]\n\n[menu]\nlabel = \"Test\"\nitems = []\n",
                std::env::consts::OS
            ),
        )
        .unwrap();

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(plans[0].supports_platform);
        assert!(plans[0].needs_rebuild);
    }

    #[test]
    fn plan_defaults_to_supported_without_plugin_toml() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        write_basic_plugin(&plugin_dir);

        let links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        let plans = plan_linked_plugin_builds(&links, &HashMap::new());

        assert_eq!(plans.len(), 1);
        assert!(plans[0].supports_platform);
    }

    #[test]
    fn build_progress_does_not_queue_plugins_that_will_be_skipped() {
        let tmp = TempDir::new().unwrap();
        let plugin_a = tmp.path().join("plugin-a");
        let plugin_b = tmp.path().join("plugin-b");
        write_basic_plugin(&plugin_a);
        fs::create_dir_all(&plugin_b).unwrap();

        let links = HashMap::from([
            ("plugin-a".to_string(), plugin_a.clone()),
            ("plugin-b".to_string(), plugin_b),
        ]);
        let known = HashMap::from([(
            "plugin-a".to_string(),
            fingerprint_plugin(&plugin_a).expect("fingerprint"),
        )]);

        let mut events: Vec<(String, BuildStatus)> = Vec::new();
        let run = build_linked_plugins_with_progress(&links, &known, |progress| {
            events.push((progress.plugin_id, progress.status));
        });

        assert!(!events.iter().any(|(plugin_id, status)| {
            plugin_id == "plugin-a" && *status == BuildStatus::Queued
        }));
        assert!(!events.iter().any(|(plugin_id, status)| {
            plugin_id == "plugin-b" && *status == BuildStatus::Queued
        }));

        assert_eq!(run.results.len(), 2);
        assert!(run.results.iter().all(|result| result.skipped));
    }
}
