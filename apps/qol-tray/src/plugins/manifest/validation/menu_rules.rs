use crate::plugins::manifest::MenuItem;
use anyhow::{bail, Result};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct MenuActionIds {
    all: BTreeSet<String>,
    pub executable: BTreeSet<String>,
}

pub(super) fn collect_menu_action_ids(items: &[MenuItem]) -> Result<MenuActionIds> {
    let mut action_ids = MenuActionIds::default();
    collect_item_slice(items, &mut action_ids)?;
    Ok(action_ids)
}

fn collect_item_slice(items: &[MenuItem], action_ids: &mut MenuActionIds) -> Result<()> {
    for item in items {
        collect_menu_item(item, action_ids)?;
    }
    Ok(())
}

fn collect_menu_item(item: &MenuItem, action_ids: &mut MenuActionIds) -> Result<()> {
    match item {
        MenuItem::Action { id, .. } => collect_executable_action(id, action_ids),
        MenuItem::Checkbox { id, .. } => collect_checkbox_action(id, action_ids),
        MenuItem::Submenu { items, .. } => collect_item_slice(items, action_ids),
        MenuItem::Separator => Ok(()),
    }
}

fn collect_executable_action(id: &str, action_ids: &mut MenuActionIds) -> Result<()> {
    validate_menu_action_id(id, &mut action_ids.all)?;
    action_ids.executable.insert(id.to_string());
    Ok(())
}

fn collect_checkbox_action(id: &str, action_ids: &mut MenuActionIds) -> Result<()> {
    validate_menu_action_id(id, &mut action_ids.all)
}

fn validate_menu_action_id(id: &str, action_ids: &mut BTreeSet<String>) -> Result<()> {
    if !super::command_rules::is_valid_action_id(id) {
        bail!("menu contains invalid action id {:?}", id);
    }

    if action_ids.insert(id.to_string()) {
        return Ok(());
    }

    bail!("menu contains duplicate action id {:?}", id)
}
