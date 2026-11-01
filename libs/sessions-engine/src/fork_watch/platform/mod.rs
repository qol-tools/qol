#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
pub(super) use fallback::{is_trusted_tool, is_users, keeper_of};
#[cfg(unix)]
pub(super) use unix::{is_trusted_tool, is_users, keeper_of};
#[cfg(windows)]
pub(super) use windows::{is_trusted_tool, is_users, keeper_of};
