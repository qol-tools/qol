#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
pub(super) use fallback::run_in_place;
#[cfg(unix)]
pub(super) use unix::run_in_place;
#[cfg(windows)]
pub(super) use windows::run_in_place;
