mod capture;
mod catalog;
mod grammar;
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

pub(crate) fn build_capture_bindings(config: HotkeyConfig) -> Vec<capture::Binding> {
    config
        .hotkeys
        .into_iter()
        .filter(|h| h.enabled)
        .map(|h| capture::Binding {
            combo: capture::parse_combo(&h.key),
            plugin_uid: h.plugin_uid,
            action: h.action,
            raw_key: h.key,
        })
        .collect()
}

pub fn start_capture(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) -> anyhow::Result<()> {
    use crate::plugins::action_executor;

    let bindings = load_bindings_for_capture().unwrap_or_else(|error| {
        log::error!(
            "hotkey config failed to load at startup; installing with no hotkeys until corrected: {error:#}"
        );
        Vec::new()
    });

    let plugin_manager_for_fire = plugin_manager.clone();
    let on_fire: capture::OnFire = Box::new(move |binding| {
        let plugin_id = match plugin_manager_for_fire.lock() {
            Ok(manager) => manager
                .identity_index()
                .display_for(&binding.plugin_uid)
                .map(|d| d.id.as_str().to_owned()),
            Err(_) => {
                log::error!("hotkey capture: plugin manager lock failed");
                return;
            }
        };
        let Some(plugin_id) = plugin_id else {
            log::warn!(
                "hotkey capture: no plugin found for uid {}",
                binding.plugin_uid.as_str()
            );
            return;
        };
        action_executor::execute_action(&plugin_manager_for_fire, &plugin_id, &binding.action);
    });

    let reload_rx = reload::subscribe();
    let rebuild: capture::RebuildBindings = Box::new(load_bindings_for_capture);

    capture::install(bindings, on_fire, reload_rx, rebuild)
}

fn load_bindings_for_capture() -> anyhow::Result<Vec<capture::Binding>> {
    let manager = HotkeyManager::new()?;
    let config = manager.load_config()?;
    Ok(build_capture_bindings(config))
}
