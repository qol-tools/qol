#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
compile_error!("qol-process requires a Unix or Windows target");

#[cfg(unix)]
pub(crate) use unix::{
    cancellation_requested, guard_current_process_tree, install_cancellation_handler,
    is_group_alive, is_pid_alive, is_pid_zombie, isolate_owned_command, kill_pid,
    own_current_process_tree, process_identity, reap_children_nonblocking, signal_term_pid,
    spawn_detached, terminate_group, terminate_owned, terminate_pid, try_wait_pid, wait_pid,
    CurrentProcessTreeGuard, ProcessTreeGuard,
};
#[cfg(windows)]
pub(crate) use windows::{
    cancellation_requested, guard_current_process_tree, install_cancellation_handler,
    is_group_alive, is_pid_alive, is_pid_zombie, isolate_owned_command, kill_pid,
    own_current_process_tree, process_identity, reap_children_nonblocking, signal_term_pid,
    spawn_detached, terminate_group, terminate_owned, terminate_pid, try_wait_pid, wait_pid,
    CurrentProcessTreeGuard, ProcessTreeGuard,
};
