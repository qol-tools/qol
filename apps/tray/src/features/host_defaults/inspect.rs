use super::catalog;
use super::intent::IntentStore;
use super::service;
use super::store;
use crate::hotkeys::{HotkeyBinding, HotkeyLayers};
use crate::plugins::PluginUid;
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
    let involved: BTreeSet<String> = inspection
        .intents
        .iter()
        .map(|intent| intent.role.clone())
        .chain(records.iter().map(|record| record.role.clone()))
        .collect();
    inspection.dead_keys = dead_keys(&host, &plugins, &involved);
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

fn dead_keys(
    host: &impl HostDefaults,
    plugins: &[catalog::InstalledPlugin],
    involved: &BTreeSet<String>,
) -> Vec<DeadKey> {
    if !crate::doctor::host_bindings_readable() {
        return Vec::new();
    }
    let bindings = match HotkeyLayers::active().and_then(|layers| layers.load()) {
        Ok(config) => config.hotkeys,
        Err(error) => {
            log::warn!("host defaults: hotkey bindings unreadable: {error:#}");
            return Vec::new();
        }
    };
    let routes: Vec<RoleRoute> = Role::ALL
        .into_iter()
        .filter(|role| role.is_slot())
        .filter_map(|role| {
            let declared_by: Vec<PluginUid> = service::declarations_for(role, plugins)
                .into_iter()
                .map(|declaration| declaration.uid)
                .collect();
            if declared_by.is_empty() {
                return None;
            }
            let route = host.slot_route(role)?;
            Some(RoleRoute {
                role,
                route,
                declared_by,
            })
        })
        .collect();
    unanswered(
        &routes,
        involved,
        &bindings,
        crate::doctor::shadowed_combos,
        crate::doctor::normalize_combo,
    )
}

struct RoleRoute {
    role: Role,
    route: SlotRoute,
    declared_by: Vec<PluginUid>,
}

fn unanswered(
    routes: &[RoleRoute],
    involved: &BTreeSet<String>,
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
    for RoleRoute {
        role,
        route,
        declared_by,
    } in routes
    {
        let reached: BTreeSet<String> = route
            .reaches
            .iter()
            .filter_map(|key| normalize(key))
            .collect();
        let set_by_qol = involved.contains(role.id())
            || bindings.iter().any(|binding| {
                binding.enabled
                    && declared_by.contains(&binding.plugin_uid)
                    && normalize(&binding.key).is_some_and(|key| reached.contains(&key))
            });
        if !set_by_qol {
            continue;
        }
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

    fn lower(combo: &str) -> Option<String> {
        Some(combo.to_lowercase())
    }

    fn switcher() -> Vec<RoleRoute> {
        vec![RoleRoute {
            role: Role::SlotWindowSwitcher,
            route: SlotRoute {
                keys: vec!["Alt+Tab".to_string()],
                reaches: vec!["Alt+Tab".to_string(), "Alt+Shift+Tab".to_string()],
                owner: "Cinnamon".to_string(),
            },
            declared_by: vec![PluginUid::new("uid-alt-tab")],
        }]
    }

    fn binding(key: &str, uid: &str, enabled: bool) -> HotkeyBinding {
        HotkeyBinding {
            id: key.to_string(),
            key: key.to_string(),
            plugin_uid: PluginUid::new(uid),
            action: "open".to_string(),
            enabled,
            os_only: false,
        }
    }

    fn roles(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn a_route_key_qol_freed_and_nobody_answers_is_dead() {
        let cases = [
            (
                "the plugin's reverse key freed it",
                vec![binding("Alt+Shift+Tab", "uid-alt-tab", true)],
                roles(&[]),
                true,
                1,
            ),
            (
                "a claim freed it and its binding is off",
                vec![binding("Alt+Tab", "uid-alt-tab", false)],
                roles(&["slot.window_switcher"]),
                true,
                1,
            ),
            (
                "bound by qol",
                vec![binding("alt+tab", "uid-other", true)],
                roles(&["slot.window_switcher"]),
                true,
                0,
            ),
            (
                "the user removed it without qol",
                vec![binding("Alt+Shift+Tab", "uid-other", true)],
                roles(&[]),
                true,
                0,
            ),
            (
                "the host still has it",
                Vec::new(),
                roles(&["slot.window_switcher"]),
                false,
                0,
            ),
        ];
        for (name, bindings, involved, freed, expected) in cases {
            let host_answers = |keys: &[String]| if freed { Vec::new() } else { keys.to_vec() };
            let dead = unanswered(&switcher(), &involved, &bindings, host_answers, lower);
            assert_eq!(dead.len(), expected, "{name}");
        }
    }

    #[test]
    fn a_dead_key_names_the_role_key_and_owner() {
        let dead = unanswered(
            &switcher(),
            &roles(&["slot.window_switcher"]),
            &[],
            |_| Vec::new(),
            lower,
        );
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
