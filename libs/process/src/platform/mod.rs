#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(all(unix, not(target_os = "linux")))]
mod unix_containment;
#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(crate) use fallback::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, install_cancellation_handler, is_group_alive, is_pid_alive,
    is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group, kill_pid,
    own_current_process_tree_with_guardian, process_identity, process_identity_matches,
    process_tree_containment_support, reload_group, run_process_tree_guardian_entry,
    signal_term_group, signal_term_pid, spawn_detached, spawn_owned, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, install_cancellation_handler, is_group_alive, is_pid_alive,
    is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group, kill_pid,
    own_current_process_tree_with_guardian, process_identity, process_identity_matches,
    process_tree_containment_support, reload_group, run_process_tree_guardian_entry,
    signal_term_group, signal_term_pid, spawn_detached, spawn_owned, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, install_cancellation_handler, is_group_alive, is_pid_alive,
    is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group, kill_pid,
    own_current_process_tree_with_guardian, process_identity, process_identity_matches,
    process_tree_containment_support, reload_group, run_process_tree_guardian_entry,
    signal_term_group, signal_term_pid, spawn_detached, spawn_owned, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
#[cfg(not(any(unix, windows)))]
pub(crate) use unsupported::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, install_cancellation_handler, is_group_alive, is_pid_alive,
    is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group, kill_pid,
    own_current_process_tree_with_guardian, process_identity, process_identity_matches,
    process_tree_containment_support, reload_group, run_process_tree_guardian_entry,
    signal_term_group, signal_term_pid, spawn_detached, spawn_owned, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
#[cfg(windows)]
pub(crate) use windows::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, install_cancellation_handler, is_group_alive, is_pid_alive,
    is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group, kill_pid,
    own_current_process_tree_with_guardian, process_identity, process_identity_matches,
    process_tree_containment_support, reload_group, run_process_tree_guardian_entry,
    signal_term_group, signal_term_pid, spawn_detached, spawn_owned, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
