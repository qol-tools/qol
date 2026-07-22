use super::super::platform;
use std::io;
use std::process::{Command, Output};
use std::time::Duration;
use tokio::task::{JoinError, JoinHandle};

const STOP_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct CommandTask {
    owner: platform::CommandTree,
    waiter: JoinHandle<io::Result<Output>>,
    timeout: u64,
}

#[derive(Debug)]
pub(super) enum TaskError {
    Command(String),
    Timeout { seconds: u64, cleanup: Vec<String> },
}

impl CommandTask {
    pub(super) fn start(command: Command, timeout: u64) -> Result<Self, TaskError> {
        let (owner, child) = platform::CommandTree::spawn(command).map_err(command_error)?;
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
