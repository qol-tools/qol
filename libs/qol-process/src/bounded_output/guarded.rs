use super::capture::{combine_errors, join_capture_thread, spawn_capture, WAIT_INTERVAL};
use super::{BoundedCommandOutput, CompletedCommandOutput};
use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CONTAINMENT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Runs a child inside verified process-tree containment with bounded output.
///
/// The target is spawned only after the platform establishes a containment
/// boundary. Timeout and error paths terminate and verify the entire contained
/// tree before reader threads are joined. Platforms without that guarantee
/// return [`io::ErrorKind::Unsupported`] before either command is spawned.
pub fn run_guarded_with_output_timeout(
    mut command: Command,
    guardian_command: Command,
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

    let guard = crate::own_current_process_tree_with_guardian(guardian_command)?;
    let prepared = guard.prepare_command(command)?;
    let mut child = match prepared.spawn() {
        Ok(child) => child,
        Err(error) => return Err(recover_prepared_spawn(&guard, error)),
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            return Err(cleanup_after_failure(
                &guard,
                &mut child,
                io::Error::other("child stdout was not piped"),
            )
            .error);
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            return Err(cleanup_after_failure(
                &guard,
                &mut child,
                io::Error::other("child stderr was not piped"),
            )
            .error);
        }
    };
    let stdout_reader = match spawn_capture(stdout, output_limit, "stdout") {
        Ok(reader) => reader,
        Err(error) => return Err(cleanup_after_failure(&guard, &mut child, error).error),
    };
    let stderr_reader = match spawn_capture(stderr, output_limit, "stderr") {
        Ok(reader) => reader,
        Err(error) => {
            let failure = cleanup_after_failure(&guard, &mut child, error);
            drop(guard);
            if failure.containment_closed {
                let _ = stdout_reader.join();
            } else {
                drop(stdout_reader);
            }
            return Err(failure.error);
        }
    };

    let completion = wait_for_guarded_child(&mut child, &guard, deadline);
    drop(guard);
    let completion = match completion {
        Ok(completion) => completion,
        Err(failure) if failure.containment_closed => {
            let _ = join_capture_thread(stdout_reader, "stdout");
            let _ = join_capture_thread(stderr_reader, "stderr");
            return Err(failure.error);
        }
        Err(failure) => {
            drop(stdout_reader);
            drop(stderr_reader);
            return Err(failure.error);
        }
    };
    let stdout = join_capture_thread(stdout_reader, "stdout");
    let stderr = join_capture_thread(stderr_reader, "stderr");
    let stdout = stdout?;
    let stderr = stderr?;

    match completion {
        WaitOutcome::Completed(status) => {
            Ok(BoundedCommandOutput::Completed(CompletedCommandOutput {
                status,
                stdout,
                stderr,
            }))
        }
        WaitOutcome::TimedOut => Ok(BoundedCommandOutput::TimedOut { stdout, stderr }),
    }
}

enum WaitOutcome {
    Completed(std::process::ExitStatus),
    TimedOut,
}

struct GuardedWaitFailure {
    error: io::Error,
    containment_closed: bool,
}

fn wait_for_guarded_child(
    child: &mut std::process::Child,
    guard: &crate::ProcessTreeGuard,
    deadline: Instant,
) -> Result<WaitOutcome, GuardedWaitFailure> {
    let mut status = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(completed)) => status = Some(completed),
                Ok(None) => {}
                Err(error) => return Err(cleanup_after_failure(guard, child, error)),
            }
        }
        let tree_has_exited = match guard.tree_has_exited() {
            Ok(exited) => exited,
            Err(error) => return Err(cleanup_after_failure(guard, child, error)),
        };
        if tree_has_exited {
            if let Some(completed) = status {
                return seal_completed_tree(guard).map(|()| WaitOutcome::Completed(completed));
            }
        }

        let now = Instant::now();
        if now >= deadline {
            return terminate_timed_out_tree(guard, child);
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn recover_prepared_spawn(
    guard: &crate::ProcessTreeGuard,
    error: crate::PreparedSpawnError,
) -> io::Error {
    let cleanup = error.cleanup;
    let source = error.source;
    if cleanup != crate::PreparedSpawnCleanup::RecoveryPending {
        return source;
    }
    match guard.recover_pending_spawn(CONTAINMENT_CLEANUP_TIMEOUT) {
        Ok(_) => source,
        Err(recovery) => combine_errors(source, "prepared process-tree recovery failed", recovery),
    }
}

fn cleanup_after_failure(
    guard: &crate::ProcessTreeGuard,
    child: &mut std::process::Child,
    error: io::Error,
) -> GuardedWaitFailure {
    match terminate_tree_and_reap(guard, child) {
        Ok(()) => GuardedWaitFailure {
            error,
            containment_closed: true,
        },
        Err(cleanup) => GuardedWaitFailure {
            error: combine_errors(error, "process-tree cleanup failed", cleanup.error),
            containment_closed: cleanup.containment_closed,
        },
    }
}

fn terminate_timed_out_tree(
    guard: &crate::ProcessTreeGuard,
    child: &mut std::process::Child,
) -> Result<WaitOutcome, GuardedWaitFailure> {
    terminate_tree_and_reap(guard, child).map(|()| WaitOutcome::TimedOut)
}

fn terminate_tree_and_reap(
    guard: &crate::ProcessTreeGuard,
    child: &mut std::process::Child,
) -> Result<(), GuardedWaitFailure> {
    match guard.terminate_and_wait(CONTAINMENT_CLEANUP_TIMEOUT) {
        Ok(_) => child
            .wait()
            .map(|_| ())
            .map_err(|error| GuardedWaitFailure {
                error,
                containment_closed: true,
            }),
        Err(error) => {
            let containment_closed = guard.tree_has_exited().unwrap_or(false);
            if !containment_closed {
                return Err(GuardedWaitFailure {
                    error,
                    containment_closed,
                });
            }
            match child.wait() {
                Ok(_) => Err(GuardedWaitFailure {
                    error,
                    containment_closed,
                }),
                Err(reap) => Err(GuardedWaitFailure {
                    error: combine_errors(error, "root-process reap failed", reap),
                    containment_closed,
                }),
            }
        }
    }
}

fn seal_completed_tree(guard: &crate::ProcessTreeGuard) -> Result<(), GuardedWaitFailure> {
    guard
        .terminate_and_wait(CONTAINMENT_CLEANUP_TIMEOUT)
        .map(|_| ())
        .map_err(|error| GuardedWaitFailure {
            containment_closed: guard.tree_has_exited().unwrap_or(false),
            error,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "linux", windows))]
    use crate::CapturedOutput;
    use std::io::Write;

    const HELPER_ENV: &str = "QOL_PROCESS_BOUNDED_OUTPUT_HELPER";
    const MARKER_ENV: &str = "QOL_PROCESS_BOUNDED_OUTPUT_MARKER";
    const TREE_ROOT_ENV: &str = "QOL_PROCESS_BOUNDED_OUTPUT_TREE_ROOT";

    #[test]
    fn bounded_output_child_helper() {
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
            "descendant" => {
                let root = std::path::PathBuf::from(std::env::var_os(TREE_ROOT_ENV).unwrap());
                std::fs::write(root.join("root"), std::process::id().to_string()).unwrap();
                let mut descendant = helper_command("sleep").spawn().unwrap();
                std::fs::write(root.join("descendant"), descendant.id().to_string()).unwrap();
                let _ = descendant.wait();
            }
            "mark" => {
                std::fs::write(std::env::var_os(MARKER_ENV).unwrap(), "spawned").unwrap();
            }
            #[cfg(target_os = "linux")]
            "observe_root_exit" => {
                let root = std::path::PathBuf::from(std::env::var_os(TREE_ROOT_ENV).unwrap());
                let root_pid = read_pid(&root.join("root"));
                while crate::is_pid_alive(root_pid) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                std::fs::write(root.join("root-exited"), "exited").unwrap();
                std::thread::sleep(Duration::from_secs(10));
            }
            #[cfg(target_os = "linux")]
            "setsid_root_exit" => spawn_setsid_descendant(false),
            #[cfg(target_os = "linux")]
            "setsid_root_wait" => spawn_setsid_descendant(true),
            "sleep" => std::thread::sleep(Duration::from_secs(10)),
            other => panic!("unknown helper mode: {other}"),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_output_guardian_helper() {
        if std::env::var_os("QOL_PROCESS_GUARDIAN_PROTOCOL").is_none() {
            return;
        }
        crate::run_process_tree_guardian_entry().unwrap();
    }

    #[test]
    #[cfg(any(target_os = "linux", windows))]
    fn guarded_runner_preserves_successful_output() {
        let output = run_guarded_with_output_timeout(
            helper_command("capture"),
            guardian_command(),
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
    #[cfg(any(target_os = "linux", windows))]
    fn guarded_runner_drains_streams_while_bounding_retained_bytes() {
        let output = run_guarded_with_output_timeout(
            helper_command("large"),
            guardian_command(),
            Duration::from_secs(5),
            256,
        )
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
    #[cfg(any(target_os = "linux", windows))]
    fn guarded_runner_terminates_and_reaps_a_timed_out_child() {
        let started = Instant::now();
        let output = run_guarded_with_output_timeout(
            helper_command("sleep"),
            guardian_command(),
            Duration::from_millis(25),
            1024,
        )
        .unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn guarded_runner_contains_a_setsid_descendant_after_root_exit() {
        let root = tempfile::tempdir().unwrap();
        let mut command = helper_command("setsid_root_exit");
        command.env(TREE_ROOT_ENV, root.path());
        let started = Instant::now();
        let output = run_guarded_with_output_timeout(
            command,
            guardian_command(),
            Duration::from_secs(1),
            1024,
        )
        .unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(5));
        let root_pid = read_pid(&root.path().join("root"));
        let descendant_pid = read_pid(&root.path().join("descendant"));
        assert!(root.path().join("root-exited").exists());
        assert!(wait_for_exit(root_pid));
        assert!(wait_for_exit(descendant_pid));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn guarded_runner_contains_a_setsid_descendant_when_the_root_times_out() {
        let root = tempfile::tempdir().unwrap();
        let mut command = helper_command("setsid_root_wait");
        command.env(TREE_ROOT_ENV, root.path());
        let output = run_guarded_with_output_timeout(
            command,
            guardian_command(),
            Duration::from_secs(1),
            1024,
        )
        .unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(wait_for_exit(read_pid(&root.path().join("root"))));
        assert!(wait_for_exit(read_pid(&root.path().join("descendant"))));
    }

    #[test]
    #[cfg(any(target_os = "linux", windows))]
    fn guarded_runner_cleans_up_a_failed_prepared_spawn() {
        let command = Command::new("/qol-process/nonexistent-command");
        let started = Instant::now();
        let error = run_guarded_with_output_timeout(
            command,
            guardian_command(),
            Duration::from_secs(1),
            1024,
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    #[cfg(windows)]
    fn guarded_runner_contains_a_descendant_when_the_root_times_out() {
        let root = tempfile::tempdir().unwrap();
        let mut command = helper_command("descendant");
        command.env(TREE_ROOT_ENV, root.path());
        let output = run_guarded_with_output_timeout(
            command,
            guardian_command(),
            Duration::from_secs(1),
            1024,
        )
        .unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(wait_for_exit(read_pid(&root.path().join("root"))));
        assert!(wait_for_exit(read_pid(&root.path().join("descendant"))));
    }

    #[test]
    #[cfg(all(unix, not(target_os = "linux")))]
    fn guarded_runner_rejects_unsupported_containment_before_spawn() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("target-spawned");
        let guardian_marker = root.path().join("guardian-spawned");
        let mut command = helper_command("mark");
        command.env(MARKER_ENV, &marker);
        let mut guardian = helper_command("mark");
        guardian.env(MARKER_ENV, &guardian_marker);
        let error =
            run_guarded_with_output_timeout(command, guardian, Duration::from_secs(1), 1024)
                .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(!marker.exists());
        assert!(!guardian_marker.exists());
    }

    #[cfg(target_os = "linux")]
    fn spawn_setsid_descendant(wait: bool) {
        let root = std::path::PathBuf::from(std::env::var_os(TREE_ROOT_ENV).unwrap());
        std::fs::write(root.join("root"), std::process::id().to_string()).unwrap();
        let mut descendant = helper_command(if wait { "sleep" } else { "observe_root_exit" });
        crate::isolate_owned_session(&mut descendant).unwrap();
        let mut descendant = descendant.spawn().unwrap();
        std::fs::write(root.join("descendant"), descendant.id().to_string()).unwrap();
        if wait {
            let _ = descendant.wait();
        } else {
            std::thread::spawn(move || {
                let _ = descendant.wait();
            });
        }
    }

    #[cfg(any(target_os = "linux", windows))]
    fn guardian_command() -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "bounded_output::guarded::tests::bounded_output_guardian_helper",
            "--nocapture",
        ]);
        command
    }

    fn helper_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "bounded_output::guarded::tests::bounded_output_child_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, mode);
        command
    }

    #[cfg(any(target_os = "linux", windows))]
    fn text(output: &CapturedOutput) -> String {
        String::from_utf8_lossy(output.as_bytes()).into_owned()
    }

    #[cfg(any(target_os = "linux", windows))]
    fn read_pid(path: &std::path::Path) -> u32 {
        std::fs::read_to_string(path).unwrap().parse().unwrap()
    }

    #[cfg(any(target_os = "linux", windows))]
    fn wait_for_exit(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while crate::is_pid_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        !crate::is_pid_alive(pid)
    }
}
