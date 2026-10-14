use super::super::{ActivateOutcome, MirrorAccess};

pub(in super::super) fn sync(_enabled: bool) {}

pub(in super::super) fn access() -> MirrorAccess {
    MirrorAccess::Unsupported
}

pub(in super::super) fn activate(_id: &str) -> anyhow::Result<ActivateOutcome> {
    Ok(ActivateOutcome::Missing)
}

pub(in super::super) fn is_safe_banner_id(_id: &str) -> bool {
    false
}
