use crate::manifest::DefaultFor;
use anyhow::{bail, Result};
use std::collections::BTreeSet;

pub(super) fn validate_default_for(
    declarations: &[DefaultFor],
    executable_action_ids: &BTreeSet<String>,
) -> Result<()> {
    let mut roles = BTreeSet::new();
    for declaration in declarations {
        validate_default_for_entry(declaration, executable_action_ids, &mut roles)?;
    }
    Ok(())
}

fn validate_default_for_entry(
    declaration: &DefaultFor,
    executable_action_ids: &BTreeSet<String>,
    roles: &mut BTreeSet<String>,
) -> Result<()> {
    if declaration.role.trim().is_empty() {
        bail!("default_for.role must not be empty");
    }
    if !executable_action_ids.contains(&declaration.action) {
        bail!(
            "default_for.action references unknown action {:?}",
            declaration.action
        );
    }
    if roles.insert(declaration.role.clone()) {
        return Ok(());
    }

    bail!("default_for contains duplicate role {:?}", declaration.role)
}
