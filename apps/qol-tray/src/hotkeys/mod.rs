mod listener;
mod parser;
mod store;
#[cfg(test)]
mod tests;
mod types;

use crate::paths;
use anyhow::Result;
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub use listener::{start_hotkey_listener, trigger_reload};
use parser::parse_hotkey;
pub use types::{HotkeyAction, HotkeyConfig};

pub struct HotkeyManager {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    bindings: HashMap<u32, HotkeyAction>,
    config_path: PathBuf,
}

impl HotkeyManager {
    pub fn new() -> Result<Self> {
        let config_path = paths::hotkeys_path()?;
        Ok(Self {
            manager: None,
            registered: Vec::new(),
            bindings: HashMap::new(),
            config_path,
        })
    }

    pub fn load_config(&self) -> Result<HotkeyConfig> {
        store::load_config(&self.config_path)
    }

    pub fn save_config(&self, config: &HotkeyConfig) -> Result<()> {
        store::save_config(&self.config_path, config)
    }

    pub fn register_hotkeys(
        &mut self,
        config: &HotkeyConfig,
        available_actions: &HashMap<String, HashSet<String>>,
    ) -> Result<()> {
        self.unregister_all();

        let new_manager = GlobalHotKeyManager::new()?;

        for binding in &config.hotkeys {
            if !binding.enabled {
                continue;
            }

            if !is_binding_available(available_actions, &binding.plugin_id, &binding.action) {
                log::warn!(
                    "Skipping hotkey {} -> {}::{} (plugin/action unavailable)",
                    binding.key,
                    binding.plugin_id,
                    binding.action
                );
                continue;
            }

            let hotkey = match parse_hotkey(&binding.key) {
                Some(hk) => hk,
                None => {
                    log::warn!("Invalid hotkey string: {}", binding.key);
                    continue;
                }
            };

            if let Err(e) = new_manager.register(hotkey) {
                log::error!("Failed to register hotkey {}: {}", binding.key, e);
                continue;
            }

            self.registered.push(hotkey);
            self.bindings.insert(
                hotkey.id(),
                HotkeyAction {
                    plugin_id: binding.plugin_id.clone(),
                    action: binding.action.clone(),
                },
            );

            log::info!(
                "Registered hotkey: {} -> {}::{}",
                binding.key,
                binding.plugin_id,
                binding.action
            );
        }

        self.manager = Some(new_manager);
        Ok(())
    }

    fn unregister_all(&mut self) {
        if let Some(ref manager) = self.manager {
            if !self.registered.is_empty() {
                log::info!("Unregistering {} hotkeys", self.registered.len());
                if let Err(e) = manager.unregister_all(&self.registered) {
                    log::error!("Failed to unregister hotkeys: {}", e);
                }
            }
        }
        self.manager = None;
        self.registered.clear();
        self.bindings.clear();
    }

    pub fn get_action(&self, event: &GlobalHotKeyEvent) -> Option<&HotkeyAction> {
        self.bindings.get(&event.id())
    }
}
fn is_binding_available(
    available_actions: &HashMap<String, HashSet<String>>,
    plugin_id: &str,
    action_id: &str,
) -> bool {
    available_actions
        .get(plugin_id)
        .is_some_and(|actions| actions.contains(action_id))
}
