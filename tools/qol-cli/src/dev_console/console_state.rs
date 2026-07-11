use std::fs;
use std::path::PathBuf;

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
    #[serde(default)]
    pub(super) feature_flags: Vec<String>,
}

fn console_state_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("dev/console.json"))
}

pub(super) fn load_console_state() -> ConsoleState {
    let Some(path) = console_state_path() else {
        return ConsoleState::default();
    };
    let Ok(contents) = fs::read_to_string(&path) else {
        return ConsoleState::default();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

pub(super) fn save_console_state(state: &ConsoleState) {
    let Some(path) = console_state_path() else {
        return;
    };
    let Ok(json) = serde_json::to_string_pretty(state) else {
        return;
    };
    let _ = qol_fs::atomic_write(&path, json.as_bytes());
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
            feature_flags: Vec::new(),
        };
        let json = serde_json::to_string(&state).expect("serialize console state");
        let back: ConsoleState = serde_json::from_str(&json).expect("deserialize console state");
        assert_eq!(back.filters, state.filters);
        assert!(back.trace_details);
        assert_eq!(back.trace_rate, TraceRate::Realtime);
        assert!(back.keys_hidden);
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
}
