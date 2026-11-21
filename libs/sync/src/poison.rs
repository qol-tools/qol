use std::fmt;
use std::panic::Location;
use std::sync::{LockResult, PoisonError, TryLockError, TryLockResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WouldBlock;

impl fmt::Display for WouldBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the lock is held elsewhere")
    }
}

impl std::error::Error for WouldBlock {}

pub(crate) fn report(caller: &Location<'_>) {
    log::error!("recovered a poisoned lock at {caller}");
}

pub(crate) fn release<G>(caller: &Location<'_>, poisoned: PoisonError<G>, clear: impl FnOnce()) {
    clear();
    drop(poisoned);
    report(caller);
}

pub(crate) fn acquire<G>(
    caller: &Location<'_>,
    mut attempt: impl FnMut() -> LockResult<G>,
    clear: impl Fn(),
) -> G {
    loop {
        match attempt() {
            Ok(guard) => return guard,
            Err(poisoned) => release(caller, poisoned, &clear),
        }
    }
}

pub(crate) fn try_acquire<G>(
    caller: &Location<'_>,
    mut attempt: impl FnMut() -> TryLockResult<G>,
    clear: impl Fn(),
) -> Result<G, WouldBlock> {
    loop {
        match attempt() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => release(caller, poisoned, &clear),
            Err(TryLockError::WouldBlock) => return Err(WouldBlock),
        }
    }
}
