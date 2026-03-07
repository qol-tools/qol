use super::catalog::AvailableActions;
use super::parser::parse_hotkey;
use super::types::HotkeyBinding;
use super::{HotkeyAction, HotkeyConfig};
use global_hotkey::hotkey::HotKey;

pub(super) struct PlannedRegistration {
    pub(super) binding_key: String,
    pub(super) hotkey: HotKey,
    pub(super) action: HotkeyAction,
}

pub(super) fn plan_registrations(
    config: &HotkeyConfig,
    available_actions: &AvailableActions,
) -> Vec<PlannedRegistration> {
    let mut registrations = Vec::new();

    for binding in &config.hotkeys {
        let Some(registration) = plan_binding(binding, available_actions) else {
            continue;
        };
        registrations.push(registration);
    }

    registrations
}

fn plan_binding(
    binding: &HotkeyBinding,
    available_actions: &AvailableActions,
) -> Option<PlannedRegistration> {
    if !binding.enabled {
        return None;
    }
    if !binding_available(available_actions, binding) {
        warn_unavailable_binding(binding);
        return None;
    }

    let hotkey = parse_planned_hotkey(binding)?;
    Some(PlannedRegistration::from_binding(binding, hotkey))
}

fn binding_available(available_actions: &AvailableActions, binding: &HotkeyBinding) -> bool {
    available_actions
        .get(binding.plugin_id.as_str())
        .is_some_and(|actions| actions.contains(&binding.action))
}

fn parse_planned_hotkey(binding: &HotkeyBinding) -> Option<HotKey> {
    let Some(hotkey) = parse_hotkey(&binding.key) else {
        warn_invalid_binding(binding);
        return None;
    };
    Some(hotkey)
}

fn warn_unavailable_binding(binding: &HotkeyBinding) {
    log::warn!(
        "Skipping hotkey {} -> {}::{} (plugin/action unavailable)",
        binding.key,
        binding.plugin_id,
        binding.action
    );
}

fn warn_invalid_binding(binding: &HotkeyBinding) {
    log::warn!("Invalid hotkey string: {}", binding.key);
}

impl PlannedRegistration {
    fn from_binding(binding: &HotkeyBinding, hotkey: HotKey) -> Self {
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
