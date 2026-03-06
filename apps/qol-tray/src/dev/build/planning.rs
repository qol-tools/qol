pub(in crate::dev::build) mod queue;
mod rebuild_reason;
mod selection;

use std::collections::HashMap;
use std::path::PathBuf;

use super::types::PluginBuildPlan;

pub fn plan_linked_plugin_builds(
    dev_links: &HashMap<String, PathBuf>,
    known_fingerprints: &HashMap<String, String>,
) -> Vec<PluginBuildPlan> {
    selection::select_linked_plugins(dev_links)
        .into_iter()
        .map(|selection| rebuild_reason::plan_selection(selection, known_fingerprints))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev::build::fingerprint::fingerprint_plugin;
    use crate::dev::build::{
        build_linked_plugins_with_progress, load_build_fingerprints, save_build_fingerprints,
    };
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
