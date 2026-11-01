use super::catalog::AvailableActions;
use super::hold::{timed_chord, ChordMember, HoldPlan, Target};
use super::parser::parse_hotkey;
use super::types::HotkeyBinding;
use super::{HotkeyAction, HotkeyConfig};
use global_hotkey::hotkey::HotKey;
use std::collections::HashMap;

pub(super) struct PlannedRegistration {
    pub(super) binding_key: String,
    pub(super) hotkey: HotKey,
    pub(super) action: HotkeyAction,
    pub(super) hold: Option<HoldPlan>,
}

struct PlannedBinding {
    registration: PlannedRegistration,
    hold_ms: Option<u32>,
}

pub(super) fn plan_registrations(
    config: &HotkeyConfig,
    available_actions: &AvailableActions,
) -> Vec<PlannedRegistration> {
    let planned: Vec<PlannedBinding> = config
        .hotkeys
        .iter()
        .filter_map(|binding| {
            plan_binding(binding, available_actions).map(|registration| PlannedBinding {
                registration,
                hold_ms: binding.hold_ms,
            })
        })
        .collect();
    let mut by_chord: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, binding) in planned.iter().enumerate() {
        by_chord
            .entry(binding.registration.hotkey.id())
            .or_default()
            .push(index);
    }
    let mut folded = vec![false; planned.len()];
    let mut merged: HashMap<usize, HoldPlan> = HashMap::new();
    for indices in by_chord.values() {
        let members: Vec<ChordMember> = indices
            .iter()
            .map(|&index| ChordMember {
                target: target_of(&planned[index].registration.action),
                hold_ms: planned[index].hold_ms,
                continuous: planned[index].registration.action.continuous,
            })
            .collect();
        let Some(timed) = timed_chord(&members) else {
            continue;
        };
        for &index in indices {
            folded[index] = true;
        }
        let leader = indices[timed.leader];
        folded[leader] = false;
        merged.insert(leader, timed.plan);
    }
    planned
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !folded[*index])
        .map(|(index, binding)| match merged.remove(&index) {
            Some(plan) => PlannedRegistration {
                action: plan.primary().hotkey_action(),
                hold: Some(plan),
                ..binding.registration
            },
            None => binding.registration,
        })
        .collect()
}

fn target_of(action: &HotkeyAction) -> Target {
    Target {
        plugin_uid: action.plugin_uid.clone(),
        action: action.action.clone(),
    }
}

fn plan_binding(
    binding: &HotkeyBinding,
    available_actions: &AvailableActions,
) -> Option<PlannedRegistration> {
    if !binding.enabled {
        return None;
    }
    let Some(available_action) = available_action(available_actions, binding) else {
        warn_unavailable_binding(binding);
        return None;
    };

    let hotkey = parse_planned_hotkey(binding)?;
    Some(PlannedRegistration::from_binding(
        binding,
        hotkey,
        *available_action,
    ))
}

fn available_action<'a>(
    available_actions: &'a AvailableActions,
    binding: &HotkeyBinding,
) -> Option<&'a bool> {
    available_actions
        .get(&binding.plugin_uid)
        .and_then(|actions| actions.get(&binding.action))
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
        binding.plugin_uid.as_str(),
        binding.action
    );
}

fn warn_invalid_binding(binding: &HotkeyBinding) {
    log::warn!("Invalid hotkey string: {}", binding.key);
}

impl PlannedRegistration {
    fn from_binding(binding: &HotkeyBinding, hotkey: HotKey, continuous: bool) -> Self {
        Self {
            binding_key: binding.key.clone(),
            hotkey,
            action: HotkeyAction {
                plugin_uid: binding.plugin_uid.clone(),
                action: binding.action.clone(),
                continuous,
            },
            hold: None,
        }
    }
}
