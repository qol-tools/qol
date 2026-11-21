use std::fmt;
use std::panic::Location;
use std::sync::PoisonError;

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

pub(crate) fn recover<G>(
    caller: &Location<'_>,
    poisoned: PoisonError<G>,
    clear: impl FnOnce(),
) -> G {
    report(caller);
    clear();
    poisoned.into_inner()
}
