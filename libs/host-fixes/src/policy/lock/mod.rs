mod platform;

use super::{PolicyError, ResidentPolicy};
use anyhow::Result;
use std::time::{Duration, Instant};

const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const LOCK_RETRY_WINDOW: Duration = Duration::from_secs(10);

fn lock_retry_window() -> Duration {
    #[cfg(any(test, feature = "sandbox"))]
    if let Ok(ms) = std::env::var("QOL_POLICY_LOCK_RETRY_WINDOW_MS") {
        if let Ok(ms) = ms.parse::<u64>() {
            return Duration::from_millis(ms);
        }
    }
    LOCK_RETRY_WINDOW
}

pub struct PolicyLockGuard {
    _held: platform::HeldLock,
}

impl std::fmt::Debug for PolicyLockGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PolicyLockGuard")
    }
}

pub(crate) fn acquire(policy: &ResidentPolicy) -> Result<PolicyLockGuard> {
    let deadline = Instant::now() + lock_retry_window();
    loop {
        match try_acquire(policy) {
            Ok(guard) => return Ok(guard),
            Err(error) if is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(LOCK_RETRY_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn try_acquire(policy: &ResidentPolicy) -> Result<PolicyLockGuard> {
    platform::try_acquire(policy).map(|held| PolicyLockGuard { _held: held })
}

pub(crate) fn lock_name(policy: &ResidentPolicy) -> Result<String> {
    let base = format!("qol-resident-policy:{}", policy.id());
    #[cfg(any(test, feature = "sandbox"))]
    if let Ok(namespace) = std::env::var("QOL_POLICY_LOCK_NAMESPACE") {
        if !namespace.is_empty()
            && namespace.len() <= 64
            && namespace
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Ok(format!("{base}:{namespace}"));
        }
    }
    Ok(base)
}

fn is_busy(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<PolicyError>(),
        Some(PolicyError::Busy { .. })
    )
}
