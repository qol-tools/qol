use std::io;
use std::process::{Child, Command};
use std::time::Duration;

const STOP_TIMEOUT: Duration = Duration::from_secs(2);

pub(in crate::features::task_runner) struct CommandTree {
    cleanup: Option<qol_process::ProcessTreeGuard>,
}

impl CommandTree {
    pub(in crate::features::task_runner) fn spawn(
        mut command: Command,
    ) -> io::Result<(Self, Child)> {
        let tree = super::super::verified_process_tree()?;
        qol_process::isolate_owned_session(&mut command)?;
        let prepared = tree.prepare_command(command)?;
        let child = prepared.spawn().map_err(|error| {
            let cleanup = error.cleanup();
            io::Error::other(format!("{error}; cleanup state: {cleanup:?}"))
        })?;
        Ok((
            Self {
                cleanup: Some(tree),
            },
            child,
        ))
    }

    pub(in crate::features::task_runner) fn is_alive(&self) -> io::Result<bool> {
        self.cleanup
            .as_ref()
            .map(|tree| tree.tree_has_exited().map(|exited| !exited))
            .unwrap_or(Ok(false))
    }

    pub(in crate::features::task_runner) async fn cleanup(&mut self) -> io::Result<()> {
        let Some(tree) = self.cleanup.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || tree.terminate_and_wait(STOP_TIMEOUT).map(drop))
            .await
            .map_err(|error| io::Error::other(format!("command waiter failed: {error}")))?
    }
}

impl Drop for CommandTree {
    fn drop(&mut self) {
        let Some(tree) = self.cleanup.take() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("qol-task-cleanup".to_string())
            .spawn(move || {
                let _ = tree.terminate_and_wait(STOP_TIMEOUT);
            });
    }
}
