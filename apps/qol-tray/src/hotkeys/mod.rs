mod capture;
mod catalog;
mod listener;
mod manager;
mod parser;
mod planning;
mod reload;
mod store;
#[cfg(test)]
mod tests;
mod types;

mod registration_status;

pub use listener::start_hotkey_listener;
pub use manager::HotkeyManager;
pub use registration_status::{get_registration_errors, RegistrationError};
pub use reload::trigger_reload;
pub use types::{HotkeyAction, HotkeyBinding, HotkeyConfig};

use std::collections::HashSet;

type ContinuousActions = HashSet<(crate::plugins::PluginUid, String)>;

pub fn start_recording(session_id: u64, events: std::sync::Arc<crate::daemon::EventBus>) -> bool {
    capture::start_recording(session_id, events)
}

pub fn cancel_recording(session_id: u64) {
    capture::cancel_recording(session_id);
}

pub(crate) fn build_capture_bindings(
    config: HotkeyConfig,
    continuous_actions: &ContinuousActions,
) -> Vec<capture::Binding> {
    config
        .hotkeys
        .into_iter()
        .filter(|h| h.enabled)
        .map(|h| {
            let continuous = continuous_actions.contains(&(h.plugin_uid.clone(), h.action.clone()));
            capture::Binding {
                combo: capture::parse_combo(&h.key),
                plugin_uid: h.plugin_uid,
                action: h.action,
                raw_key: h.key,
                continuous,
            }
        })
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
    let on_fire: capture::OnFire = Box::new(move |event| {
        let plugin_id = match plugin_manager_for_fire.lock() {
            Ok(manager) => manager
                .identity_index()
                .display_for(&event.binding.plugin_uid)
                .map(|d| d.id.as_str().to_owned()),
            Err(_) => {
                log::error!("hotkey capture: plugin manager lock failed");
                return;
            }
        };
        let Some(plugin_id) = plugin_id else {
            log::warn!(
                "hotkey capture: no plugin found for uid {}",
                event.binding.plugin_uid.as_str()
            );
            return;
        };
        if event.binding.continuous {
            let phase = match event.phase {
                capture::Phase::Start => "start",
                capture::Phase::Heartbeat => "heartbeat",
                capture::Phase::Stop => "stop",
            };
            action_executor::execute_action_with_input(
                &plugin_manager_for_fire,
                &plugin_id,
                &event.binding.action,
                serde_json::json!({ "phase": phase }),
            );
        } else if event.phase == capture::Phase::Start {
            action_executor::execute_action(
                &plugin_manager_for_fire,
                &plugin_id,
                &event.binding.action,
            );
        }
    });

    let reload_rx = reload::subscribe();
    let plugin_manager_for_rebuild = plugin_manager.clone();
    let rebuild: capture::RebuildBindings =
        Box::new(move || load_bindings_for_capture(&plugin_manager_for_rebuild));

    capture::install(bindings, on_fire, reload_rx, rebuild)
}

pub fn start_capture_with_fallback(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) {
    match start_capture(plugin_manager.clone()) {
        Ok(()) => log::info!("Hotkey capture: native"),
        Err(error) => {
            log::info!("Hotkey capture fallback to global_hotkey ({error})");
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
    let continuous_actions = plugin_manager
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
        .collect();
    Ok(build_capture_bindings(config, &continuous_actions))
}
