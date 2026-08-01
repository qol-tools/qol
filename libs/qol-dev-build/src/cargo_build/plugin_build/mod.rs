mod progress;
mod streams;

use std::path::Path;
use std::process::Command;

use super::super::types::BuildResult;
use super::codesign::codesign_debug_binaries;
use super::{spawn_piped, CargoChild};

pub(super) fn build_cargo_plugin_with_progress<F>(
    plugin_id: &str,
    path: &Path,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let CargoChild {
        mut child,
        stdout,
        stderr,
        process_tree,
    } = match start_build(plugin_id, path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = streams::spawn_output_readers(stdout, stderr);
    let mut progress_driver = progress::ProgressDriver::default();
    let wait_result = super::wait_with_timeout_and_poll_owned(
        plugin_id,
        &mut child,
        &process_tree,
        super::BUILD_TIMEOUT,
        || progress_driver.poll(readers.progress_rx(), &mut on_progress),
    );
    progress_driver.poll(readers.progress_rx(), &mut on_progress);
    let tree_closed = process_tree.tree_has_exited().unwrap_or(false);
    if let Err(message) = &wait_result {
        if !tree_closed {
            log::error!(
                "[dev-build] event=reader_join plugin_id={} joined=false process_tree_closed=false reason=cleanup_failed",
                plugin_id
            );
            drop(readers);
            return super::failed_build(plugin_id, message.clone());
        }
    }
    let join_started = std::time::Instant::now();
    let combined = readers.join_output();
    log::debug!(
        "[dev-build] event=reader_join plugin_id={} joined=true process_tree_closed={} elapsed_ms={}",
        plugin_id,
        tree_closed,
        join_started.elapsed().as_millis()
    );
    super::finish_build_after_wait(
        plugin_id,
        wait_result,
        combined,
        &mut on_progress,
        |output, progress| success_build(plugin_id, path, output, progress),
    )
}

fn start_build<F>(
    plugin_id: &str,
    path: &Path,
    on_progress: &mut F,
) -> Result<CargoChild, BuildResult>
where
    F: FnMut(u8, String),
{
    log::info!("Building linked plugin via cargo: {}", plugin_id);
    on_progress(0, "Preparing build".to_string());
    spawn_build(path).map_err(|error| failed_spawn(plugin_id, error))
}

fn spawn_build(path: &Path) -> Result<CargoChild, std::io::Error> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(path);
    spawn_piped(command)
}

fn success_build<F>(
    plugin_id: &str,
    path: &Path,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    codesign_debug_binaries(plugin_id, path);
    on_progress(100, "Build complete".to_string());
    log::info!("Cargo build succeeded for {}", plugin_id);
    super::finished_build(plugin_id, combined)
}

fn failed_spawn(plugin_id: &str, error: std::io::Error) -> BuildResult {
    let message = format!("Failed to run cargo build: {}", error);
    log::error!("Build error for {}: {}", plugin_id, message);
    super::failed_build(plugin_id, message)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    use std::time::{Duration, Instant};

    #[cfg(windows)]
    #[test]
    #[allow(clippy::zombie_processes)]
    fn windows_output_descendant_helper() {
        let Some(marker) = std::env::var_os("QOL_DEV_BUILD_WINDOWS_DESCENDANT") else {
            return;
        };
        let mut descendant = Command::new("cmd");
        descendant.args(["/C", "ping -n 31 127.0.0.1 >NUL"]);
        let mut descendant = descendant.spawn().unwrap();
        std::fs::write(marker, descendant.id().to_string()).unwrap();
        let _ = descendant.wait();
    }

    #[cfg(windows)]
    #[test]
    fn windows_timeout_closes_output_readers_after_tree_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("descendant");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "cargo_build::plugin_build::tests::windows_output_descendant_helper",
                "--nocapture",
            ])
            .env("QOL_DEV_BUILD_WINDOWS_DESCENDANT", &marker);
        let CargoChild {
            mut child,
            stdout,
            stderr,
            process_tree,
        } = super::super::spawn_piped(command).unwrap();
        let readers = streams::spawn_output_readers(stdout, stderr);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.is_file() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.is_file(), "descendant pid was not recorded");
        let descendant_pid = std::fs::read_to_string(&marker)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();

        let started = Instant::now();
        let wait_result = super::super::wait_with_timeout_and_poll_owned(
            "windows-reader-timeout-test",
            &mut child,
            &process_tree,
            Duration::from_millis(100),
            || {},
        );
        let cleanup_elapsed = started.elapsed();
        let join_started = Instant::now();
        let _output = readers.join_output();
        let join_elapsed = join_started.elapsed();

        assert!(wait_result.is_err(), "the hanging process must time out");
        assert!(cleanup_elapsed < Duration::from_secs(3));
        assert!(join_elapsed < Duration::from_secs(1));
        assert!(process_tree.tree_has_exited().unwrap());
        assert!(!qol_process::is_pid_alive(descendant_pid));
    }
}
