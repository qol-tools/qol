#[cfg(not(any(unix, target_os = "windows")))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(unix, target_os = "windows")))]
pub use fallback::{adopt_handed_off_fds, prepare_for_exec};
#[cfg(not(any(unix, target_os = "windows")))]
pub(crate) use fallback::{register, unregister};
#[cfg(unix)]
pub use unix::{adopt_handed_off_fds, prepare_for_exec};
#[cfg(unix)]
pub(crate) use unix::{register, unregister};
#[cfg(target_os = "windows")]
pub use windows::{adopt_handed_off_fds, prepare_for_exec};
#[cfg(target_os = "windows")]
pub(crate) use windows::{register, unregister};
