use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use qol_host_fixes::elevation::{
    run_privileged, spawn_privileged_piped, wait_until_authorized, PrivilegedExit, PromptTimedOut,
};

use super::{LABEL, SHELL};
use crate::platform::Refusal;

const READY: &str = "ready";
const POLL: Duration = Duration::from_millis(200);
const DRAIN_WAIT: Duration = Duration::from_secs(5);

pub(crate) fn spawn(script: &str, args: &[String]) -> Result<Child, Refusal> {
    let mut all: Vec<OsString> = vec!["-c".into(), script.into(), LABEL.into()];
    all.extend(args.iter().map(OsString::from));
    spawn_privileged_piped(LABEL, Path::new(SHELL), &all)
        .map_err(|error| Refusal::Failed(format!("{error:#}")))
}

pub(crate) fn refusal(status: ExitStatus) -> Refusal {
    match PrivilegedExit::check(status) {
        Ok(()) => Refusal::Failed("the helper stopped".into()),
        Err(error) => classify(&error),
    }
}

pub(crate) fn classify(error: &anyhow::Error) -> Refusal {
    if error.is::<PromptTimedOut>() {
        return Refusal::TimedOut;
    }
    match error.downcast_ref::<PrivilegedExit>() {
        Some(exit) if exit.dismissed() => Refusal::Dismissed,
        Some(exit) if exit.not_authorized() => Refusal::Denied,
        _ => Refusal::Failed(format!("{error:#}")),
    }
}

pub(crate) fn refused(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("not authorized")
        || stderr.contains("access denied")
        || stderr.contains("interactive authentication required")
}

pub(crate) fn run_script(script: &str, args: &[String]) -> Result<(), Refusal> {
    run_privileged(LABEL, script, args).map_err(|error| classify(&error))
}

pub(crate) fn run_long_script(
    script: &str,
    args: &[String],
    limit: Duration,
) -> Result<(), Refusal> {
    let mut child = spawn(&format!("echo {READY}\nexec 2>&1\n{script}"), args)?;
    drop(child.stdin.take());
    wait_until_authorized(&mut child, READY).map_err(|error| classify(&error))?;
    follow(child, limit)
}

fn follow(mut child: Child, limit: Duration) -> Result<(), Refusal> {
    let (sender, last) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            let line = BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .last();
            let _ = sender.send(line);
        });
    }
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                if child.kill().is_ok() {
                    let _ = child.wait();
                }
                return Err(Refusal::Failed(format!(
                    "it did not finish within {} minutes and may still be running",
                    limit.as_secs() / 60
                )));
            }
            Ok(None) => thread::sleep(POLL),
            Err(error) => return Err(Refusal::Failed(error.to_string())),
        }
    };
    if status.success() {
        return Ok(());
    }
    let said = last.recv_timeout(DRAIN_WAIT).ok().flatten();
    Err(Refusal::Failed(
        said.unwrap_or_else(|| format!("it stopped with {status}")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polkit_refusals_are_recognised() {
        assert!(refused(
            "Failed to reboot system via logind: Interactive authentication required."
        ));
        assert!(refused("Access denied"));
        assert!(!refused("Unit not found"));
    }

    fn authorized(script: &str) -> Child {
        let mut child = std::process::Command::new(SHELL)
            .args(["-c", script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("sh starts");
        wait_until_authorized(&mut child, READY).expect("the ready line");
        child
    }

    #[test]
    fn a_long_run_reports_its_last_line_and_stops_at_its_limit() {
        let long = Duration::from_secs(30);
        assert_eq!(follow(authorized("echo ready; echo working"), long), Ok(()));
        assert_eq!(
            follow(
                authorized("echo ready; echo first; echo E: it broke; exit 3"),
                long
            ),
            Err(Refusal::Failed("E: it broke".into()))
        );
        assert_eq!(
            follow(
                authorized("echo ready; echo 'env: snap: No such file or directory'; exit 127"),
                long
            ),
            Err(Refusal::Failed(
                "env: snap: No such file or directory".into()
            )),
            "after the password, a missing program is the script's failure, not a closed prompt"
        );
        let started = Instant::now();
        let stopped = follow(
            authorized("echo ready; exec sleep 30"),
            Duration::from_millis(300),
        );
        assert!(
            matches!(stopped, Err(Refusal::Failed(ref reason)) if reason.contains("did not finish")),
            "{stopped:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_password_prompt_left_open_is_a_timed_out_refusal() {
        assert_eq!(
            classify(&anyhow::Error::from(PromptTimedOut)),
            Refusal::TimedOut
        );
    }
}
