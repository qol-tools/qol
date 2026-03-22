use super::catalog::AvailableActions;
use super::planning::{plan_registrations, PlannedRegistration};
use super::registration_status::{self, RegistrationError};
use super::store;
use super::{HotkeyAction, HotkeyConfig};
use anyhow::Result;
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::paths;

pub struct HotkeyManager {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    bindings: HashMap<u32, HotkeyAction>,
    config_path: PathBuf,
}

impl HotkeyManager {
    pub fn new() -> Result<Self> {
        Ok(Self::with_config_path(paths::hotkeys_path()?))
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
        available_actions: &AvailableActions,
    ) -> Result<()> {
        self.unregister_all();
        self.apply_registration_plan(plan_registrations(config, available_actions))
    }

    pub fn get_action(&self, event: &GlobalHotKeyEvent) -> Option<&HotkeyAction> {
        self.bindings.get(&event.id())
    }

    fn with_config_path(config_path: PathBuf) -> Self {
        Self {
            manager: None,
            registered: Vec::new(),
            bindings: HashMap::new(),
            config_path,
        }
    }

    fn apply_registration_plan(&mut self, registrations: Vec<PlannedRegistration>) -> Result<()> {
        let manager = GlobalHotKeyManager::new()?;
        let mut errors = Vec::new();
        for registration in registrations {
            if let Some(error) = self.register_planned_hotkey(&manager, registration) {
                errors.push(error);
            }
        }
        registration_status::set_registration_errors(errors);
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
    ) -> Option<RegistrationError> {
        if let Err(error) = manager.register(registration.hotkey) {
            let msg = error.to_string();
            log::error!(
                "Failed to register hotkey {}: {}",
                registration.binding_key,
                msg
            );
            return Some(RegistrationError {
                key: registration.binding_key,
                error: msg,
            });
        }
        self.store_registration(registration);
        None
    }

    fn store_registration(&mut self, registration: PlannedRegistration) {
        let PlannedRegistration {
            binding_key,
            hotkey,
            action,
        } = registration;

        self.bind_action(&hotkey, &action);
        self.registered.push(hotkey);
        log_registered_hotkey(&binding_key, &action);
    }

    fn try_unregister_registered_hotkeys(&self) {
        let Some(manager) = &self.manager else {
            return;
        };
        if self.registered.is_empty() {
            return;
        }

        log_unregistering_hotkeys(self.registered.len());
        try_unregister_all(manager, &self.registered);
    }

    fn bind_action(&mut self, hotkey: &HotKey, action: &HotkeyAction) {
        self.bindings.insert(hotkey.id(), action.clone());
    }
}

fn log_registered_hotkey(binding_key: &str, action: &HotkeyAction) {
    log::info!(
        "Registered hotkey: {} -> {}::{}",
        binding_key,
        action.plugin_id,
        action.action
    );
}

fn log_unregistering_hotkeys(count: usize) {
    log::info!("Unregistering {} hotkeys", count);
}

fn try_unregister_all(manager: &GlobalHotKeyManager, registered: &[HotKey]) {
    if let Err(error) = manager.unregister_all(registered) {
        log::error!("Failed to unregister hotkeys: {}", error);
    }
}
