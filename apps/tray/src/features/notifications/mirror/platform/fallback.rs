use super::super::{ActivateOutcome, Mechanism};

pub(in super::super) fn apply(_wanted: bool) -> Vec<Mechanism> {
    Vec::new()
}

pub(in super::super) fn activate(_id: &str) -> anyhow::Result<ActivateOutcome> {
    Ok(ActivateOutcome::Missing)
}

pub(in super::super) fn is_safe_banner_id(_id: &str) -> bool {
    false
}
