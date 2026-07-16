use super::super::platform;
use std::io;
use std::process::{Child, Command, Output};
use std::thread;
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;
use tokio::task::{JoinError, JoinHandle};

#[cfg(target_os = "macos")]
const POLL_INTERVAL: Duration = Duration::from_millis(20);
#[cfg(target_os = "macos")]
const STOP_GRACE: Duration = Duration::from_millis(250);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct CommandTask {
    owner: AsyncCommandTree,
    waiter: JoinHandle<io::Result<Output>>,
    timeout: u64,
}

#[derive(Debug)]
pub(super) enum TaskError {
    Command(String),
    Timeout { seconds: u64, cleanup: Vec<String> },
}

struct AsyncCommandTree {
    cleanup: Option<TreeCleanup>,
}

enum TreeCleanup {
    Verified(qol_process::ProcessTreeGuard),
    #[cfg(target_os = "macos")]
    ProcessGroup(u32),
}

impl CommandTask {
    pub(super) fn start(command: Command, timeout: u64) -> Result<Self, TaskError> {
        let (owner, child) = AsyncCommandTree::spawn(command)?;
        let waiter = tokio::task::spawn_blocking(move || child.wait_with_output());
        Ok(Self {
            owner,
            waiter,
            timeout,
        })
    }

    pub(super) async fn wait(mut self) -> Result<Output, TaskError> {
        tokio::select! {
            result = &mut self.waiter => self.finish(result).await,
            _ = tokio::time::sleep(Duration::from_secs(self.timeout)) => {
                self.finish_timeout().await
            }
        }
    }

    async fn finish(
        &mut self,
        result: Result<io::Result<Output>, JoinError>,
    ) -> Result<Output, TaskError> {
        let output = join_output(result);
        let residual = self.owner.is_alive();
        let cleanup = self.owner.cleanup().await;
        combine_completion(output, residual, cleanup)
    }

    async fn finish_timeout(&mut self) -> Result<Output, TaskError> {
        let mut cleanup = self
            .owner
            .cleanup()
            .await
            .err()
            .map(|error| vec![error.to_string()])
            .unwrap_or_default();
        let wait = tokio::time::timeout(STOP_TIMEOUT, &mut self.waiter).await;
        if let Err(error) = timeout_wait_error(wait) {
            cleanup.push(error);
        }
        Err(TaskError::Timeout {
            seconds: self.timeout,
            cleanup,
        })
    }
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Command(message) => write!(formatter, "Command failed: {message}"),
            Self::Timeout { seconds, cleanup } => {
                write!(formatter, "Timeout after {seconds}s")?;
                if cleanup.is_empty() {
                    return Ok(());
                }
                write!(formatter, "; cleanup failed: {}", cleanup.join("; "))
            }
        }
    }
}

impl AsyncCommandTree {
    fn spawn(mut command: Command) -> Result<(Self, Child), TaskError> {
        match platform::verified_process_tree() {
            Ok(tree) => {
                qol_process::isolate_owned_session(&mut command).map_err(command_error)?;
                let prepared = tree.prepare_command(command).map_err(command_error)?;
                let child = prepared.spawn().map_err(|error| {
                    let cleanup = error.cleanup();
                    TaskError::Command(format!("{error}; cleanup state: {cleanup:?}"))
                })?;
                Ok((Self::verified(tree), child))
            }
            #[cfg(target_os = "macos")]
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                qol_process::isolate_owned_command(&mut command).map_err(command_error)?;
                let child = command.spawn().map_err(command_error)?;
                Ok((Self::process_group(child.id()), child))
            }
            Err(error) => Err(command_error(error)),
        }
    }

    fn verified(tree: qol_process::ProcessTreeGuard) -> Self {
        Self {
            cleanup: Some(TreeCleanup::Verified(tree)),
        }
    }

    #[cfg(target_os = "macos")]
    fn process_group(pid: u32) -> Self {
        Self {
            cleanup: Some(TreeCleanup::ProcessGroup(pid)),
        }
    }

    fn is_alive(&self) -> io::Result<bool> {
        match self.cleanup.as_ref() {
            Some(TreeCleanup::Verified(tree)) => tree.tree_has_exited().map(|exited| !exited),
            #[cfg(target_os = "macos")]
            Some(TreeCleanup::ProcessGroup(pid)) => Ok(qol_process::is_group_alive(*pid)),
            None => Ok(false),
        }
    }

    async fn cleanup(&mut self) -> io::Result<()> {
        let Some(cleanup) = self.cleanup.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || cleanup.run())
            .await
            .map_err(join_error)?
    }
}

impl Drop for AsyncCommandTree {
    fn drop(&mut self) {
        let Some(cleanup) = self.cleanup.take() else {
            return;
        };
        #[cfg(target_os = "macos")]
        let fallback_pid = cleanup.process_group_id();
        let _started = thread::Builder::new()
            .name("qol-task-cleanup".to_string())
            .spawn(move || {
                let _ = cleanup.run();
            });
        #[cfg(target_os = "macos")]
        if _started.is_err() {
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
            #[cfg(target_os = "macos")]
            Self::ProcessGroup(pid) => stop_process_group(pid),
        }
    }

    #[cfg(target_os = "macos")]
    fn process_group_id(&self) -> Option<u32> {
        match self {
            Self::Verified(_) => None,
            Self::ProcessGroup(pid) => Some(*pid),
        }
    }
}

#[cfg(target_os = "macos")]
fn stop_process_group(pid: u32) -> io::Result<()> {
    tolerate_stopped(qol_process::signal_term_group(pid), pid)?;
    let deadline = Instant::now() + STOP_GRACE;
    while qol_process::is_group_alive(pid) && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
    }
    if !qol_process::is_group_alive(pid) {
        return Ok(());
    }
    tolerate_stopped(qol_process::kill_group(pid), pid)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while qol_process::is_group_alive(pid) && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
    }
    if qol_process::is_group_alive(pid) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("command process group {pid} did not exit"),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn tolerate_stopped(result: io::Result<()>, pid: u32) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(_) if !qol_process::is_group_alive(pid) => Ok(()),
        Err(error) => Err(error),
    }
}

fn join_output(result: Result<io::Result<Output>, JoinError>) -> io::Result<Output> {
    result.map_err(join_error)?
}

fn combine_completion(
    output: io::Result<Output>,
    residual: io::Result<bool>,
    cleanup: io::Result<()>,
) -> Result<Output, TaskError> {
    let mut failures = Vec::new();
    let output = match output {
        Ok(output) => Some(output),
        Err(error) => {
            failures.push(error.to_string());
            None
        }
    };
    match residual {
        Ok(true) => failures.push("command exited while descendants remained".to_string()),
        Ok(false) => {}
        Err(error) => failures.push(format!("failed to inspect command tree: {error}")),
    }
    if let Err(error) = cleanup {
        failures.push(format!("failed to clean command tree: {error}"));
    }
    if !failures.is_empty() {
        return Err(TaskError::Command(failures.join("; ")));
    }
    output.ok_or_else(|| TaskError::Command("command produced no output result".to_string()))
}

fn join_error(error: JoinError) -> io::Error {
    io::Error::other(format!("command waiter failed: {error}"))
}

fn timeout_wait_error(
    result: Result<Result<io::Result<Output>, JoinError>, tokio::time::error::Elapsed>,
) -> Result<(), String> {
    match result {
        Ok(Ok(Ok(_))) => Ok(()),
        Ok(Ok(Err(error))) => Err(format!("failed to reap command root: {error}")),
        Ok(Err(error)) => Err(format!("command waiter failed: {error}")),
        Err(_) => Err("command root was not reaped after tree cleanup".to_string()),
    }
}

fn command_error(error: impl std::fmt::Display) -> TaskError {
    TaskError::Command(error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_guardian_test_entry() {
        if std::env::var_os("QOL_PROCESS_GUARDIAN_PROTOCOL").is_some() {
            qol_process::run_process_tree_guardian_entry().unwrap();
        }
    }
}
