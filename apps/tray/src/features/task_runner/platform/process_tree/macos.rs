use std::io;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const STOP_GRACE: Duration = Duration::from_millis(250);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

pub(in crate::features::task_runner) struct CommandTree {
    cleanup: Option<TreeCleanup>,
}

enum TreeCleanup {
    Verified(qol_process::ProcessTreeGuard),
    ProcessGroup(u32),
}

impl CommandTree {
    pub(in crate::features::task_runner) fn spawn(
        mut command: Command,
    ) -> io::Result<(Self, Child)> {
        match super::super::verified_process_tree() {
            Ok(tree) => {
                qol_process::isolate_owned_session(&mut command)?;
                let prepared = tree.prepare_command(command)?;
                let child = prepared.spawn().map_err(|error| {
                    let cleanup = error.cleanup();
                    io::Error::other(format!("{error}; cleanup state: {cleanup:?}"))
                })?;
                Ok((
                    Self {
                        cleanup: Some(TreeCleanup::Verified(tree)),
                    },
                    child,
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                qol_process::isolate_owned_command(&mut command)?;
                let child = command.spawn()?;
                Ok((
                    Self {
                        cleanup: Some(TreeCleanup::ProcessGroup(child.id())),
                    },
                    child,
                ))
            }
            Err(error) => Err(error),
        }
    }

    pub(in crate::features::task_runner) fn is_alive(&self) -> io::Result<bool> {
        match self.cleanup.as_ref() {
            Some(TreeCleanup::Verified(tree)) => tree.tree_has_exited().map(|exited| !exited),
            Some(TreeCleanup::ProcessGroup(pid)) => Ok(qol_process::is_group_alive(*pid)),
            None => Ok(false),
        }
    }

    pub(in crate::features::task_runner) async fn cleanup(&mut self) -> io::Result<()> {
        let Some(cleanup) = self.cleanup.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || cleanup.run())
            .await
            .map_err(|error| io::Error::other(format!("command waiter failed: {error}")))?
    }
}

impl Drop for CommandTree {
    fn drop(&mut self) {
        let Some(cleanup) = self.cleanup.take() else {
            return;
        };
        let fallback_pid = cleanup.process_group_id();
        let started = std::thread::Builder::new()
            .name("qol-task-cleanup".to_string())
            .spawn(move || {
                let _ = cleanup.run();
            });
        if started.is_err() {
            if let Some(pid) = fallback_pid {
                let _ = stop_process_group(pid);
            }
        }
    }
}

impl TreeCleanup {
    fn run(self) -> io::Result<()> {
        match self {
            Self::Verified(tree) => tree.terminate_and_wait(STOP_TIMEOUT).map(drop),
            Self::ProcessGroup(pid) => stop_process_group(pid),
        }
    }

    fn process_group_id(&self) -> Option<u32> {
        match self {
            Self::Verified(_) => None,
            Self::ProcessGroup(pid) => Some(*pid),
        }
    }
}

fn stop_process_group(pid: u32) -> io::Result<()> {
    tolerate_stopped(qol_process::signal_term_group(pid), pid)?;
    let deadline = Instant::now() + STOP_GRACE;
    while qol_process::is_group_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);
    }
    if !qol_process::is_group_alive(pid) {
        return Ok(());
    }
    tolerate_stopped(qol_process::kill_group(pid), pid)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while qol_process::is_group_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);
    }
    if qol_process::is_group_alive(pid) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("command process group {pid} did not exit"),
        ));
    }
    Ok(())
}

fn tolerate_stopped(result: io::Result<()>, pid: u32) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(_) if !qol_process::is_group_alive(pid) => Ok(()),
        Err(error) => Err(error),
    }
}
