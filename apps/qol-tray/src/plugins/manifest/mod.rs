pub use qol_plugin_api::manifest::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse_manifest(s: &str) -> Result<PluginManifest, toml::de::Error> {
        toml::from_str(s)
    }

    #[test]
    fn manifest_parses_optional_uid() {
        let toml = "[plugin]\nid = \"plugin-x\"\nuid = \"u-123\"\nname = \"X\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n";
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.plugin.uid.as_ref().map(|u| u.as_str()), Some("u-123"));

        let toml_no_uid = "[plugin]\nid = \"plugin-x\"\nname = \"X\"\ndescription = \"\"\nversion = \"1.0.0\"\n[menu]\nlabel = \"\"\nitems = []\n";
        assert_eq!(parse_manifest(toml_no_uid).unwrap().plugin.uid, None);
    }

    #[test]
    fn core_plugin_manifests_have_frozen_uid() {
        let plugins_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let entries = std::fs::read_dir(&plugins_dir)
            .unwrap_or_else(|e| panic!("cannot read plugins dir {}: {}", plugins_dir.display(), e));

        let mut checked_count = 0;
        for entry in entries {
            let entry = entry.expect("dir entry");
            let metadata = entry.metadata().expect("dir entry metadata");

            if !metadata.is_dir() {
                continue;
            }

            let folder = entry.file_name();
            let folder_name = folder.to_string_lossy();

            if qol_conventions::is_reserved_plugin_id(&folder_name) {
                continue;
            }

            let manifest_path = entry.path().join("plugin.toml");
            if !manifest_path.exists() {
                panic!("plugin directory '{}' is missing plugin.toml", folder_name);
            }

            let content = std::fs::read_to_string(&manifest_path)
                .unwrap_or_else(|e| panic!("cannot read {}: {}", manifest_path.display(), e));
            let manifest: PluginManifest = toml::from_str(&content)
                .unwrap_or_else(|e| panic!("cannot parse {}: {}", manifest_path.display(), e));

            assert!(
                manifest.plugin.uid.is_some(),
                "plugin '{}' is missing uid in plugin.toml",
                folder_name
            );
            assert!(
                !manifest.plugin.uid.as_ref().unwrap().as_str().is_empty(),
                "plugin '{}' has empty uid in plugin.toml",
                folder_name
            );

            checked_count += 1;
        }

        assert!(
            checked_count >= 11,
            "expected to check at least 11 plugin manifests (excluding plugin-template), but only checked {}",
            checked_count
        );
    }
}
