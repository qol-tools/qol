use crate::manifest::RuntimeConfig;
use anyhow::{bail, Result};
use std::collections::{BTreeSet, HashMap};

impl RuntimeConfig {
    pub fn validate(&self) -> Result<()> {
        super::command_rules::validate_command_name("runtime.command", &self.command)?;
        validate_runtime_actions(self.actions.as_ref())?;
        Ok(())
    }
}

pub(super) fn validate_optional_runtime_config(
    runtime: Option<&RuntimeConfig>,
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let Some(runtime) = runtime else {
        return Ok(());
    };

    runtime.validate()?;
    validate_runtime_action_coverage(runtime.actions.as_ref(), executable_action_ids)
}

fn validate_runtime_actions(actions: Option<&HashMap<String, Vec<String>>>) -> Result<()> {
    let Some(actions) = actions else {
        return Ok(());
    };

    if actions.is_empty() {
        bail!("runtime.actions cannot be empty when present");
    }

    for (action_id, args) in actions {
        validate_runtime_action(action_id, args)?;
    }
    Ok(())
}

fn validate_runtime_action(action_id: &str, args: &[String]) -> Result<()> {
    if !super::command_rules::is_valid_action_id(action_id) {
        bail!("runtime.actions contains invalid action id {:?}", action_id);
    }

    validate_runtime_args(action_id, args)
}

fn validate_runtime_args(action_id: &str, args: &[String]) -> Result<()> {
    if args.iter().all(|arg| !arg.contains('\0')) {
        return Ok(());
    }

    bail!("runtime.actions for {:?} contains null bytes", action_id)
}

fn validate_runtime_action_coverage(
    actions: Option<&HashMap<String, Vec<String>>>,
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let Some(actions) = actions else {
        return Ok(());
    };

    for action_id in executable_action_ids {
        if actions.contains_key(action_id) {
            continue;
        }

        bail!(
            "runtime.actions missing mapping for menu action {:?}",
            action_id
        );
    }

    Ok(())
}
