#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(crate) use linux::{expected_policy_file_owner, fail_next, sync_directory_fd_strict};
