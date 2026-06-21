use crate::manifest::{ActionCatalog, ActionDeclaration, ActionType};
use anyhow::{bail, Result};
use std::collections::BTreeSet;

pub(super) fn validate_action_catalog(actions: &ActionCatalog) -> Result<BTreeSet<String>> {
    let mut executable = BTreeSet::new();

    for (action_id, action) in actions {
        validate_action_id(action_id)?;
        validate_action_declaration(action_id, action)?;
        if action.kind.is_executable() {
            executable.insert(action_id.clone());
        }
    }

    Ok(executable)
}

fn validate_action_id(action_id: &str) -> Result<()> {
    if super::command_rules::is_valid_action_id(action_id) {
        return Ok(());
    }

    bail!("action catalog contains invalid action id {:?}", action_id)
}

fn validate_action_declaration(action_id: &str, action: &ActionDeclaration) -> Result<()> {
    if action.label.trim().is_empty() {
        bail!("action catalog entry {:?} has an empty label", action_id);
    }

    validate_args(action_id, action.args.as_deref())?;
    validate_kind_contract(action_id, action)
}

fn validate_args(action_id: &str, args: Option<&[String]>) -> Result<()> {
    let Some(args) = args else {
        return Ok(());
    };

    if args.iter().all(|arg| !arg.contains('\0')) {
        return Ok(());
    }

    bail!("action catalog args for {:?} contain null bytes", action_id)
}

fn validate_kind_contract(action_id: &str, action: &ActionDeclaration) -> Result<()> {
    match action.kind {
        ActionType::Run | ActionType::Settings => {
            if action.config_key.is_none() {
                return Ok(());
            }
            bail!(
                "action catalog entry {:?} cannot set config_key unless kind is toggle-config",
                action_id
            )
        }
        ActionType::ToggleConfig => validate_toggle_config(action_id, action),
    }
}

fn validate_toggle_config(action_id: &str, action: &ActionDeclaration) -> Result<()> {
    if action.args.is_some() {
        bail!(
            "action catalog entry {:?} cannot set args when kind is toggle-config",
            action_id
        );
    }

    if action
        .config_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
    {
        return Ok(());
    }

    bail!(
        "action catalog entry {:?} must set config_key when kind is toggle-config",
        action_id
    )
}
