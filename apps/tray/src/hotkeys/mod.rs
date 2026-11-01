pub(crate) mod capture;
mod catalog;
pub(crate) mod hold;
mod hold_resolver;
mod listener;
mod manager;
mod parser;
mod planning;
mod platform;
mod reload;
mod store;
pub(crate) use store::HotkeyLayers;
pub(crate) mod takeover;
#[cfg(test)]
mod tests;
mod types;

mod registration_status;

pub use listener::start_hotkey_listener;
pub use manager::HotkeyManager;
pub use registration_status::{get_registration_errors, RegistrationError};
pub use reload::trigger_reload;
pub use takeover::{restore_all as restore_desktop_bindings, RestoreSummary};
pub use types::{HotkeyAction, HotkeyBinding, HotkeyConfig};

use std::collections::{HashMap, HashSet};

type ContinuousActions = HashSet<(crate::plugins::PluginUid, String)>;

fn chord_id(key: &str) -> Option<u32> {
    parser::parse_hotkey(key).map(|hotkey| hotkey.id())
}

pub(crate) fn duplicate_enabled_chord(config: &HotkeyConfig) -> Option<String> {
    hold::duplicate_slot(config, chord_id)
}

pub(crate) fn invalid_hold(
    config: &HotkeyConfig,
    plugin_manager: &std::sync::Mutex<crate::plugins::PluginManager>,
) -> Option<String> {
    let Ok(continuous) = continuous_actions(plugin_manager) else {
        return Some("Hotkeys cannot be checked right now. Try again.".to_string());
    };
    hold::invalid_hold(config, chord_id, |binding| {
        continuous.contains(&(binding.plugin_uid.clone(), binding.action.clone()))
    })
}

pub fn start_recording(session_id: u64, events: std::sync::Arc<crate::daemon::EventBus>) -> bool {
    capture::start_recording(session_id, events)
}

/// Flush key-ups for everything the capture backend still holds down on its
/// virtual keyboard. Call on every way out of the process that skips the
/// readers' own exit release (exec-restart, graceful shutdown); without it a
/// key that is physically down at that instant stays latched in the desktop.
pub fn release_held_keys() {
    capture::release_held_keys();
}

pub fn cancel_recording(session_id: u64) {
    capture::cancel_recording(session_id);
}

enum ChordRole {
    Leader(hold::HoldPlan),
    Folded,
}

pub(crate) fn build_capture_bindings(
    config: HotkeyConfig,
    continuous_actions: &ContinuousActions,
) -> Vec<capture::Binding> {
    let enabled: Vec<HotkeyBinding> = config
        .hotkeys
        .into_iter()
        .filter(|binding| binding.enabled)
        .collect();
    let combos: Vec<Option<capture::Combo>> = enabled
        .iter()
        .map(|binding| capture::parse_combo(&binding.key))
        .collect();
    let continuous: Vec<bool> = enabled
        .iter()
        .map(|binding| {
            continuous_actions.contains(&(binding.plugin_uid.clone(), binding.action.clone()))
        })
        .collect();

    let mut by_chord: HashMap<capture::Combo, Vec<usize>> = HashMap::new();
    for (index, combo) in combos.iter().enumerate() {
        if let Some(combo) = combo {
            by_chord.entry(combo.clone()).or_default().push(index);
        }
    }
    let mut roles: HashMap<usize, ChordRole> = HashMap::new();
    for indices in by_chord.values() {
        let members: Vec<hold::ChordMember> = indices
            .iter()
            .map(|&index| hold::ChordMember {
                target: hold::Target::of(&enabled[index]),
                hold_ms: enabled[index].hold_ms,
                continuous: continuous[index],
            })
            .collect();
        let Some(timed) = hold::timed_chord(&members) else {
            continue;
        };
        for &index in indices {
            roles.insert(index, ChordRole::Folded);
        }
        roles.insert(indices[timed.leader], ChordRole::Leader(timed.plan));
    }

    enabled
        .into_iter()
        .zip(combos)
        .zip(continuous)
        .enumerate()
        .filter_map(
            |(index, ((binding, combo), continuous))| match roles.remove(&index) {
                Some(ChordRole::Folded) => None,
                Some(ChordRole::Leader(plan)) => {
                    let primary = plan.primary().clone();
                    Some(capture::Binding {
                        combo,
                        plugin_uid: primary.plugin_uid,
                        action: primary.action,
                        raw_key: binding.key,
                        continuous: false,
                        hold: Some(plan),
                    })
                }
                None => Some(capture::Binding {
                    combo,
                    plugin_uid: binding.plugin_uid,
                    action: binding.action,
                    raw_key: binding.key,
                    continuous,
                    hold: None,
                }),
            },
        )
        .collect()
}

pub fn start_capture(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) -> anyhow::Result<()> {
    use crate::plugins::action_executor;

    let bindings = load_bindings_for_capture(&plugin_manager).unwrap_or_else(|error| {
        log::error!(
            "hotkey config failed to load at startup; installing with no hotkeys until corrected: {error:#}"
        );
        Vec::new()
    });

    let plugin_manager_for_fire = plugin_manager.clone();
    let plugin_manager_for_holds = plugin_manager.clone();
    let resolver = hold_resolver::HoldResolver::spawn(move |decision| match decision {
        hold::Fire::Tap(target) => {
            run_one_shot(&plugin_manager_for_holds, &target, "one-shot");
        }
        hold::Fire::Hold(target) => {
            run_one_shot(&plugin_manager_for_holds, &target, "hold");
        }
    })?;
    let on_fire: capture::OnFire = Box::new(move |event| {
        if event.binding.hold.is_some() {
            resolver.feed(event);
            return;
        }
        let target = hold::Target {
            plugin_uid: event.binding.plugin_uid.clone(),
            action: event.binding.action.clone(),
        };
        if event.binding.continuous {
            let plugin_id = match plugin_id_for(&plugin_manager_for_fire, &target) {
                Some(plugin_id) => plugin_id,
                None => return,
            };
            action_executor::execute_action_with_input(
                &plugin_manager_for_fire,
                &plugin_id,
                &event.binding.action,
                serde_json::json!({ "phase": event.phase.as_str() }),
            );
        } else if event.phase == capture::Phase::START {
            run_one_shot(&plugin_manager_for_fire, &target, "one-shot");
        }
    });

    let reload_rx = reload::subscribe();
    let plugin_manager_for_rebuild = plugin_manager.clone();
    let rebuild: capture::RebuildBindings = Box::new(move || {
        let bindings = load_bindings_for_capture(&plugin_manager_for_rebuild)?;
        #[cfg(debug_assertions)]
        trace_native_bindings("reload", &bindings);
        Ok(bindings)
    });

    #[cfg(debug_assertions)]
    let startup_bindings = bindings.clone();
    capture::install(bindings, on_fire, reload_rx, rebuild)?;
    #[cfg(debug_assertions)]
    trace_native_bindings("startup", &startup_bindings);
    Ok(())
}

type SharedPluginManager = std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>;

fn plugin_id_for(plugin_manager: &SharedPluginManager, target: &hold::Target) -> Option<String> {
    let plugin_id = match plugin_manager.lock() {
        Ok(manager) => manager
            .identity_index()
            .display_for(&target.plugin_uid)
            .map(|d| d.id.as_str().to_owned()),
        Err(_) => {
            log::error!("hotkey capture: plugin manager lock failed");
            return None;
        }
    };
    if plugin_id.is_none() {
        log::warn!(
            "hotkey capture: no plugin found for uid {}",
            target.plugin_uid.as_str()
        );
    }
    plugin_id
}

fn run_one_shot(plugin_manager: &SharedPluginManager, target: &hold::Target, kind: &str) {
    let Some(plugin_id) = plugin_id_for(plugin_manager, target) else {
        return;
    };
    qol_runtime::probe!(
        "HOTKEY_DISPATCH",
        "source=native-capture kind={} uid={} action={}",
        kind,
        qol_runtime::probe::token(target.plugin_uid.as_str()),
        qol_runtime::probe::token(&target.action)
    );
    crate::plugins::action_executor::execute_action(plugin_manager, &plugin_id, &target.action);
}

pub fn start_capture_with_fallback(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) {
    match start_capture(plugin_manager.clone()) {
        Ok(()) => {
            log::info!("Hotkey capture: native");
            qol_runtime::probe!("HOTKEY_BACKEND", "backend=native result=ready");
            if capture::KEEP_REGISTERED_ONE_SHOTS {
                listener::start_listener(plugin_manager, manager::RegistrationScope::OneShotBackup);
                qol_runtime::probe!(
                    "HOTKEY_BACKEND",
                    "backend=global-hotkey result=one-shot-backup"
                );
            }
        }
        Err(error) => {
            log::info!("Hotkey capture fallback to global_hotkey ({error})");
            qol_runtime::probe!(
                "HOTKEY_BACKEND",
                "backend=global-hotkey result=fallback reason={}",
                qol_runtime::probe::token(&error.to_string())
            );
            if let Err(error) = start_hotkey_listener(plugin_manager) {
                log::warn!("Failed to start hotkey listener: {}", error);
            } else {
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    trigger_reload();
                });
            }
        }
    }
}

fn load_bindings_for_capture(
    plugin_manager: &std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) -> anyhow::Result<Vec<capture::Binding>> {
    let manager = HotkeyManager::new()?;
    let config = manager.load_config()?;
    let continuous_actions = continuous_actions(plugin_manager)?;
    Ok(build_capture_bindings(config, &continuous_actions))
}

fn continuous_actions(
    plugin_manager: &std::sync::Mutex<crate::plugins::PluginManager>,
) -> anyhow::Result<ContinuousActions> {
    Ok(plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock failed"))?
        .plugins()
        .flat_map(|plugin| {
            let uid = plugin.uid();
            plugin
                .manifest
                .actions
                .iter()
                .filter(|(_, action)| action.continuous)
                .map(move |(action_id, _)| (uid.clone(), action_id.clone()))
        })
        .collect())
}

#[cfg(debug_assertions)]
fn trace_native_bindings(phase: &str, bindings: &[capture::Binding]) {
    qol_runtime::probe!(
        "HOTKEY_BINDINGS",
        "backend=native phase={} result=loaded count={}",
        phase,
        bindings.len()
    );
}
