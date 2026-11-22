use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::filters::ViewFilters;
use super::TraceRate;

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ConsoleState {
    #[serde(default)]
    pub(super) filters: ViewFilters,
    #[serde(default)]
    pub(super) trace_details: bool,
    #[serde(default)]
    pub(super) trace_rate: TraceRate,
    #[serde(default)]
    pub(super) keys_hidden: bool,
    #[serde(skip)]
    pub(super) feature_flags: Vec<String>,
    #[serde(
        rename = "feature_flags",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(super) unmigrated_flags: Vec<String>,
}

fn profile_core_dir() -> Option<PathBuf> {
    let root = qol_config::config_dir()?.join(qol_plugin_api::profile::PROFILE_DIR);
    Some(qol_plugin_api::profile::active_core_dir(&root))
}

pub(super) fn load_console_state() -> ConsoleState {
    load_from(
        qol_config::dev_console_state_path().as_deref(),
        profile_core_dir().as_deref(),
    )
}

pub(super) fn save_console_state(state: &ConsoleState) {
    if let Some(path) = qol_config::dev_console_state_path() {
        save_to(&path, state);
    }
}

pub(super) fn save_feature_flag_changes(
    seed: &[String],
    changes: &[(&str, bool)],
) -> io::Result<Vec<String>> {
    let core = profile_core_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    apply_flag_changes(&core, seed, changes)
}

fn load_from(console: Option<&Path>, core: Option<&Path>) -> ConsoleState {
    let mut state: ConsoleState = console
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default();
    let legacy = std::mem::take(&mut state.unmigrated_flags);
    if let Some(flags) = core.and_then(qol_config::feature_flags::load) {
        state.feature_flags = flags;
        return state;
    }
    let migrated = !legacy.is_empty()
        && core.is_some_and(|core| qol_config::feature_flags::save(core, &legacy).is_ok());
    state.feature_flags = legacy.clone();
    if !migrated {
        state.unmigrated_flags = legacy;
    }
    state
}

fn save_to(path: &Path, state: &ConsoleState) {
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    let _ = qol_fs::atomic_write(path, json.as_bytes());
}

fn apply_flag_changes(
    core: &Path,
    seed: &[String],
    changes: &[(&str, bool)],
) -> io::Result<Vec<String>> {
    let mut ids = qol_config::feature_flags::load(core).unwrap_or_else(|| seed.to_vec());
    for (id, on) in changes {
        ids.retain(|known| known != id);
        if *on {
            ids.push((*id).to_string());
        }
    }
    qol_config::feature_flags::save(core, &ids)?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dev_console::filters::FilterStrategy;
    use crate::dev_console::testkit::*;
    use crate::dev_console::*;

    #[test]
    fn console_state_round_trips_through_json() {
        let state = ConsoleState {
            filters: ViewFilters {
                logs: vec![log_filter(FilterStrategy::Exclude, "noise")],
                trace: vec![log_filter(FilterStrategy::Include, "focus")],
                emu: Vec::new(),
            },
            trace_details: true,
            trace_rate: TraceRate::Realtime,
            keys_hidden: true,
            feature_flags: vec!["plugin.sources".to_string()],
            unmigrated_flags: Vec::new(),
        };
        let json = serde_json::to_string(&state).expect("serialize console state");
        let back: ConsoleState = serde_json::from_str(&json).expect("deserialize console state");
        assert_eq!(back.filters, state.filters);
        assert!(back.trace_details);
        assert_eq!(back.trace_rate, TraceRate::Realtime);
        assert!(back.keys_hidden);
        assert!(!json.contains("feature_flags"), "{json}");
    }

    #[test]
    fn console_state_defaults_every_missing_field() {
        let state: ConsoleState = serde_json::from_str("{}").expect("empty object deserializes");
        assert!(state.filters.logs.is_empty());
        assert!(state.filters.trace.is_empty());
        assert!(!state.trace_details);
        assert_eq!(state.trace_rate, TraceRate::Relaxed);
        assert!(!state.keys_hidden);
        assert!(state.feature_flags.is_empty());
    }

    struct Places {
        _root: tempfile::TempDir,
        console: PathBuf,
        core: PathBuf,
    }

    fn places(console_json: Option<&str>) -> Places {
        let root = tempfile::tempdir().unwrap();
        let console = root.path().join("console.json");
        let core = root.path().join("profile").join("default").join("core");
        if let Some(text) = console_json {
            fs::write(&console, text).unwrap();
        }
        Places {
            _root: root,
            console,
            core,
        }
    }

    #[test]
    fn legacy_flags_move_to_the_profile_on_load_and_leave_console_json() {
        let at = places(Some(
            r#"{"keys_hidden":true,"feature_flags":["plugin.sources"]}"#,
        ));
        let state = load_from(Some(&at.console), Some(&at.core));
        assert_eq!(state.feature_flags, ["plugin.sources"]);
        assert!(state.unmigrated_flags.is_empty());
        assert_eq!(
            qol_config::feature_flags::load(&at.core),
            Some(vec!["plugin.sources".to_string()])
        );
        save_to(&at.console, &state);
        let saved = fs::read_to_string(&at.console).unwrap();
        assert!(!saved.contains("feature_flags"), "{saved}");
        assert!(saved.contains("keys_hidden"), "{saved}");
    }

    #[test]
    fn the_profile_flags_win_over_console_json() {
        let at = places(Some(r#"{"feature_flags":["plugin.sources"]}"#));
        qol_config::feature_flags::save(&at.core, &["updates.pull_request_builds".to_string()])
            .unwrap();
        let state = load_from(Some(&at.console), Some(&at.core));
        assert_eq!(state.feature_flags, ["updates.pull_request_builds"]);
        assert!(state.unmigrated_flags.is_empty());
    }

    #[test]
    fn flags_that_could_not_move_stay_in_console_json() {
        let at = places(Some(r#"{"feature_flags":["plugin.sources"]}"#));
        fs::write(
            at.core.parent().unwrap().parent().unwrap(),
            "a file, not a folder",
        )
        .unwrap();
        let state = load_from(Some(&at.console), Some(&at.core));
        assert_eq!(state.feature_flags, ["plugin.sources"]);
        assert_eq!(state.unmigrated_flags, ["plugin.sources"]);
        save_to(&at.console, &state);
        let saved = fs::read_to_string(&at.console).unwrap();
        assert!(saved.contains("plugin.sources"), "{saved}");
    }

    #[test]
    fn a_flag_change_keeps_what_the_synced_file_holds() {
        let at = places(None);
        qol_config::feature_flags::save(
            &at.core,
            &[
                "from.another.machine".to_string(),
                "plugin.sources".to_string(),
            ],
        )
        .unwrap();
        let ids = apply_flag_changes(
            &at.core,
            &[],
            &[
                ("plugin.sources", false),
                ("updates.pull_request_builds", true),
            ],
        )
        .unwrap();
        assert_eq!(ids, ["from.another.machine", "updates.pull_request_builds"]);
        assert_eq!(qol_config::feature_flags::load(&at.core), Some(ids));
    }

    #[test]
    fn a_flag_change_without_a_profile_file_starts_from_the_seed() {
        let at = places(None);
        let seed = ["plugin.sources".to_string()];
        let ids =
            apply_flag_changes(&at.core, &seed, &[("updates.pull_request_builds", true)]).unwrap();
        assert_eq!(ids, ["plugin.sources", "updates.pull_request_builds"]);
    }
}
