use super::capture::{parse_combo, Combo};
use super::catalog::AvailableActions;
use super::planning::{plan_registrations, PlannedRegistration};
use super::registration_status::{self, RegistrationError};
use super::store::HotkeyLayers;
use super::{HotkeyAction, HotkeyConfig};
use anyhow::Result;
use global_hotkey::hotkey::{HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use qol_hotkeys::grammar::Key;
use std::collections::HashMap;

pub struct HotkeyManager {
    manager: Option<GlobalHotKeyManager>,
    applied: HashMap<String, AppliedHotkey>,
    bindings: HashMap<u32, RegisteredHotkey>,
    layers: HotkeyLayers,
    scope: RegistrationScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RegistrationScope {
    All,
    OneShotBackup,
}

impl RegistrationScope {
    fn admits(self, registration: &PlannedRegistration) -> bool {
        let altgr_chord = registration
            .hotkey
            .mods
            .contains(Modifiers::CONTROL | Modifiers::ALT);
        let layout_symbol = parse_combo(&registration.binding_key)
            .is_some_and(|combo| matches!(combo.key, Key::Symbol(_)));
        self == Self::All || !(registration.action.continuous || altgr_chord || layout_symbol)
    }
}

struct AppliedHotkey {
    hotkey: HotKey,
    action: HotkeyAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RegisteredHotkey {
    pub(super) action: HotkeyAction,
    pub(super) physical_chord: Option<Combo>,
}

impl HotkeyManager {
    pub fn new() -> Result<Self> {
        Ok(Self::with_layers(HotkeyLayers::active()?))
    }

    pub(super) fn with_scope(scope: RegistrationScope) -> Result<Self> {
        Ok(Self {
            scope,
            ..Self::new()?
        })
    }

    pub fn load_config(&self) -> Result<HotkeyConfig> {
        self.layers.load()
    }

    pub fn save_config(&self, config: &HotkeyConfig) -> Result<()> {
        self.layers.save(config)
    }

    pub fn register_hotkeys(
        &mut self,
        config: &HotkeyConfig,
        available_actions: &AvailableActions,
    ) -> Result<()> {
        let mut plan = plan_registrations(config, available_actions);
        plan.retain(|registration| self.scope.admits(registration));
        if self.manager.is_none() {
            return self.apply_cold_start(plan);
        }
        self.apply_diff(plan);
        Ok(())
    }

    pub(super) fn get_registration(&self, event: &GlobalHotKeyEvent) -> Option<&RegisteredHotkey> {
        self.bindings.get(&event.id())
    }

    pub(super) fn continuous_registrations(
        &self,
    ) -> impl Iterator<Item = (u32, &RegisteredHotkey)> {
        self.bindings
            .iter()
            .filter(|(_, registration)| registration.action.continuous)
            .map(|(id, registration)| (*id, registration))
    }

    pub(super) fn release_active_grab(&self) -> Result<()> {
        if let Some(manager) = self.manager.as_ref() {
            super::platform::release_active_grab(manager)?;
        }
        Ok(())
    }

    pub(super) fn reassert_all(&mut self) -> Result<()> {
        let Some(manager) = self.manager.as_ref() else {
            return Ok(());
        };
        let hotkeys: Vec<HotKey> = self.applied.values().map(|entry| entry.hotkey).collect();
        for hotkey in &hotkeys {
            if let Err(error) = manager.unregister(*hotkey) {
                log::warn!(
                    "Failed to release a hotkey grab during re-assert: {}",
                    error
                );
            }
        }
        let mut errors = Vec::new();
        for hotkey in hotkeys {
            if let Err(error) = manager.register(hotkey) {
                errors.push(RegistrationError {
                    key: format!("{:?}", hotkey.key),
                    error: error.to_string(),
                });
            }
        }
        self.publish_errors(errors);
        Ok(())
    }

    fn with_layers(layers: HotkeyLayers) -> Self {
        Self {
            manager: None,
            applied: HashMap::new(),
            bindings: HashMap::new(),
            layers,
            scope: RegistrationScope::All,
        }
    }

    fn publish_errors(&self, errors: Vec<RegistrationError>) {
        match self.scope {
            RegistrationScope::All => registration_status::set_registration_errors(errors),
            RegistrationScope::OneShotBackup => {
                for error in errors {
                    log::debug!(
                        "elevated-window hotkey backup unavailable for {}: {}",
                        error.key,
                        error.error
                    );
                }
            }
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
        self.publish_errors(errors);
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
                if let Some(error) = register_via(
                    manager,
                    registration,
                    self.scope,
                    &mut self.applied,
                    &mut self.bindings,
                ) {
                    errors.push(error);
                }
            }
        }
        self.publish_errors(errors);
    }

    fn refresh_action_if_changed(&mut self, key: &str, planned: PlannedRegistration) {
        let Some(existing) = self.applied.get_mut(key) else {
            return;
        };
        if existing.action == planned.action {
            return;
        }
        self.bindings.insert(
            existing.hotkey.id(),
            RegisteredHotkey {
                action: planned.action.clone(),
                physical_chord: parse_combo(key),
            },
        );
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
        register_via(
            manager,
            registration,
            self.scope,
            &mut self.applied,
            &mut self.bindings,
        )
    }
}

fn register_via(
    manager: &GlobalHotKeyManager,
    registration: PlannedRegistration,
    scope: RegistrationScope,
    applied: &mut HashMap<String, AppliedHotkey>,
    bindings: &mut HashMap<u32, RegisteredHotkey>,
) -> Option<RegistrationError> {
    let PlannedRegistration {
        binding_key,
        hotkey,
        action,
    } = registration;
    if let Err(error) = manager.register(hotkey) {
        let msg = error.to_string();
        if scope == RegistrationScope::OneShotBackup {
            return Some(RegistrationError {
                key: binding_key,
                error: msg,
            });
        }
        log::error!("Failed to register hotkey {}: {}", binding_key, msg);
        qol_runtime::probe!(
            "HOTKEY_REGISTRATION",
            "result=failed key={} uid={} action={} error={}",
            qol_runtime::probe::token(&binding_key),
            qol_runtime::probe::token(action.plugin_uid.as_str()),
            qol_runtime::probe::token(&action.action),
            qol_runtime::probe::token(&msg)
        );
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
    bindings.insert(
        hotkey.id(),
        RegisteredHotkey {
            action: action.clone(),
            physical_chord: parse_combo(&binding_key),
        },
    );
    log_registered_hotkey(&binding_key, &action);
    applied.insert(binding_key, AppliedHotkey { hotkey, action });
    None
}

fn log_registered_hotkey(binding_key: &str, action: &HotkeyAction) {
    log::info!(
        "Registered hotkey: {} -> {}::{}",
        binding_key,
        action.plugin_uid.as_str(),
        action.action
    );
    qol_runtime::probe!(
        "HOTKEY_REGISTRATION",
        "result=registered key={} uid={} action={}",
        qol_runtime::probe::token(binding_key),
        qol_runtime::probe::token(action.plugin_uid.as_str()),
        qol_runtime::probe::token(&action.action)
    );
}

fn log_unregistered_hotkey(binding_key: &str) {
    log::info!("Unregistered hotkey: {}", binding_key);
    qol_runtime::probe!(
        "HOTKEY_REGISTRATION",
        "result=unregistered key={}",
        qol_runtime::probe::token(binding_key)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(key: &str, continuous: bool) -> PlannedRegistration {
        PlannedRegistration {
            binding_key: key.into(),
            hotkey: super::super::parser::parse_hotkey(key).expect("hotkey"),
            action: HotkeyAction {
                plugin_uid: crate::plugins::PluginUid::new("plugin"),
                action: "open".into(),
                continuous,
            },
        }
    }

    #[test]
    fn the_one_shot_backup_admits_only_one_shots_the_hook_resolves_identically() {
        let cases = [
            ("Super+R", false, true),
            ("Shift+Super+J", false, true),
            ("Super+R", true, false),
            ("Ctrl+Alt+E", false, false),
            ("Super+/", false, false),
        ];
        for (key, continuous, expected) in cases {
            let registration = planned(key, continuous);
            assert_eq!(
                RegistrationScope::OneShotBackup.admits(&registration),
                expected,
                "{key} continuous={continuous}"
            );
            assert!(RegistrationScope::All.admits(&registration));
        }
    }
}
