use crate::plugins::action_executor::{try_execute_action, ActionExecutionError};
use crate::plugins::PluginManager;
use std::sync::{Arc, Mutex, MutexGuard};

const GIVE_BACK_ACTION: &str = "give_back";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GiveBackOutcome {
    GivenBack,
    DaemonNotReady,
    Refused { error: String },
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GiveBackEntry {
    pub plugin_id: String,
    pub outcome: GiveBackOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GiveBackReport {
    entries: Vec<GiveBackEntry>,
}

impl GiveBackReport {
    pub(crate) fn entries(&self) -> &[GiveBackEntry] {
        &self.entries
    }

    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<GiveBackEntry>) -> Self {
        Self { entries }
    }

    pub(crate) fn log_failures(&self, context: &str) {
        for entry in self.entries() {
            match &entry.outcome {
                GiveBackOutcome::GivenBack | GiveBackOutcome::NotApplicable => {}
                GiveBackOutcome::DaemonNotReady => log::warn!(
                    "{context}: plugin {} is not running, so the host state it holds was not given back",
                    entry.plugin_id
                ),
                GiveBackOutcome::Refused { error } => log::warn!(
                    "{context}: plugin {} refused to give back the host state it holds: {error}",
                    entry.plugin_id
                ),
            }
        }
    }
}

pub(crate) fn give_back_all(plugin_manager: &Arc<Mutex<PluginManager>>) -> GiveBackReport {
    let candidates = declared_candidates(plugin_manager);
    give_back_all_with(&candidates, |plugin_id| ask(plugin_manager, plugin_id))
}

pub(crate) fn give_back_one(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
) -> GiveBackReport {
    let declares = declares_give_back(plugin_manager, plugin_id);
    let candidates = vec![(plugin_id.to_string(), declares)];
    give_back_all_with(&candidates, |plugin_id| ask(plugin_manager, plugin_id))
}

fn declared_candidates(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<(String, bool)> {
    let manager = lock_manager(plugin_manager);
    manager
        .plugins()
        .map(|plugin| {
            (
                plugin.id.as_str().to_string(),
                plugin.manifest.actions.contains_key(GIVE_BACK_ACTION),
            )
        })
        .collect()
}

fn declares_give_back(plugin_manager: &Arc<Mutex<PluginManager>>, plugin_id: &str) -> bool {
    let manager = lock_manager(plugin_manager);
    manager
        .get(plugin_id)
        .is_some_and(|plugin| plugin.manifest.actions.contains_key(GIVE_BACK_ACTION))
}

fn lock_manager(plugin_manager: &Arc<Mutex<PluginManager>>) -> MutexGuard<'_, PluginManager> {
    match plugin_manager.lock() {
        Ok(manager) => manager,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn give_back_all_with(
    candidates: &[(String, bool)],
    mut ask: impl FnMut(&str) -> GiveBackOutcome,
) -> GiveBackReport {
    let mut entries = Vec::with_capacity(candidates.len());
    for (plugin_id, declares) in candidates {
        let outcome = if *declares {
            ask(plugin_id)
        } else {
            GiveBackOutcome::NotApplicable
        };
        trace_give_back(plugin_id, &outcome);
        entries.push(GiveBackEntry {
            plugin_id: plugin_id.clone(),
            outcome,
        });
    }
    GiveBackReport { entries }
}

fn ask(plugin_manager: &Arc<Mutex<PluginManager>>, plugin_id: &str) -> GiveBackOutcome {
    match try_execute_action(plugin_manager, plugin_id, GIVE_BACK_ACTION) {
        Ok(()) => GiveBackOutcome::GivenBack,
        Err(ActionExecutionError::DaemonNotReady { .. }) => GiveBackOutcome::DaemonNotReady,
        Err(error) => GiveBackOutcome::Refused {
            error: format!("{error}"),
        },
    }
}

fn outcome_as_str(outcome: &GiveBackOutcome) -> &'static str {
    match outcome {
        GiveBackOutcome::GivenBack => "given_back",
        GiveBackOutcome::DaemonNotReady => "daemon_not_ready",
        GiveBackOutcome::Refused { .. } => "refused",
        GiveBackOutcome::NotApplicable => "not_applicable",
    }
}

fn trace_give_back(plugin_id: &str, outcome: &GiveBackOutcome) {
    let reason = match outcome {
        GiveBackOutcome::Refused { error } => Some(qol_runtime::probe::token(error)),
        _ => None,
    };
    let outcome = outcome_as_str(outcome);
    #[cfg(debug_assertions)]
    {
        match reason.as_deref() {
            Some(reason) => qol_runtime::probe!(
                "GIVE_BACK",
                "plugin={plugin_id} outcome={outcome} reason={reason}"
            ),
            None => qol_runtime::probe!("GIVE_BACK", "plugin={plugin_id} outcome={outcome}"),
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (plugin_id, outcome, reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(pairs: &[(&str, bool)]) -> Vec<(String, bool)> {
        pairs
            .iter()
            .map(|(plugin_id, declares)| (plugin_id.to_string(), *declares))
            .collect()
    }

    #[test]
    fn every_declaring_plugin_is_asked_once() {
        let mut asked = Vec::new();
        let report = give_back_all_with(
            &candidates(&[("plugin-a", true), ("plugin-b", true)]),
            |plugin_id| {
                asked.push(plugin_id.to_string());
                GiveBackOutcome::GivenBack
            },
        );
        assert_eq!(asked, vec!["plugin-a", "plugin-b"]);
        assert_eq!(
            report.entries(),
            &[
                GiveBackEntry {
                    plugin_id: "plugin-a".to_string(),
                    outcome: GiveBackOutcome::GivenBack,
                },
                GiveBackEntry {
                    plugin_id: "plugin-b".to_string(),
                    outcome: GiveBackOutcome::GivenBack,
                },
            ]
        );
    }

    #[test]
    fn a_plugin_that_declares_no_give_back_is_not_asked() {
        let mut asked = Vec::new();
        let report = give_back_all_with(
            &candidates(&[("plugin-a", false), ("plugin-b", true)]),
            |plugin_id| {
                asked.push(plugin_id.to_string());
                GiveBackOutcome::GivenBack
            },
        );
        assert_eq!(asked, vec!["plugin-b"]);
        assert_eq!(report.entries()[0].outcome, GiveBackOutcome::NotApplicable);
        assert_eq!(report.entries()[1].outcome, GiveBackOutcome::GivenBack);
    }

    #[test]
    fn a_daemon_that_is_not_ready_does_not_stop_the_other_plugins() {
        let report = give_back_all_with(
            &candidates(&[("plugin-a", true), ("plugin-b", true)]),
            |plugin_id| {
                if plugin_id == "plugin-a" {
                    GiveBackOutcome::DaemonNotReady
                } else {
                    GiveBackOutcome::GivenBack
                }
            },
        );
        assert_eq!(report.entries()[0].outcome, GiveBackOutcome::DaemonNotReady);
        assert_eq!(report.entries()[1].outcome, GiveBackOutcome::GivenBack);
    }

    #[test]
    fn a_refusing_plugin_reports_the_error_text() {
        let report = give_back_all_with(&candidates(&[("plugin-a", true)]), |_| {
            GiveBackOutcome::Refused {
                error: "the output is busy".to_string(),
            }
        });
        assert_eq!(
            report.entries(),
            &[GiveBackEntry {
                plugin_id: "plugin-a".to_string(),
                outcome: GiveBackOutcome::Refused {
                    error: "the output is busy".to_string(),
                },
            }]
        );
    }

    #[test]
    fn an_unloaded_plugin_is_not_applicable() {
        let plugin_manager = Arc::new(Mutex::new(PluginManager::new()));
        let report = give_back_one(&plugin_manager, "plugin-sound");
        assert_eq!(
            report.entries(),
            &[GiveBackEntry {
                plugin_id: "plugin-sound".to_string(),
                outcome: GiveBackOutcome::NotApplicable,
            }]
        );
    }
}
