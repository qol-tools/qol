use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::plugins::manifest::PluginManifest;

const ID: &str = "plugin_uid_table";

pub(super) struct PluginUidTableCheck;

impl DoctorCheck for PluginUidTableCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin uid decode table", CheckCategory::Plugins)
    }

    fn run(&self, ctx: &DoctorContext) -> CheckReport {
        let registry = match ctx.registry() {
            Ok(r) => r,
            Err(e) => return CheckReport::ok(format!("could not load registry: {e}")),
        };

        let mut rows: Vec<(String, String, String, bool)> = registry
            .entries
            .iter()
            .filter_map(|entry| {
                let manifest_path = entry.active.path.join("plugin.toml");
                let content = std::fs::read_to_string(&manifest_path).ok()?;
                let manifest: PluginManifest = toml::from_str(&content).ok()?;
                let id = entry.id.clone();
                let name = manifest.plugin.name.clone();
                let (uid, transitional) = match &manifest.plugin.uid {
                    Some(u) => (u.as_str().to_owned(), false),
                    None => (id.clone(), true),
                };
                Some((uid, id, name, transitional))
            })
            .collect();

        rows.sort_by(|a, b| a.1.cmp(&b.1));

        let lines: Vec<String> = rows
            .into_iter()
            .map(|(uid, id, name, transitional)| {
                if transitional {
                    format!("{uid}  {id}  {name}  (transitional: no uid in manifest)")
                } else {
                    format!("{uid}  {id}  {name}")
                }
            })
            .collect();

        if lines.is_empty() {
            return CheckReport::ok("no installed plugins".to_string());
        }

        CheckReport::ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::registry::{Registry, Slot, SlotSource, CURRENT_REGISTRY_VERSION};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_plugin_toml(dir: &std::path::Path, id: &str, uid: Option<&str>, name: &str) {
        let uid_line = match uid {
            Some(u) => format!("uid = \"{u}\"\n"),
            None => String::new(),
        };
        let content = format!(
            "[plugin]\nid = \"{id}\"\n{uid_line}name = \"{name}\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n"
        );
        std::fs::write(dir.join("plugin.toml"), content).unwrap();
    }

    fn registry_with_entries(entries: Vec<(String, PathBuf)>) -> Registry {
        Registry {
            version: CURRENT_REGISTRY_VERSION,
            entries: entries
                .into_iter()
                .map(|(id, path)| crate::plugins::registry::Entry {
                    id,
                    active: Slot {
                        path,
                        source: SlotSource::ReleaseAsset,
                    },
                    fallback: None,
                })
                .collect(),
        }
    }

    #[test]
    fn report_lists_uid_id_name_for_plugins_with_explicit_uid() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-foo");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_plugin_toml(&plugin_dir, "plugin-foo", Some("uid-abc123"), "Foo Plugin");

        let reg_dir = tmp.path().join("cfg");
        std::fs::create_dir_all(&reg_dir).unwrap();
        let registry = registry_with_entries(vec![("plugin-foo".to_string(), plugin_dir)]);
        crate::plugins::registry::save_registry(&reg_dir, &registry).unwrap();

        let ctx = crate::doctor::framework::DoctorContext::with_config_dir(reg_dir);
        let check = PluginUidTableCheck;
        let report = check.run(&ctx);

        assert!(
            report.issues.is_empty(),
            "uid table check must never fail: {:?}",
            report.issues
        );
        assert!(
            report.summary.contains("uid-abc123"),
            "summary must contain uid: {}",
            report.summary
        );
        assert!(
            report.summary.contains("plugin-foo"),
            "summary must contain id: {}",
            report.summary
        );
        assert!(
            report.summary.contains("Foo Plugin"),
            "summary must contain name: {}",
            report.summary
        );
        assert!(
            !report.summary.contains("transitional"),
            "explicit uid must not be marked transitional: {}",
            report.summary
        );
    }

    #[test]
    fn report_marks_transitional_when_plugin_has_no_uid() {
        let tmp = TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugin-bar");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        write_plugin_toml(&plugin_dir, "plugin-bar", None, "Bar Plugin");

        let reg_dir = tmp.path().join("cfg");
        std::fs::create_dir_all(&reg_dir).unwrap();
        let registry = registry_with_entries(vec![("plugin-bar".to_string(), plugin_dir)]);
        crate::plugins::registry::save_registry(&reg_dir, &registry).unwrap();

        let ctx = crate::doctor::framework::DoctorContext::with_config_dir(reg_dir);
        let check = PluginUidTableCheck;
        let report = check.run(&ctx);

        assert!(
            report.issues.is_empty(),
            "transitional uid must not produce issues: {:?}",
            report.issues
        );
        assert!(
            report.summary.contains("plugin-bar"),
            "summary must contain transitional uid (== id): {}",
            report.summary
        );
        assert!(
            report.summary.contains("transitional"),
            "transitional uid must be annotated: {}",
            report.summary
        );
    }

    #[test]
    fn report_rows_are_sorted_by_id() {
        let tmp = TempDir::new().unwrap();
        let dirs: Vec<(&str, &str, &str)> = vec![
            ("plugin-z", "uid-zzz", "Z Plugin"),
            ("plugin-a", "uid-aaa", "A Plugin"),
            ("plugin-m", "uid-mmm", "M Plugin"),
        ];
        let mut entries = Vec::new();
        for (id, uid, name) in &dirs {
            let plugin_dir = tmp.path().join(id);
            std::fs::create_dir_all(&plugin_dir).unwrap();
            write_plugin_toml(&plugin_dir, id, Some(uid), name);
            entries.push((id.to_string(), plugin_dir));
        }

        let reg_dir = tmp.path().join("cfg");
        std::fs::create_dir_all(&reg_dir).unwrap();
        let registry = registry_with_entries(entries);
        crate::plugins::registry::save_registry(&reg_dir, &registry).unwrap();

        let ctx = crate::doctor::framework::DoctorContext::with_config_dir(reg_dir);
        let report = PluginUidTableCheck.run(&ctx);

        let a_pos = report.summary.find("plugin-a").expect("plugin-a missing");
        let m_pos = report.summary.find("plugin-m").expect("plugin-m missing");
        let z_pos = report.summary.find("plugin-z").expect("plugin-z missing");
        assert!(
            a_pos < m_pos && m_pos < z_pos,
            "rows must appear in id-sorted order: {}",
            report.summary
        );
    }

    #[test]
    fn report_is_ok_when_no_plugins_installed() {
        let tmp = TempDir::new().unwrap();
        let ctx =
            crate::doctor::framework::DoctorContext::with_config_dir(tmp.path().to_path_buf());
        let report = PluginUidTableCheck.run(&ctx);
        assert!(report.issues.is_empty());
        assert_eq!(report.summary, "no installed plugins");
    }
}
