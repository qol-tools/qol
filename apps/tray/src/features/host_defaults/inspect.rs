use super::catalog;
use super::intent::IntentStore;
use super::store;
use crate::hotkeys::{HotkeyBinding, HotkeyLayers};
use qol_host_defaults::handler::backends::desktop_entry;
use qol_host_defaults::handler::HandlerStore;
use qol_host_defaults::xdg::XdgDirs;
use qol_host_defaults::{host, HostDefaults, Role, SlotRoute, State};
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
    pub dead_keys: Vec<DeadKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeadKey {
    pub role: String,
    pub key: String,
    pub owner: String,
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
        dead_keys: dead_keys(&host, &plugins),
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

fn dead_keys(host: &impl HostDefaults, plugins: &[catalog::InstalledPlugin]) -> Vec<DeadKey> {
    let bindings = match HotkeyLayers::active().and_then(|layers| layers.load()) {
        Ok(config) => config.hotkeys,
        Err(error) => {
            log::warn!("host defaults: hotkey bindings unreadable: {error:#}");
            return Vec::new();
        }
    };
    let routes: Vec<(Role, SlotRoute)> = Role::ALL
        .into_iter()
        .filter(|role| role.is_slot() && !catalog::declaring(*role, plugins).is_empty())
        .filter_map(|role| host.slot_route(role).map(|route| (role, route)))
        .collect();
    unanswered(
        &routes,
        &bindings,
        crate::doctor::shadowed_combos,
        crate::doctor::normalize_combo,
    )
}

fn unanswered(
    routes: &[(Role, SlotRoute)],
    bindings: &[HotkeyBinding],
    host_answers: impl Fn(&[String]) -> Vec<String>,
    normalize: impl Fn(&str) -> Option<String>,
) -> Vec<DeadKey> {
    let bound: BTreeSet<String> = bindings
        .iter()
        .filter(|binding| binding.enabled)
        .filter_map(|binding| normalize(&binding.key))
        .collect();
    let mut dead = Vec::new();
    for (role, route) in routes {
        let answered = host_answers(&route.keys);
        for key in &route.keys {
            let by_qol = normalize(key).is_some_and(|key| bound.contains(&key));
            if !by_qol && !answered.contains(key) {
                dead.push(DeadKey {
                    role: role.id().to_string(),
                    key: key.clone(),
                    owner: route.owner.clone(),
                });
            }
        }
    }
    dead
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginUid;

    fn lower(combo: &str) -> Option<String> {
        Some(combo.to_lowercase())
    }

    fn switcher() -> Vec<(Role, SlotRoute)> {
        vec![(
            Role::SlotWindowSwitcher,
            SlotRoute {
                keys: vec!["Alt+Tab".to_string()],
                reaches: vec!["Alt+Tab".to_string(), "Alt+Shift+Tab".to_string()],
                owner: "Cinnamon".to_string(),
            },
        )]
    }

    fn binding(key: &str, enabled: bool) -> HotkeyBinding {
        HotkeyBinding {
            id: key.to_string(),
            key: key.to_string(),
            plugin_uid: PluginUid::new("uid-alt-tab"),
            action: "open".to_string(),
            enabled,
            os_only: false,
        }
    }

    #[test]
    fn a_route_key_neither_the_host_nor_qol_answers_is_dead() {
        let freed = |_: &[String]| Vec::new();
        let kept = |keys: &[String]| keys.to_vec();
        let cases = [
            (
                "freed and only the reverse key bound",
                vec![binding("Alt+Shift+Tab", true)],
                true,
                1,
            ),
            (
                "freed and bound by qol",
                vec![binding("alt+tab", true)],
                true,
                0,
            ),
            (
                "freed and the binding off",
                vec![binding("Alt+Tab", false)],
                true,
                1,
            ),
            ("the host still has it", Vec::new(), false, 0),
        ];
        for (name, bindings, is_freed, expected) in cases {
            let dead = if is_freed {
                unanswered(&switcher(), &bindings, freed, lower)
            } else {
                unanswered(&switcher(), &bindings, kept, lower)
            };
            assert_eq!(dead.len(), expected, "{name}");
        }
        let dead = unanswered(&switcher(), &[], freed, lower);
        assert_eq!(
            dead,
            vec![DeadKey {
                role: "slot.window_switcher".to_string(),
                key: "Alt+Tab".to_string(),
                owner: "Cinnamon".to_string(),
            }]
        );
    }
}
