#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
compile_error!("qol-process requires a Unix or Windows target");

#[cfg(unix)]
pub(crate) use unix::{
    cancellation_requested, cancellation_signal_count, guard_current_process_tree,
    install_cancellation_handler, is_group_alive, is_pid_alive, is_pid_zombie,
    isolate_owned_command, isolate_owned_session, kill_pid, own_current_process_tree_with_guardian,
    process_identity, process_tree_containment_support, reap_children_nonblocking,
    run_process_tree_guardian_entry, signal_term_pid, spawn_detached, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
#[cfg(windows)]
pub(crate) use windows::{
    cancellation_requested, cancellation_signal_count, guard_current_process_tree,
    install_cancellation_handler, is_group_alive, is_pid_alive, is_pid_zombie,
    isolate_owned_command, isolate_owned_session, kill_pid, own_current_process_tree_with_guardian,
    process_identity, process_tree_containment_support, reap_children_nonblocking,
    run_process_tree_guardian_entry, signal_term_pid, spawn_detached, terminate_group,
    terminate_owned, terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard, PreparedSpawn,
    ProcessTreeGuard,
};
