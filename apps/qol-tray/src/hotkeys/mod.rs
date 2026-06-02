mod capture;
mod catalog;
mod listener;
mod manager;
mod parser;
mod planning;
mod store;
#[cfg(test)]
mod tests;
mod types;

mod registration_status;

pub use listener::{start_hotkey_listener, trigger_reload};
pub use manager::HotkeyManager;
pub use registration_status::{get_registration_errors, RegistrationError};
pub use types::{HotkeyAction, HotkeyBinding, HotkeyConfig};

pub fn start_capture(
    plugin_manager: std::sync::Arc<std::sync::Mutex<crate::plugins::PluginManager>>,
) -> anyhow::Result<()> {
    use crate::plugins::action_executor;

    let manager = HotkeyManager::new()?;
    let config = manager.load_config().unwrap_or_else(|error| {
        log::error!(
            "hotkey config failed to load; starting with no hotkeys until corrected: {error:#}"
        );
        HotkeyConfig::default()
    });
    let bindings: Vec<capture::Binding> = config
        .hotkeys
        .into_iter()
        .filter(|h| h.enabled)
        .map(|h| capture::Binding {
            combo: capture::parse_combo(&h.key),
            plugin_id: h.plugin_id,
            action: h.action,
            raw_key: h.key,
        })
        .collect();

    let plugin_manager = plugin_manager.clone();
    capture::install(
        bindings,
        Box::new(move |binding| {
            action_executor::execute_action(&plugin_manager, &binding.plugin_id, &binding.action);
        }),
    )
}
