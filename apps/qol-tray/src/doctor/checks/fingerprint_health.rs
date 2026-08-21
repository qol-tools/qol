use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use std::collections::HashMap;
use std::path::PathBuf;

const ID: &str = "fingerprint_health";

pub(super) struct FingerprintHealthCheck;

impl DoctorCheck for FingerprintHealthCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Fingerprint health", CheckCategory::Runtime)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let dev_links = crate::plugins::registry::dev_linked_paths(ctx.config_dir());
        let stale = stale_sidecar_builds(&dev_links);
        if stale.is_empty() {
            return CheckReport::ok("all dev-linked build sidecars are fresh".to_string());
        }
        let mut report = CheckReport::warn(
            format!(
                "sidecar pair check failed for dev-linked build(s): {}",
                stale.join(", ")
            ),
            "stale_sidecar",
            Vec::new(),
        );
        report.advice.push(format!(
            "rebuild via `qol dev` or the in-app Recompile button: {}",
            stale.join(", ")
        ));
        report
    }
}

fn stale_sidecar_builds(dev_links: &HashMap<String, PathBuf>) -> Vec<String> {
    let mut stale: Vec<String> = dev_links
        .iter()
        .filter_map(|(id, plugin_dir)| {
            let binary = qol_dev_build::plugin_binary_path(plugin_dir)?;
            let fingerprint = qol_dev_build::fingerprint_plugin(plugin_dir).ok()?;
            (!qol_dev_build::binary_is_fresh(&binary, &fingerprint)).then(|| id.clone())
        })
        .collect();
    stale.sort();
    stale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_workspace_plugin(root: &std::path::Path, id: &str) -> PathBuf {
        std::fs::write(
            root.join("Cargo.toml"),
            format!("[workspace]\nmembers = [\"{id}\"]\n"),
        )
        .unwrap();
        let plugin_dir = root.join(id);
        std::fs::create_dir_all(plugin_dir.join("src")).unwrap();
        std::fs::write(
            plugin_dir.join("Cargo.toml"),
            format!("[package]\nname = \"{id}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        )
        .unwrap();
        std::fs::write(plugin_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"{id}\"\nitems = []\n\n[daemon]\nenabled = true\ncommand = \"{id}\"\n"
            ),
        )
        .unwrap();
        plugin_dir
    }

    fn write_binary(root: &std::path::Path, id: &str) -> PathBuf {
        let binary = root.join("target").join("debug").join(id);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"elf").unwrap();
        binary
    }

    #[test]
    fn fresh_sidecar_pair_reports_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = write_workspace_plugin(tmp.path(), "plugin-a");
        let binary = write_binary(tmp.path(), "plugin-a");
        let fingerprint = qol_dev_build::fingerprint_plugin(&plugin_dir).unwrap();
        qol_dev_build::write_fingerprint_sidecar(&binary, &fingerprint).unwrap();

        let dev_links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        assert!(stale_sidecar_builds(&dev_links).is_empty());
    }

    #[test]
    fn mismatched_sidecar_reports_the_plugin() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = write_workspace_plugin(tmp.path(), "plugin-a");
        let binary = write_binary(tmp.path(), "plugin-a");
        qol_dev_build::write_fingerprint_sidecar(&binary, "stale-hash").unwrap();

        let dev_links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        assert_eq!(
            stale_sidecar_builds(&dev_links),
            vec!["plugin-a".to_string()]
        );
    }

    #[test]
    fn missing_binary_reports_the_plugin() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = write_workspace_plugin(tmp.path(), "plugin-a");

        let dev_links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        assert_eq!(
            stale_sidecar_builds(&dev_links),
            vec!["plugin-a".to_string()]
        );
    }

    #[test]
    fn plugin_without_declared_binary_is_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"plugin-a\"]\n",
        )
        .unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        std::fs::create_dir_all(plugin_dir.join("src")).unwrap();
        std::fs::write(
            plugin_dir.join("Cargo.toml"),
            "[package]\nname = \"plugin-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid = \"plugin-a\"\nname = \"plugin-a\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"plugin-a\"\nitems = []\n",
        )
        .unwrap();

        let dev_links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        assert!(stale_sidecar_builds(&dev_links).is_empty());
    }

    #[test]
    fn non_cargo_plugin_is_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-a");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid = \"plugin-a\"\nname = \"plugin-a\"\ndescription = \"\"\nversion = \"1.0.0\"\n\n[menu]\nlabel = \"plugin-a\"\nitems = []\n\n[daemon]\nenabled = true\ncommand = \"plugin-a\"\n",
        )
        .unwrap();

        let dev_links = HashMap::from([("plugin-a".to_string(), plugin_dir)]);
        assert!(stale_sidecar_builds(&dev_links).is_empty());
    }

    #[test]
    fn results_are_sorted_by_plugin_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut dev_links = HashMap::new();
        for id in ["plugin-b", "plugin-a"] {
            let plugin_dir = write_workspace_plugin(tmp.path(), id);
            dev_links.insert(id.to_string(), plugin_dir);
        }

        assert_eq!(
            stale_sidecar_builds(&dev_links),
            vec!["plugin-a".to_string(), "plugin-b".to_string()]
        );
    }
}
