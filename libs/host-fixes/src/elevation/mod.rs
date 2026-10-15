use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

mod platform;

const DISMISSED: i32 = 126;
const NOT_AUTHORIZED: i32 = 127;
const PRIVILEGED_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const KILL_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivilegedExit {
    status: ExitStatus,
}

impl PrivilegedExit {
    pub fn check(status: ExitStatus) -> Result<()> {
        if status.success() {
            return Ok(());
        }
        Err(Self { status }.into())
    }

    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    pub fn dismissed(&self) -> bool {
        self.code() == Some(DISMISSED)
    }

    pub fn not_authorized(&self) -> bool {
        self.code() == Some(NOT_AUTHORIZED)
    }
}

impl fmt::Display for PrivilegedExit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pkexec exited with {}", self.status)
    }
}

impl std::error::Error for PrivilegedExit {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptTimedOut;

impl fmt::Display for PromptTimedOut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the password prompt was not answered within {}s",
            PRIVILEGED_COMMAND_TIMEOUT.as_secs()
        )
    }
}

impl std::error::Error for PromptTimedOut {}

pub fn available() -> bool {
    platform::available()
}

pub fn run_privileged(label: &str, script: &str, args: &[String]) -> Result<()> {
    platform::run(label, script, args)
}

pub fn spawn_privileged(label: &str, program: &Path, args: &[OsString]) -> Result<Child> {
    platform::spawn(label, program, args, false)
}

pub fn spawn_privileged_piped(label: &str, program: &Path, args: &[OsString]) -> Result<Child> {
    platform::spawn(label, program, args, true)
}

pub fn wait_until_authorized(child: &mut Child, marker: &str) -> Result<()> {
    wait_for_marker(child, marker, PRIVILEGED_COMMAND_TIMEOUT)
}

fn wait_for_marker(child: &mut Child, marker: &str, timeout: Duration) -> Result<()> {
    let mut stdout = child
        .stdout
        .take()
        .context("the privileged process has no piped output")?;
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("qol-privileged-prompt".into())
        .spawn(move || {
            let line = first_line(&mut stdout);
            let _ = sender.send((stdout, line));
        })
        .context("failed to watch the password prompt")?;
    let (stdout, line) = match receiver.recv_timeout(timeout) {
        Ok(read) => read,
        Err(_) => {
            let killed = child.kill().is_ok();
            return match receiver.recv_timeout(KILL_WAIT) {
                Ok((stdout, Ok(line))) if !killed && line == marker => {
                    child.stdout = Some(stdout);
                    Ok(())
                }
                _ => {
                    if killed {
                        let _ = child.wait();
                    }
                    Err(PromptTimedOut.into())
                }
            };
        }
    };
    child.stdout = Some(stdout);
    match line {
        Ok(line) if line == marker => Ok(()),
        Ok(line) if !line.is_empty() => {
            bail!("the privileged process answered {line} before it was authorized")
        }
        _ => {
            let status = child
                .wait()
                .context("failed to wait for the privileged process")?;
            PrivilegedExit::check(status)?;
            bail!("the privileged process ended before it was authorized")
        }
    }
}

fn first_line(stdout: &mut impl Read) -> std::io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0; 1];
    loop {
        match stdout.read_exact(&mut byte) {
            Ok(()) if byte[0] == b'\n' => break,
            Ok(()) => line.push(byte[0]),
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error),
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    fn shell(script: &str) -> Child {
        Command::new("/bin/sh")
            .args(["-c", script])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn the_marker_ends_the_wait_and_the_output_after_it_stays_readable() {
        let mut child = shell("echo ready; echo first; echo second");
        wait_for_marker(&mut child, "ready", Duration::from_secs(5)).unwrap();
        let mut rest = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut rest)
            .unwrap();
        assert_eq!(rest, "first\nsecond\n");
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_prompt_left_open_times_out_and_the_process_is_stopped() {
        let mut child = shell("exec sleep 30");
        let started = Instant::now();
        let error = wait_for_marker(&mut child, "ready", Duration::from_millis(300)).unwrap_err();
        assert!(
            error.downcast_ref::<PromptTimedOut>().is_some(),
            "{error:#}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn a_closed_prompt_is_reported_as_dismissed() {
        let mut child = shell("exit 126");
        let error = wait_for_marker(&mut child, "ready", Duration::from_secs(5)).unwrap_err();
        assert!(error
            .downcast_ref::<PrivilegedExit>()
            .is_some_and(PrivilegedExit::dismissed));
    }
}
