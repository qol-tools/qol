mod cargo_build;
mod fingerprint;
mod fingerprint_store;
mod service;
mod types;

use std::collections::HashMap;
use std::path::PathBuf;

use fingerprint::fingerprint_plugin;

pub use cargo_build::build_qol_tray_self_with_progress;
pub use fingerprint_store::{load_build_fingerprints, save_build_fingerprints};
pub use service::{
    build_linked_plugins, build_linked_plugins_with_core_events, build_linked_plugins_with_progress,
    default_build_application_service, BuildApplicationService,
};
pub use types::{BuildResult, BuildRun, PluginBuildPlan, PluginBuildProgress};

pub fn plan_linked_plugin_builds(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
) -> Vec<PluginBuildPlan> {
    let mut links: Vec<_> = dev_links.iter().collect();
    links.sort_by(|(a, _), (b, _)| a.cmp(b));

    links
        .into_iter()
        .map(|(plugin_id, path)| {
            let has_cargo = path.join("Cargo.toml").is_file();
            let last_built_fingerprint = known_fingerprints.get(plugin_id).cloned();

            if !has_cargo {
                return PluginBuildPlan {
                    plugin_id: plugin_id.clone(),
                    path: path.clone(),
                    has_cargo,
                    needs_rebuild: false,
                    current_fingerprint: None,
                    last_built_fingerprint,
                    reason: "Cargo.toml missing".to_string(),
                };
            }

            match fingerprint_plugin(path) {
                Ok(current_fingerprint) => {
                    let needs_rebuild = last_built_fingerprint
                        .as_ref()
                        .map(|known| known != &current_fingerprint)
                        .unwrap_or(true);
                    let reason = if needs_rebuild {
                        if last_built_fingerprint.is_some() {
                            "Source changed".to_string()
                        } else {
                            "No successful build recorded".to_string()
                        }
                    } else {
                        "Up to date".to_string()
                    };

                    PluginBuildPlan {
                        plugin_id: plugin_id.clone(),
                        path: path.clone(),
                        has_cargo,
                        needs_rebuild,
                        current_fingerprint: Some(current_fingerprint),
                        last_built_fingerprint,
                        reason,
                    }
                }
                Err(error) => PluginBuildPlan {
                    plugin_id: plugin_id.clone(),
                    path: path.clone(),
                    has_cargo,
                    needs_rebuild: true,
                    current_fingerprint: None,
                    last_built_fingerprint,
                    reason: format!("Fingerprint unavailable: {}", error),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::core::BuildStatus;
    use std::fs;
    use std::path::Path;
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
