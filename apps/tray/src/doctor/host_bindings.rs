use super::checks::shadows_for_combos;
use super::checks::ShadowKind;
use super::de_bindings;
use super::diagnosis::apply_fix;
use anyhow::{bail, Result};
use std::collections::BTreeSet;

pub(crate) fn normalize_combo(combo: &str) -> Option<String> {
    de_bindings::normalize_combo(combo)
}

pub(crate) fn host_bindings_readable() -> bool {
    crate::hotkeys::takeover::scan().available
}

pub(crate) fn shadowed_combos(combos: &[String]) -> Vec<String> {
    shadows_for_combos(combos)
        .into_iter()
        .flat_map(|shadow| shadow.qol_combos)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn free_host_bindings(combos: &[String]) -> Result<()> {
    let mut unfixable = Vec::new();
    for shadow in shadows_for_combos(combos) {
        match &shadow.kind {
            ShadowKind::Fixable(action) => apply_fix(action)?,
            ShadowKind::Reserved { hint } | ShadowKind::RegistrationFailure { hint } => {
                unfixable.push(format!("{}: {hint}", shadow.source_label));
            }
        }
    }
    if unfixable.is_empty() {
        return Ok(());
    }
    bail!("the system keeps a binding on {}", unfixable.join("; "))
}
