#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
compile_error!("qol-process requires a Unix or Windows target");

#[cfg(unix)]
pub(crate) use unix::{
    is_pid_alive, kill_pid, reap_children_nonblocking, signal_term_pid, spawn_detached,
    terminate_group, terminate_owned, terminate_pid, try_wait_pid, wait_pid,
};
#[cfg(windows)]
pub(crate) use windows::{
    is_pid_alive, kill_pid, reap_children_nonblocking, signal_term_pid, spawn_detached,
    terminate_group, terminate_owned, terminate_pid, try_wait_pid, wait_pid,
};
