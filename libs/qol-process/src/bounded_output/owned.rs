use super::capture::{combine_errors, join_capture_thread, spawn_capture, WAIT_INTERVAL};
use super::{BoundedCommandOutput, CompletedCommandOutput};
use crate::OwnedProcessTree;
use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const OWNED_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

pub fn run_owned_with_output_timeout(
    mut command: Command,
    timeout: Duration,
    output_limit: usize,
) -> io::Result<BoundedCommandOutput> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let (mut child, tree) = crate::spawn_owned(command)?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            return Err(fail_owned_setup(
                &tree,
                &mut child,
                io::Error::other("child stdout was not piped"),
            ));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            return Err(fail_owned_setup(
                &tree,
                &mut child,
                io::Error::other("child stderr was not piped"),
            ));
        }
    };
    let stdout_reader = match spawn_capture(stdout, output_limit, "stdout") {
        Ok(reader) => reader,
        Err(error) => return Err(fail_owned_setup(&tree, &mut child, error)),
    };
    let stderr_reader = match spawn_capture(stderr, output_limit, "stderr") {
        Ok(reader) => reader,
        Err(error) => {
            let cleanup = terminate_owned_tree(&tree, &mut child);
            return Err(match cleanup {
                Ok(()) => {
                    let _ = join_capture_thread(stdout_reader, "stdout");
                    error
                }
                Err(cleanup) => {
                    drop(stdout_reader);
                    combine_errors(error, "owned probe cleanup failed", cleanup)
                }
            });
        }
    };

    match wait_for_owned_child(&mut child, &tree, deadline) {
        Ok(WaitOutcome::Completed(status)) => {
            let stdout = join_capture_thread(stdout_reader, "stdout");
            let stderr = join_capture_thread(stderr_reader, "stderr");
            Ok(BoundedCommandOutput::Completed(CompletedCommandOutput {
                status,
                stdout: stdout?,
                stderr: stderr?,
            }))
        }
        Ok(WaitOutcome::TimedOut) => match terminate_owned_tree(&tree, &mut child) {
            Ok(()) => {
                let stdout = join_capture_thread(stdout_reader, "stdout");
                let stderr = join_capture_thread(stderr_reader, "stderr");
                Ok(BoundedCommandOutput::TimedOut {
                    stdout: stdout?,
                    stderr: stderr?,
                })
            }
            Err(cleanup) => {
                drop(stdout_reader);
                drop(stderr_reader);
                Err(io::Error::new(
                    cleanup.kind(),
                    format!("timed-out owned probe cleanup failed: {cleanup}"),
                ))
            }
        },
        Err(error) => match terminate_owned_tree(&tree, &mut child) {
            Ok(()) => {
                let stdout = join_capture_thread(stdout_reader, "stdout");
                let stderr = join_capture_thread(stderr_reader, "stderr");
                match (stdout, stderr) {
                    (Ok(_), Ok(_)) => Err(error),
                    (Err(stdout_error), _) => {
                        Err(combine_errors(error, "stdout capture failed", stdout_error))
                    }
                    (_, Err(stderr_error)) => {
                        Err(combine_errors(error, "stderr capture failed", stderr_error))
                    }
                }
            }
            Err(cleanup) => {
                drop(stdout_reader);
                drop(stderr_reader);
                Err(combine_errors(error, "owned probe cleanup failed", cleanup))
            }
        },
    }
}

enum WaitOutcome {
    Completed(ExitStatus),
    TimedOut,
}

fn wait_for_owned_child(
    child: &mut Child,
    tree: &OwnedProcessTree,
    deadline: Instant,
) -> io::Result<WaitOutcome> {
    let mut status = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(completed)) => status = Some(completed),
                Ok(None) => {}
                Err(error) => return Err(error),
            }
        }
        if let (Some(status), true) = (status, tree.tree_has_exited()?) {
            return Ok(WaitOutcome::Completed(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(WaitOutcome::TimedOut);
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn fail_owned_setup(tree: &OwnedProcessTree, child: &mut Child, error: io::Error) -> io::Error {
    match terminate_owned_tree(tree, child) {
        Ok(()) => error,
        Err(cleanup) => combine_errors(error, "owned probe cleanup failed", cleanup),
    }
}

fn terminate_owned_tree(tree: &OwnedProcessTree, child: &mut Child) -> io::Result<()> {
    tree.terminate_and_wait(child, OWNED_CLEANUP_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const HELPER_ENV: &str = "QOL_PROCESS_OWNED_OUTPUT_HELPER";
    #[cfg(unix)]
    const TREE_ROOT_ENV: &str = "QOL_PROCESS_OWNED_OUTPUT_TREE_ROOT";

    #[test]
    fn owned_output_child_helper() {
        let Some(mode) = std::env::var_os(HELPER_ENV) else {
            return;
        };
        match mode.to_string_lossy().as_ref() {
            "capture" => {
                println!("captured stdout");
                eprintln!("captured stderr");
            }
            "large" => {
                let bytes = vec![b'x'; 128 * 1024];
                std::io::stdout().write_all(&bytes).unwrap();
                std::io::stderr().write_all(&bytes).unwrap();
            }
            "sleep" => std::thread::sleep(Duration::from_secs(10)),
            other => run_unix_helper_mode(other),
        }
    }

    #[cfg(unix)]
    fn run_unix_helper_mode(mode: &str) {
        match mode {
            "descendant" => {
                let root = std::path::PathBuf::from(std::env::var_os(TREE_ROOT_ENV).unwrap());
                std::fs::write(root.join("root"), std::process::id().to_string()).unwrap();
                let mut descendant = Command::new("sh").args(["-c", "sleep 30"]).spawn().unwrap();
                std::fs::write(root.join("descendant"), descendant.id().to_string()).unwrap();
                let _ = descendant.wait();
            }
            "linger" => {
                let root = std::path::PathBuf::from(std::env::var_os(TREE_ROOT_ENV).unwrap());
                std::fs::write(root.join("root"), std::process::id().to_string()).unwrap();
                let descendant = Command::new("sh").args(["-c", "sleep 30"]).spawn().unwrap();
                std::fs::write(root.join("descendant"), descendant.id().to_string()).unwrap();
                std::mem::forget(descendant);
            }
            other => panic!("unknown helper mode: {other}"),
        }
    }

    #[cfg(not(unix))]
    fn run_unix_helper_mode(mode: &str) {
        panic!("unknown helper mode: {mode}");
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn owned_runner_preserves_successful_output() {
        let output = run_owned_with_output_timeout(
            helper_command("capture"),
            Duration::from_secs(5),
            16 * 1024,
        )
        .unwrap();
        let BoundedCommandOutput::Completed(output) = output else {
            panic!("helper unexpectedly timed out");
        };

        assert!(output.status.success());
        assert!(text(&output.stdout).contains("captured stdout"));
        assert!(text(&output.stderr).contains("captured stderr"));
        assert!(!output.stdout.is_truncated());
        assert!(!output.stderr.is_truncated());
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn owned_runner_drains_streams_while_bounding_retained_bytes() {
        let output =
            run_owned_with_output_timeout(helper_command("large"), Duration::from_secs(5), 256)
                .unwrap();
        let BoundedCommandOutput::Completed(output) = output else {
            panic!("helper unexpectedly timed out");
        };

        assert!(output.status.success());
        assert_eq!(output.stdout.as_bytes().len(), 256);
        assert_eq!(output.stderr.as_bytes().len(), 256);
        assert!(output.stdout.is_truncated());
        assert!(output.stderr.is_truncated());
    }

    #[test]
    #[cfg(unix)]
    fn owned_runner_terminates_and_reaps_a_timed_out_tree_with_a_same_group_descendant() {
        let root = tempfile::tempdir().unwrap();
        let mut command = helper_command("descendant");
        command.env(TREE_ROOT_ENV, root.path());
        let started = Instant::now();
        let output = run_owned_with_output_timeout(command, Duration::from_secs(1), 1024).unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(wait_for_exit(read_pid(&root.path().join("root"))));
        assert!(wait_for_exit(read_pid(&root.path().join("descendant"))));
    }

    #[test]
    #[cfg(unix)]
    fn owned_runner_terminates_a_pipe_holding_descendant_after_root_exit() {
        let root = tempfile::tempdir().unwrap();
        let mut command = helper_command("linger");
        command.env(TREE_ROOT_ENV, root.path());
        let started = Instant::now();
        let output = run_owned_with_output_timeout(command, Duration::from_secs(1), 1024).unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(wait_for_exit(read_pid(&root.path().join("root"))));
        assert!(wait_for_exit(read_pid(&root.path().join("descendant"))));
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn owned_runner_reports_a_missing_program() {
        let command = Command::new("/qol-process/nonexistent-command");
        let error =
            run_owned_with_output_timeout(command, Duration::from_secs(1), 1024).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    fn helper_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "bounded_output::owned::tests::owned_output_child_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, mode);
        command
    }

    #[cfg(any(unix, windows))]
    fn text(output: &super::super::CapturedOutput) -> String {
        String::from_utf8_lossy(output.as_bytes()).into_owned()
    }

    #[cfg(unix)]
    fn read_pid(path: &std::path::Path) -> u32 {
        std::fs::read_to_string(path).unwrap().parse().unwrap()
    }

    #[cfg(unix)]
    fn wait_for_exit(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while crate::is_pid_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        !crate::is_pid_alive(pid)
    }
}
