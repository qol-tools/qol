//! Feature flags hide work that is not ready for everyone. Every flag is off
//! until a developer turns it on in the `qol dev` console (ctrl+f), which saves
//! the enabled ids in its console state file.

use serde::Deserialize;

pub struct FeatureFlag {
    pub id: &'static str,
    pub label: &'static str,
}

pub const PLUGIN_SOURCES: FeatureFlag = FeatureFlag {
    id: "plugin.sources",
    label: "Add plugin sources",
};

pub const ALL: &[FeatureFlag] = &[PLUGIN_SOURCES];

#[derive(Default, Deserialize)]
struct EnabledFlags {
    #[serde(default)]
    feature_flags: Vec<String>,
}

pub fn enabled(flag: &FeatureFlag) -> bool {
    super::dev_console_state_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|state| enabled_in(&state, flag))
}

fn enabled_in(state: &str, flag: &FeatureFlag) -> bool {
    serde_json::from_str::<EnabledFlags>(state)
        .unwrap_or_default()
        .feature_flags
        .iter()
        .any(|id| id == flag.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_is_on_only_when_the_console_state_names_it() {
        let cases = [
            (r#"{"feature_flags":["plugin.sources"]}"#, true),
            (r#"{"feature_flags":["trace.details"]}"#, false),
            (r#"{"filters":{}}"#, false),
            ("not json", false),
            ("", false),
        ];
        for (state, expected) in cases {
            assert_eq!(enabled_in(state, &PLUGIN_SOURCES), expected, "{state}");
        }
    }
}
