#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, test))]
mod stop_hooks;
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
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group,
    kill_pid, on_stop_request, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_image_path, process_start_time,
    process_tree_containment_support, processes, release_host_lifetime, reload_group,
    request_stop_if_identity, run_process_tree_guardian_entry, signal_term_group, signal_term_pid,
    spawn_detached, spawn_owned, system_program_path, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
    PreparedSpawn, ProcessTreeGuard,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group,
    kill_pid, on_stop_request, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_image_path, process_start_time,
    process_tree_containment_support, processes, release_host_lifetime, reload_group,
    request_stop_if_identity, run_process_tree_guardian_entry, signal_term_group, signal_term_pid,
    spawn_detached, spawn_owned, system_program_path, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
    PreparedSpawn, ProcessTreeGuard,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group,
    kill_pid, on_stop_request, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_image_path, process_start_time,
    process_tree_containment_support, processes, release_host_lifetime, reload_group,
    request_stop_if_identity, run_process_tree_guardian_entry, signal_term_group, signal_term_pid,
    spawn_detached, spawn_owned, system_program_path, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
    PreparedSpawn, ProcessTreeGuard,
};
#[cfg(not(any(unix, windows)))]
pub(crate) use unsupported::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group,
    kill_pid, on_stop_request, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_image_path, process_start_time,
    process_tree_containment_support, processes, release_host_lifetime, reload_group,
    request_stop_if_identity, run_process_tree_guardian_entry, signal_term_group, signal_term_pid,
    spawn_detached, spawn_owned, system_program_path, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
    PreparedSpawn, ProcessTreeGuard,
};
#[cfg(windows)]
pub(crate) use windows::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, is_pid_zombie, isolate_owned_command, isolate_owned_session, kill_group,
    kill_pid, on_stop_request, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_image_path, process_start_time,
    process_tree_containment_support, processes, release_host_lifetime, reload_group,
    request_stop_if_identity, run_process_tree_guardian_entry, signal_term_group, signal_term_pid,
    spawn_detached, spawn_owned, system_program_path, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
    PreparedSpawn, ProcessTreeGuard,
};
