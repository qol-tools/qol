use std::io;
use std::process::{Child, Command, ExitStatus};
use std::time::Duration;

use crate::{PlatformSpawnFailure, PreparedSpawnCleanup};

pub(crate) struct ProcessTreeGuard;

pub(crate) struct PreparedSpawn;

pub(crate) struct CurrentProcessTreeGuard;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "process lifecycle control is unavailable on this platform",
    )
}

impl ProcessTreeGuard {
    pub(crate) fn prepare_command(&self, _command: &mut Command) -> io::Result<PreparedSpawn> {
        Err(unsupported())
    }

    pub(crate) fn spawn_prepared(
        &self,
        _command: &mut Command,
        _prepared: PreparedSpawn,
    ) -> Result<Child, PlatformSpawnFailure> {
        Err(PlatformSpawnFailure {
            source: unsupported(),
            cleanup: PreparedSpawnCleanup::NotStarted,
        })
    }

    pub(crate) fn abort_prepared(&self) {}

    pub(crate) fn terminate_and_wait(&self, _timeout: Duration) -> io::Result<()> {
        Err(unsupported())
    }

    pub(crate) fn request_stop(&self) -> io::Result<()> {
        Err(unsupported())
    }

    pub(crate) fn force_stop_and_wait(&self, _timeout: Duration) -> io::Result<()> {
        Err(unsupported())
    }

    pub(crate) fn recover_pending_spawn(&self, _timeout: Duration) -> io::Result<()> {
        Err(unsupported())
    }

    pub(crate) fn terminate_root_and_wait(&self, _timeout: Duration) -> io::Result<()> {
        Err(unsupported())
    }

    pub(crate) fn root_has_exited(&self) -> io::Result<bool> {
        Err(unsupported())
    }

    pub(crate) fn tree_has_exited(&self) -> io::Result<bool> {
        Err(unsupported())
    }
}

impl CurrentProcessTreeGuard {
    pub(crate) fn disarm(&mut self) -> io::Result<()> {
        Err(unsupported())
    }
}

pub(crate) fn own_current_process_tree_with_guardian(
    _guardian_command: Command,
) -> io::Result<ProcessTreeGuard> {
    Err(unsupported())
}

pub(crate) fn run_process_tree_guardian_entry() -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn process_tree_containment_support() -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn isolate_owned_command(_command: &mut Command) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn isolate_owned_session(_command: &mut Command) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn guard_current_process_tree() -> io::Result<CurrentProcessTreeGuard> {
    Err(unsupported())
}

pub(crate) fn install_cancellation_handler() -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn cancellation_requested() -> bool {
    false
}

pub(crate) fn cancellation_signal_count() -> usize {
    0
}

pub(crate) fn is_pid_alive(_pid: u32) -> bool {
    false
}

pub(crate) fn is_group_alive(_pid: u32) -> bool {
    false
}

pub(crate) fn is_pid_zombie(_pid: u32) -> bool {
    false
}

pub(crate) fn process_identity(_pid: u32) -> io::Result<String> {
    Err(unsupported())
}

pub(crate) fn process_identity_matches(_actual: &str, _expected: &str) -> bool {
    false
}

pub(crate) fn signal_term_pid(_pid: u32) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn signal_term_group(_pid: u32) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn kill_pid(_pid: u32) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn kill_group(_pid: u32) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn try_wait_pid(_pid: u32) -> io::Result<Option<ExitStatus>> {
    Err(unsupported())
}

pub(crate) fn wait_pid(_pid: u32) -> io::Result<ExitStatus> {
    Err(unsupported())
}

pub(crate) fn terminate_pid(_pid: u32, _grace: Duration) {}

pub(crate) fn terminate_group(_pid: u32, _grace: Duration) {}

pub(crate) fn terminate_owned(_child: &mut Child, _grace: Duration) -> io::Result<()> {
    Err(unsupported())
}

pub(crate) fn spawn_detached(_command: &mut Command) -> io::Result<()> {
    Err(unsupported())
}
