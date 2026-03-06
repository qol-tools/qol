mod listener;
mod parser;
mod store;
#[cfg(test)]
mod tests;
mod types;

use anyhow::Result;
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::paths;

pub use listener::{start_hotkey_listener, trigger_reload};
use parser::parse_hotkey;
use types::HotkeyBinding;
pub use types::{HotkeyAction, HotkeyConfig};

type AvailableActions = HashMap<String, HashSet<String>>;

pub struct HotkeyManager {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    bindings: HashMap<u32, HotkeyAction>,
    config_path: PathBuf,
}

struct RegistrationPlan {
    registrations: Vec<PlannedRegistration>,
}

struct PlannedRegistration {
    binding_key: String,
    hotkey: HotKey,
    action: HotkeyAction,
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
        let plan = RegistrationPlan::from_config(config, available_actions);
        self.apply_registration_plan(plan)
    }

    fn apply_registration_plan(&mut self, plan: RegistrationPlan) -> Result<()> {
        let manager = GlobalHotKeyManager::new()?;
        for registration in plan.registrations {
            self.register_planned_hotkey(&manager, registration);
        }
        self.manager = Some(manager);
        Ok(())
    }

    fn unregister_all(&mut self) {
        self.try_unregister_registered_hotkeys();
        self.manager = None;
        self.registered.clear();
        self.bindings.clear();
    }

    fn register_planned_hotkey(
        &mut self,
        manager: &GlobalHotKeyManager,
        registration: PlannedRegistration,
    ) {
        if let Err(error) = manager.register(registration.hotkey.clone()) {
            log::error!(
                "Failed to register hotkey {}: {}",
                registration.binding_key,
                error
            );
            return;
        }
        self.store_registration(registration);
    }

    fn store_registration(&mut self, registration: PlannedRegistration) {
        let PlannedRegistration {
            binding_key,
            hotkey,
            action,
        } = registration;

        self.bindings.insert(hotkey.id(), action.clone());
        self.registered.push(hotkey);

        log::info!(
            "Registered hotkey: {} -> {}::{}",
            binding_key,
            action.plugin_id,
            action.action
        );
    }

    fn try_unregister_registered_hotkeys(&self) {
        let Some(manager) = &self.manager else {
            return;
        };
        if self.registered.is_empty() {
            return;
        }

        log::info!("Unregistering {} hotkeys", self.registered.len());

        if let Err(error) = manager.unregister_all(&self.registered) {
            log::error!("Failed to unregister hotkeys: {}", error);
        }
    }

    pub fn get_action(&self, event: &GlobalHotKeyEvent) -> Option<&HotkeyAction> {
        self.bindings.get(&event.id())
    }
}

impl RegistrationPlan {
    fn from_config(config: &HotkeyConfig, available_actions: &AvailableActions) -> Self {
        let mut registrations = Vec::new();

        for binding in &config.hotkeys {
            let Some(registration) = plan_binding(binding, available_actions) else {
                continue;
            };
            registrations.push(registration);
        }

        Self { registrations }
    }
}

impl PlannedRegistration {
    fn new(binding: &HotkeyBinding, hotkey: HotKey) -> Self {
        Self {
            binding_key: binding.key.clone(),
            hotkey,
            action: HotkeyAction {
                plugin_id: binding.plugin_id.clone(),
                action: binding.action.clone(),
            },
        }
    }
}

fn is_binding_available(
    available_actions: &AvailableActions,
    plugin_id: &str,
    action_id: &str,
) -> bool {
    available_actions
        .get(plugin_id)
        .is_some_and(|actions| actions.contains(action_id))
}

fn plan_binding(
    binding: &HotkeyBinding,
    available_actions: &AvailableActions,
) -> Option<PlannedRegistration> {
    if !binding.enabled {
        return None;
    }
    if !is_binding_available(available_actions, &binding.plugin_id, &binding.action) {
        log::warn!(
            "Skipping hotkey {} -> {}::{} (plugin/action unavailable)",
            binding.key,
            binding.plugin_id,
            binding.action
        );
        return None;
    }

    let Some(hotkey) = parse_hotkey(&binding.key) else {
        log::warn!("Invalid hotkey string: {}", binding.key);
        return None;
    };

    Some(PlannedRegistration::new(binding, hotkey))
}
