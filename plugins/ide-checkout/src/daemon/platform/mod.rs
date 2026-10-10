#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(super) use unix::{executable_candidates, inherited_listener, is_executable, launch_path};
#[cfg(not(any(unix, windows)))]
pub(super) use unsupported::{
    executable_candidates, inherited_listener, is_executable, launch_path,
};
#[cfg(windows)]
pub(super) use windows::{executable_candidates, inherited_listener, is_executable, launch_path};
