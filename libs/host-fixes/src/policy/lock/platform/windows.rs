use super::super::{lock_name, PolicyError, ResidentPolicy};
use anyhow::Result;

pub(crate) enum HeldLock {}

pub(crate) fn try_acquire(policy: &ResidentPolicy) -> Result<HeldLock> {
    lock_name(policy)?;
    Err(PolicyError::PlatformUnsupported {
        policy: policy.id().to_string(),
    }
    .into())
}
