mod containment;

use std::io;
use std::process::{Child, Command};

pub(crate) use super::unix::{
    cancellation_requested, cancellation_signal_count, guard_current_process_tree,
    install_cancellation_handler, is_group_alive, is_pid_alive, isolate_owned_command, kill_group,
    kill_pid, signal_term_group, signal_term_pid, spawn_detached, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard,
};
pub(crate) use containment::{
    is_pid_zombie, isolate_owned_session, own_current_process_tree_with_guardian, process_identity,
    process_identity_matches, process_tree_containment_support, run_process_tree_guardian_entry,
    PreparedSpawn, ProcessTreeGuard,
};

pub(crate) fn spawn_owned(mut command: Command) -> io::Result<(Child, Option<ProcessTreeGuard>)> {
    isolate_owned_command(&mut command)?;
    Ok((command.spawn()?, None))
}
