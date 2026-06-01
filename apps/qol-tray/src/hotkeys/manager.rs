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
    applied: HashMap<String, AppliedHotkey>,
    bindings: HashMap<u32, HotkeyAction>,
    config_path: PathBuf,
}

struct AppliedHotkey {
    hotkey: HotKey,
    action: HotkeyAction,
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
        let plan = plan_registrations(config, available_actions);
        if self.manager.is_none() {
            return self.apply_cold_start(plan);
        }
        self.apply_diff(plan);
        Ok(())
    }

    pub fn get_action(&self, event: &GlobalHotKeyEvent) -> Option<&HotkeyAction> {
        self.bindings.get(&event.id())
    }

    fn with_config_path(config_path: PathBuf) -> Self {
        Self {
            manager: None,
            applied: HashMap::new(),
            bindings: HashMap::new(),
            config_path,
        }
    }

    fn apply_cold_start(&mut self, registrations: Vec<PlannedRegistration>) -> Result<()> {
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

    fn apply_diff(&mut self, plan: Vec<PlannedRegistration>) {
        let mut planned_by_key: HashMap<String, PlannedRegistration> = plan
            .into_iter()
            .map(|p| (p.binding_key.clone(), p))
            .collect();

        let existing_keys: Vec<String> = self.applied.keys().cloned().collect();
        for key in existing_keys {
            match planned_by_key.remove(&key) {
                Some(planned) => self.refresh_action_if_changed(&key, planned),
                None => self.drop_binding(&key),
            }
        }

        let mut errors = Vec::new();
        if let Some(manager) = self.manager.as_ref() {
            for (_, registration) in planned_by_key {
                if let Some(error) =
                    register_via(manager, registration, &mut self.applied, &mut self.bindings)
                {
                    errors.push(error);
                }
            }
        }
        registration_status::set_registration_errors(errors);
    }

    fn refresh_action_if_changed(&mut self, key: &str, planned: PlannedRegistration) {
        let Some(existing) = self.applied.get_mut(key) else {
            return;
        };
        if existing.action == planned.action {
            return;
        }
        self.bindings
            .insert(existing.hotkey.id(), planned.action.clone());
        existing.action = planned.action;
        log_registered_hotkey(key, &existing.action);
    }

    fn drop_binding(&mut self, key: &str) {
        let Some(entry) = self.applied.remove(key) else {
            return;
        };
        if let Some(manager) = self.manager.as_ref() {
            if let Err(error) = manager.unregister(entry.hotkey) {
                log::warn!("Failed to unregister hotkey {}: {}", key, error);
            }
        }
        self.bindings.remove(&entry.hotkey.id());
        log_unregistered_hotkey(key);
    }

    fn register_planned_hotkey(
        &mut self,
        manager: &GlobalHotKeyManager,
        registration: PlannedRegistration,
    ) -> Option<RegistrationError> {
        register_via(manager, registration, &mut self.applied, &mut self.bindings)
    }
}

fn register_via(
    manager: &GlobalHotKeyManager,
    registration: PlannedRegistration,
    applied: &mut HashMap<String, AppliedHotkey>,
    bindings: &mut HashMap<u32, HotkeyAction>,
) -> Option<RegistrationError> {
    let PlannedRegistration {
        binding_key,
        hotkey,
        action,
    } = registration;
    if let Err(error) = manager.register(hotkey) {
        let msg = error.to_string();
        log::error!("Failed to register hotkey {}: {}", binding_key, msg);
        if let Err(write_err) = crate::doctor::trigger::mark_needed(
            "hotkey_shadows",
            &format!("{} failed to grab: {}", binding_key, msg),
        ) {
            log::warn!("doctor trigger: mark_needed failed: {}", write_err);
        }
        return Some(RegistrationError {
            key: binding_key,
            error: msg,
        });
    }
    bindings.insert(hotkey.id(), action.clone());
    log_registered_hotkey(&binding_key, &action);
    applied.insert(binding_key, AppliedHotkey { hotkey, action });
    None
}

fn log_registered_hotkey(binding_key: &str, action: &HotkeyAction) {
    log::info!(
        "Registered hotkey: {} -> {}::{}",
        binding_key,
        action.plugin_id,
        action.action
    );
}

fn log_unregistered_hotkey(binding_key: &str) {
    log::info!("Unregistered hotkey: {}", binding_key);
}
