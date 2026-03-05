mod ignore_pids;
mod platform;

#[cfg(target_os = "macos")]
pub(crate) use ignore_pids::is_ignored_pid;
pub(crate) use ignore_pids::{add_ignore_pid, remove_ignore_pid};
pub(crate) use platform::{create, Platform};
