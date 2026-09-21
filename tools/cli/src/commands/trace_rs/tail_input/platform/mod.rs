#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::CbreakGuard;
#[cfg(unix)]
pub(super) use unix::CbreakGuard;
