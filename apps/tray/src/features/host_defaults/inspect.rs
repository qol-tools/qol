use super::catalog;
use super::intent::IntentStore;
use super::store;
use qol_host_defaults::handler::backends::desktop_entry;
use qol_host_defaults::handler::HandlerStore;
use qol_host_defaults::xdg::XdgDirs;
use qol_host_defaults::{host, HostDefaults, Role, State};
use std::collections::BTreeSet;

const OPENER_ROLE: Role = Role::FolderOpen;

#[derive(Clone, Debug, Default)]
pub(crate) struct Inspection {
    pub ledger_error: Option<String>,
    pub unknown_probes: Vec<UnknownProbe>,
    pub claims: Vec<InspectedClaim>,
    pub installed: BTreeSet<String>,
    pub opener_missing: bool,
    pub intents: Vec<IntendedRole>,
    pub intent_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntendedRole {
    pub role: String,
    pub plugin_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UnknownProbe {
    pub role: String,
    pub plugins: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InspectedClaim {
    pub role: String,
    pub plugin_id: String,
    pub store: String,
    pub key: String,
    pub applied: String,
    pub current: CurrentValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CurrentValue {
    Value(String),
    Absent,
    Unread(String),
    NotRead,
}

pub(crate) fn inspect() -> Inspection {
    let plugins = catalog::installed();
    let host = host();
    let mut inspection = Inspection {
        installed: plugins.iter().map(|plugin| plugin.id.clone()).collect(),
        unknown_probes: unknown_probes(&host, &plugins),
        ..Inspection::default()
    };
    match IntentStore::active().and_then(|store| store.load()) {
        Ok(intents) => {
            inspection.intents = intents
                .roles
                .into_iter()
                .map(|(role, plugin_id)| IntendedRole { role, plugin_id })
                .collect();
        }
        Err(error) => inspection.intent_error = Some(format!("{error:#}")),
    }
    let records = match store::ledger().and_then(|ledger| store::claims(&ledger)) {
        Ok(records) => records,
        Err(error) => {
            inspection.ledger_error = Some(format!("{error:#}"));
            return inspection;
        }
    };
    inspection.opener_missing = opener_missing(&records);
    inspection.claims = records
        .into_iter()
        .map(|record| InspectedClaim {
            current: current_value(&record.store, &record.key),
            role: record.role,
            plugin_id: record.plugin_id,
            store: record.store,
            key: record.key,
            applied: record.applied,
        })
        .collect();
    inspection
}

fn unknown_probes(
    host: &impl HostDefaults,
    plugins: &[catalog::InstalledPlugin],
) -> Vec<UnknownProbe> {
    Role::ALL
        .into_iter()
        .filter_map(|role| {
            let declaring = catalog::declaring(role, plugins);
            if declaring.is_empty() {
                return None;
            }
            match host.probe(role).state {
                State::Unknown { reason } => Some(UnknownProbe {
                    role: role.id().to_string(),
                    plugins: declaring,
                    reason,
                }),
                State::Silent
                | State::NeedsConfirmation { .. }
                | State::NeedsPermission { .. }
                | State::Impossible { .. } => None,
            }
        })
        .collect()
}

fn current_value(store_name: &str, key: &str) -> CurrentValue {
    let Some(handler_store) = HandlerStore::from_name(store_name) else {
        return CurrentValue::NotRead;
    };
    match handler_store.read(key) {
        Ok(Some(value)) => CurrentValue::Value(value),
        Ok(None) => CurrentValue::Absent,
        Err(error) => CurrentValue::Unread(format!("{error:#}")),
    }
}

fn opener_missing(records: &[qol_host_defaults::ClaimRecord]) -> bool {
    let claimed = records.iter().any(|record| {
        record.role == OPENER_ROLE.id() && HandlerStore::from_name(&record.store).is_some()
    });
    if !claimed {
        return false;
    }
    XdgDirs::from_env().is_some_and(|dirs| !desktop_entry::opener_path(&dirs).is_file())
}
