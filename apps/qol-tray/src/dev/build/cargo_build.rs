mod codesign;
mod plugin_build;
mod self_build;

use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use super::types::BuildResult;
use crate::dev::adapters::CargoPluginBuilder;

pub(super) struct CargoChild {
    pub(super) child: Child,
    pub(super) stdout: ChildStdout,
    pub(super) stderr: ChildStderr,
}

pub(super) fn spawn_piped(command: &mut Command) -> Result<CargoChild, std::io::Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    Ok(CargoChild {
        child,
        stdout,
        stderr,
    })
}

pub(super) const BUILD_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Build timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("Failed waiting for build: {}", e)),
        }
    }
}

pub(crate) struct CargoCommandPluginBuilder;

impl CargoPluginBuilder for CargoCommandPluginBuilder {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult {
        plugin_build::build_cargo_plugin_with_progress(plugin_id, path, on_progress)
    }
}

pub fn build_qol_tray_self_with_progress<F>(repo_root: Option<&Path>, on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    self_build::build_qol_tray_self_with_progress(repo_root, on_progress)
}
