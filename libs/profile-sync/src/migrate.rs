//! Profile schema repairs applied on the sync path.
//!
//! Both sync entry points must run the same migrations before pushing so a
//! profile that still carries legacy plugin ids is normalized identically no
//! matter which side touches it first.

use anyhow::{anyhow, Result};
use qol_migrations::FileMigration;
use std::path::Path;

/// Runs the plugin schema repairs the tray applies on every sync. Returns
/// whether a repair was actually applied.
pub fn repair_profile_schema(profile_root: &Path) -> Result<bool> {
    let config_dir = profile_root.parent().ok_or_else(|| {
        anyhow!(
            "profile repo {} has no config parent",
            profile_root.display()
        )
    })?;
    let mut repaired = false;
    let uid = qol_migrations::V3_19ToV3_20PluginUid::default_for_production();
    if uid.applies(config_dir)? {
        uid.migrate(config_dir, config_dir)?;
        repaired = true;
    }
    let rename = qol_migrations::V3_21ToV3_22PluginIdRename::new();
    if rename.applies(config_dir)? {
        rename.migrate(config_dir, config_dir)?;
        repaired = true;
    }
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    const LIGHTS_UID: &str = "368871df-de60-4a7a-ab7c-8d33fcd22511";

    fn profile_root(dir: &Path) -> PathBuf {
        let root = dir.join("profile");
        let default = root.join("default");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("manifest.json"), b"{\"version\":1}").unwrap();
        root
    }

    fn write_lock(root: &Path, entries: Value) {
        let path = root.join("default/core/plugins.lock.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lock = json!({"version": 1, "plugins": entries});
        std::fs::write(&path, lock.to_string()).unwrap();
    }

    fn read_lock(root: &Path) -> Value {
        let path = root.join("default/core/plugins.lock.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn renames_old_plugin_ids_and_reports_a_repair() {
        let dir = tempfile::tempdir().unwrap();
        let root = profile_root(dir.path());
        write_lock(
            &root,
            json!([{"uid": LIGHTS_UID, "id": "plugin-lights", "version": "1.0.0"}]),
        );

        assert!(repair_profile_schema(&root).unwrap());

        let lock = read_lock(&root);
        assert_eq!(lock["plugins"][0]["id"], "qol-lights");
        assert_eq!(lock["plugins"][0]["uid"], LIGHTS_UID);
        assert!(!repair_profile_schema(&root).unwrap());
    }

    #[test]
    fn still_runs_the_uid_repair() {
        let dir = tempfile::tempdir().unwrap();
        let root = profile_root(dir.path());
        write_lock(&root, json!([{"id": "qol-lights", "version": "1.0.0"}]));

        assert!(repair_profile_schema(&root).unwrap());

        let lock = read_lock(&root);
        assert_eq!(lock["plugins"][0]["uid"], "qol-lights");
        assert_eq!(lock["plugins"][0]["id"], "qol-lights");
        assert!(!repair_profile_schema(&root).unwrap());
    }

    #[test]
    fn is_a_no_op_when_nothing_is_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let root = profile_root(dir.path());
        write_lock(
            &root,
            json!([{"uid": LIGHTS_UID, "id": "qol-lights", "version": "1.0.0"}]),
        );

        assert!(!repair_profile_schema(&root).unwrap());
    }
}
