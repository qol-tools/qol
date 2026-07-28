mod platform;

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

pub(crate) fn own_process_tree() -> Result<qol_process::ProcessTreeGuard> {
    let executable = platform::guardian_executable()?;
    let guardian = command(&executable);
    qol_process::own_current_process_tree_with_guardian(guardian)
        .context("failed to start the process-tree guardian")
}

#[cfg(not(test))]
pub(crate) fn command(executable: &Path) -> Command {
    qol_process::process_tree_guardian_command(executable)
}

#[cfg(test)]
pub(crate) fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.args([
        "--exact",
        "process_guardian::process_tree_guardian_test_entry",
        "--nocapture",
    ]);
    command
}

pub(crate) fn run() -> Result<()> {
    qol_process::run_process_tree_guardian_entry().context("process-tree guardian failed")
}

#[cfg(test)]
#[test]
fn process_tree_guardian_test_entry() {
    if std::env::var_os("QOL_PROCESS_GUARDIAN_PROTOCOL").is_some() {
        run().unwrap();
    }
}
