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

pub(crate) fn build_capture_bindings(config: HotkeyConfig) -> Vec<capture::Binding> {
    config
        .hotkeys
        .into_iter()
        .filter(|h| h.enabled)
        .map(|h| capture::Binding {
            combo: capture::parse_combo(&h.key),
            plugin_id: h.plugin_id,
            action: h.action,
            raw_key: h.key,
        })
        .collect()
}

pub fn start_capture(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) -> anyhow::Result<()> {
    use crate::plugins::action_executor;

    let bindings = load_bindings_for_capture()?;

    let plugin_manager_for_fire = plugin_manager.clone();
    let on_fire: capture::OnFire = Box::new(move |binding| {
        action_executor::execute_action(
            &plugin_manager_for_fire,
            &binding.plugin_id,
            &binding.action,
        );
    });

    let reload_rx = reload::subscribe();
    let rebuild: capture::RebuildBindings =
        Box::new(|| load_bindings_for_capture().unwrap_or_default());

    capture::install(bindings, on_fire, reload_rx, rebuild)
}

fn load_bindings_for_capture() -> anyhow::Result<Vec<capture::Binding>> {
    let manager = HotkeyManager::new()?;
    let config = manager.load_config().unwrap_or_else(|error| {
        log::error!(
            "hotkey config failed to load; starting with no hotkeys until corrected: {error:#}"
        );
        HotkeyConfig::default()
    });
    Ok(build_capture_bindings(config))
}
