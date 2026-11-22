use serde::{Deserialize, Serialize};
use std::path::Path;

pub struct FeatureFlag {
    pub id: &'static str,
    pub label: &'static str,
}

pub const PLUGIN_SOURCES: FeatureFlag = FeatureFlag {
    id: "plugin.sources",
    label: "Add plugin sources",
};

pub const PULL_REQUEST_BUILDS: FeatureFlag = FeatureFlag {
    id: "updates.pull_request_builds",
    label: "Try pull request builds",
};

pub const ALL: &[FeatureFlag] = &[PLUGIN_SOURCES, PULL_REQUEST_BUILDS];

pub const FILE: &str = "feature-flags.json";

#[derive(Default, Serialize, Deserialize)]
struct StoredFlags {
    #[serde(default)]
    enabled: Vec<String>,
}

pub fn enabled_in(core_dir: &Path, flag: &FeatureFlag) -> bool {
    load(core_dir)
        .unwrap_or_default()
        .iter()
        .any(|id| id == flag.id)
}

pub fn load(core_dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(core_dir.join(FILE)).ok()?;
    Some(parse(&text))
}

pub fn save(core_dir: &Path, ids: &[String]) -> std::io::Result<()> {
    let stored = StoredFlags {
        enabled: ids.to_vec(),
    };
    let json = serde_json::to_string_pretty(&stored).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(core_dir)?;
    qol_fs::atomic_write(&core_dir.join(FILE), json.as_bytes())
}

fn parse(text: &str) -> Vec<String> {
    serde_json::from_str::<StoredFlags>(text)
        .unwrap_or_default()
        .enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_is_on_only_when_the_profile_names_it() {
        let cases = [
            (r#"{"enabled":["plugin.sources"]}"#, true),
            (r#"{"enabled":["trace.details"]}"#, false),
            (r#"{"other":[]}"#, false),
            ("not json", false),
            ("", false),
        ];
        for (text, expected) in cases {
            let on = parse(text).iter().any(|id| id == PLUGIN_SOURCES.id);
            assert_eq!(on, expected, "{text}");
        }
    }

    #[test]
    fn saved_flags_load_back_and_a_missing_file_loads_none() {
        let dir = tempfile::tempdir().unwrap();
        let core = dir.path().join("core");
        assert_eq!(load(&core), None);
        assert!(!enabled_in(&core, &PULL_REQUEST_BUILDS));
        save(&core, &[PULL_REQUEST_BUILDS.id.to_string()]).unwrap();
        assert_eq!(load(&core), Some(vec![PULL_REQUEST_BUILDS.id.to_string()]));
        assert!(enabled_in(&core, &PULL_REQUEST_BUILDS));
        assert!(!enabled_in(&core, &PLUGIN_SOURCES));
    }
}
