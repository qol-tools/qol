use std::io;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;

use crate::{PlatformSpawnFailure, PreparedSpawnCleanup};

pub(crate) struct ProcessTreeGuard {
    _state: Mutex<()>,
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {}
}

pub(crate) struct PreparedSpawn;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "verified process-tree containment is unavailable on this Unix platform",
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
