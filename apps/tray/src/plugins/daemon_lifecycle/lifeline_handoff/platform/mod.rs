#[cfg(not(any(unix, target_os = "windows")))]
mod fallback;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(unix, target_os = "windows")))]
pub use fallback::{adopt_handed_off_fds, prepare_for_exec};
#[cfg(unix)]
pub use unix::{adopt_handed_off_fds, prepare_for_exec};
#[cfg(target_os = "windows")]
pub use windows::{adopt_handed_off_fds, prepare_for_exec};
