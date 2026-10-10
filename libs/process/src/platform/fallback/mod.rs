use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub(crate) use super::unix::{
    bind_to_host_lifetime, cancellation_requested, cancellation_signal_count,
    guard_current_process_tree, hide_console_window, install_cancellation_handler, is_group_alive,
    is_pid_alive, isolate_owned_command, kill_group, kill_pid, reload_group, signal_term_group,
    signal_term_pid, spawn_detached, terminate_group, terminate_owned, terminate_pid, try_wait_pid,
    wait_for_stop_request, wait_pid, CurrentProcessTreeGuard,
};
pub(crate) use super::unix_containment::{
    own_current_process_tree_with_guardian, process_tree_containment_support,
    run_process_tree_guardian_entry, PreparedSpawn, ProcessTreeGuard,
};

pub(crate) fn isolate_owned_session(command: &mut Command) -> io::Result<()> {
    unsafe {
        command.pre_exec(|| loop {
            if libc::setsid() != -1 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error);
            }
        });
    }
    Ok(())
}

pub(crate) fn spawn_owned(mut command: Command) -> io::Result<(Child, Option<ProcessTreeGuard>)> {
    isolate_owned_command(&mut command)?;
    Ok((command.spawn()?, None))
}

pub(crate) fn is_pid_zombie(_pid: u32) -> bool {
    false
}

pub(crate) fn process_identity(_pid: u32) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "durable process identity is unsupported on this Unix platform",
    ))
}

pub(crate) fn process_identity_matches(actual: &str, expected: &str) -> bool {
    actual == expected
}

pub(crate) fn processes() -> io::Result<Vec<crate::ProcessEntry>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process snapshots are unsupported on this platform",
    ))
}

pub(crate) fn process_image_path(_pid: u32) -> io::Result<std::path::PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process image paths are unsupported on this platform",
    ))
}
