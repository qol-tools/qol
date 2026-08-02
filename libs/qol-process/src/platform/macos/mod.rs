use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

use super::unix::pid_t;
pub(crate) use super::unix::{
    cancellation_requested, cancellation_signal_count, guard_current_process_tree,
    install_cancellation_handler, is_group_alive, is_pid_alive, isolate_owned_command, kill_group,
    kill_pid, signal_term_group, signal_term_pid, spawn_detached, terminate_group, terminate_owned,
    terminate_pid, try_wait_pid, wait_pid, CurrentProcessTreeGuard,
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
            let error = raw_preexec_error();
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

fn raw_preexec_error() -> io::Error {
    let code = unsafe { *libc::__error() };
    io::Error::from_raw_os_error(code)
}

pub(crate) fn is_pid_zombie(_pid: u32) -> bool {
    false
}

pub(crate) fn process_identity(pid: u32) -> io::Result<String> {
    let pid = pid_t(pid)?;
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            i32::try_from(size).map_err(|_| io::Error::other("process info is too large"))?,
        )
    };
    if read != i32::try_from(size).unwrap_or(i32::MAX) {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(
        "macos:{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

pub(crate) fn process_identity_matches(actual: &str, expected: &str) -> bool {
    actual == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_containment_fails_closed() {
        let result = own_current_process_tree_with_guardian(Command::new("unused"));
        let error = match result {
            Ok(_) => panic!("macOS unexpectedly accepted verified process-tree containment"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn process_identity_uses_the_process_start_generation() {
        let identity = process_identity(std::process::id()).unwrap();
        let fields = identity.split(':').collect::<Vec<_>>();

        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], "macos");
        assert!(fields[1].parse::<u64>().is_ok());
        assert!(fields[2].parse::<u64>().is_ok());
    }
}
