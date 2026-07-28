use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const WAIT_INTERVAL: Duration = Duration::from_millis(10);
const TERMINATION_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedOutput {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug)]
pub struct CompletedCommandOutput {
    pub status: ExitStatus,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

#[derive(Debug)]
pub enum BoundedCommandOutput {
    Completed(CompletedCommandOutput),
    TimedOut {
        stdout: CapturedOutput,
        stderr: CapturedOutput,
    },
}

/// Runs an owned child process while bounding both its lifetime and retained output.
///
/// The command is isolated before spawning so a timeout can use the platform's
/// owned-process termination semantics. Standard output and error are drained
/// concurrently to prevent a full pipe from blocking the child; bytes beyond
/// `output_limit` are discarded while the stream continues to drain.
pub fn run_with_output_timeout(
    command: &mut Command,
    timeout: Duration,
    output_limit: usize,
) -> io::Result<BoundedCommandOutput> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timeout is too large"))?;
    crate::isolate_owned_command(command)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("child stderr was not piped"))?;

    std::thread::scope(|scope| {
        let stdout_reader = scope.spawn(|| capture_stream(stdout, output_limit));
        let stderr_reader = scope.spawn(|| capture_stream(stderr, output_limit));
        let completion = wait_for_child(&mut child, deadline);
        let stdout = join_capture(stdout_reader, "stdout")?;
        let stderr = join_capture(stderr_reader, "stderr")?;

        match completion? {
            WaitOutcome::Completed(status) => {
                Ok(BoundedCommandOutput::Completed(CompletedCommandOutput {
                    status,
                    stdout,
                    stderr,
                }))
            }
            WaitOutcome::TimedOut => Ok(BoundedCommandOutput::TimedOut { stdout, stderr }),
        }
    })
}

enum WaitOutcome {
    Completed(ExitStatus),
    TimedOut,
}

fn wait_for_child(child: &mut std::process::Child, deadline: Instant) -> io::Result<WaitOutcome> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(WaitOutcome::Completed(status)),
            Ok(None) => {}
            Err(error) => {
                let _ = crate::terminate_owned(child, TERMINATION_GRACE);
                return Err(error);
            }
        }

        let now = Instant::now();
        if now >= deadline {
            crate::terminate_owned(child, TERMINATION_GRACE)?;
            return Ok(WaitOutcome::TimedOut);
        }
        std::thread::sleep(WAIT_INTERVAL.min(deadline.duration_since(now)));
    }
}

fn capture_stream(mut stream: impl Read, output_limit: usize) -> io::Result<CapturedOutput> {
    let mut bytes = Vec::with_capacity(output_limit.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let retained = output_limit.saturating_sub(bytes.len()).min(read);
        bytes.extend_from_slice(&chunk[..retained]);
        truncated |= retained < read;
    }
    Ok(CapturedOutput { bytes, truncated })
}

fn join_capture(
    reader: std::thread::ScopedJoinHandle<'_, io::Result<CapturedOutput>>,
    stream_name: &str,
) -> io::Result<CapturedOutput> {
    reader
        .join()
        .map_err(|_| io::Error::other(format!("{stream_name} reader panicked")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const HELPER_ENV: &str = "QOL_PROCESS_BOUNDED_OUTPUT_HELPER";

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
            "sleep" => std::thread::sleep(Duration::from_secs(10)),
            other => panic!("unknown helper mode: {other}"),
        }
    }

    #[test]
    fn captures_both_output_streams() {
        let mut command = helper_command("capture");
        let output =
            run_with_output_timeout(&mut command, Duration::from_secs(5), 16 * 1024).unwrap();
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
    fn drains_large_streams_while_bounding_retained_bytes() {
        let mut command = helper_command("large");
        let output = run_with_output_timeout(&mut command, Duration::from_secs(5), 256).unwrap();
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
    fn terminates_and_reaps_a_timed_out_child() {
        let mut command = helper_command("sleep");
        let started = Instant::now();
        let output =
            run_with_output_timeout(&mut command, Duration::from_millis(25), 1024).unwrap();

        assert!(matches!(output, BoundedCommandOutput::TimedOut { .. }));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    fn helper_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "bounded_output::tests::bounded_output_child_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, mode);
        command
    }

    fn text(output: &CapturedOutput) -> String {
        String::from_utf8_lossy(output.as_bytes()).into_owned()
    }
}
