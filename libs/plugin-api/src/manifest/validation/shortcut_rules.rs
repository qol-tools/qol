use crate::manifest::ShortcutDeclaration;
use anyhow::{bail, Result};
use std::collections::BTreeSet;

const MAX_SHORTCUT_NAME_LEN: usize = 128;

pub(super) fn validate_shortcuts(
    shortcuts: &[ShortcutDeclaration],
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    for shortcut in shortcuts {
        validate_shortcut(shortcut, executable_action_ids, &mut ids)?;
    }
    Ok(())
}

fn validate_shortcut(
    shortcut: &ShortcutDeclaration,
    executable_action_ids: &BTreeSet<String>,
    ids: &mut BTreeSet<String>,
) -> Result<()> {
    validate_shortcut_id(&shortcut.id)?;
    validate_shortcut_name(&shortcut.name)?;
    validate_shortcut_action(&shortcut.action, executable_action_ids)?;
    if ids.insert(shortcut.id.clone()) {
        return Ok(());
    }

    bail!("shortcuts contains duplicate id {:?}", shortcut.id)
}

fn validate_shortcut_id(id: &str) -> Result<()> {
    if super::command_rules::is_valid_action_id(id) {
        return Ok(());
    }

    bail!("shortcuts contains invalid id {:?}", id)
}

fn validate_shortcut_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("shortcuts.name must not be empty");
    }
    if name.len() > MAX_SHORTCUT_NAME_LEN {
        bail!(
            "shortcuts.name must be at most {} characters",
            MAX_SHORTCUT_NAME_LEN
        );
    }
    if name.contains('\0') {
        bail!("shortcuts.name must not contain null bytes");
    }
    Ok(())
}

fn validate_shortcut_action(action: &str, executable_action_ids: &BTreeSet<String>) -> Result<()> {
    if !super::command_rules::is_valid_action_id(action) {
        bail!("shortcuts.action contains invalid action id {:?}", action);
    }
    if executable_action_ids.contains(action) {
        return Ok(());
    }

    bail!(
        "shortcuts.action references unknown menu action {:?}",
        action
    )
}
