use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, ExitStatus};
use std::time::{Duration, Instant};

use qol_host_fixes::elevation::{
    run_privileged, spawn_privileged_piped, wait_until_authorized, PrivilegedExit, PromptTimedOut,
};

use crate::updates::{Failure, Holding, Reason, Refusal};

const SHELL: &str = "/bin/sh";
const LABEL: &str = "qol-update-manager";
const STOP_WAIT: Duration = Duration::from_secs(5);
const READY: &str = "ready";
const REFRESHED: &str = "refreshed";

const HELPER_SH: &str = r#"
timer="$1"
held=0
release() {
  if [ "$held" = 1 ]; then
    held=0
    if systemctl is-enabled --quiet "$timer"; then systemctl start "$timer"; fi
  fi
}
trap 'release; exit 0' HUP INT TERM PIPE
echo ready
while IFS= read -r line; do
  case "$line" in
    hold)
      if [ "$held" = 1 ]; then echo held
      elif systemctl is-active --quiet "$timer"; then
        if systemctl stop "$timer"; then held=1; echo held; else echo failed; fi
      else echo theirs
      fi ;;
    release) release; echo released ;;
    *) echo unknown ;;
  esac
done
release
"#;

const INSTALL_SH: &str = r#"echo ready; env LANG=C LC_ALL=C apt-get update -o DPkg::Lock::Timeout=30 2>&1; echo refreshed; exec env DEBIAN_FRONTEND=noninteractive LANG=C LC_ALL=C apt-get install --only-upgrade --no-remove -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold -o DPkg::Lock::Timeout=30 -o APT::Status-Fd=1 -o qol::update-manager=1 "$@" 2>&1"#;

pub(crate) struct Helper {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Helper {
    pub(crate) fn start() -> Result<Self, Refusal> {
        let mut child = spawn(HELPER_SH, &[super::mint::TIMER.to_string()])?;
        wait_until_authorized(&mut child, READY).map_err(|error| classify(&error))?;
        let stdin = child.stdin.take();
        let Some(stdout) = child.stdout.take() else {
            return Err(Refusal::Failed("the helper has no output".into()));
        };
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub(crate) fn hold(&mut self) -> Result<Holding, Refusal> {
        match self.ask("hold")?.as_str() {
            "held" => Ok(Holding::Ours),
            "theirs" => Ok(Holding::Theirs),
            _ => Err(Refusal::Failed("Mint's timer would not stop".into())),
        }
    }

    pub(crate) fn release(&mut self) -> Result<(), Refusal> {
        match self.ask("release")?.as_str() {
            "released" => Ok(()),
            other => Err(Refusal::Failed(format!("the helper answered {other}"))),
        }
    }

    fn ask(&mut self, line: &str) -> Result<String, Refusal> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Refusal::Failed("the helper has stopped".into()))?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|_| Refusal::Failed("the helper has stopped".into()))?;
        self.answer()
    }

    fn answer(&mut self) -> Result<String, Refusal> {
        let mut line = String::new();
        match self.stdout.read_line(&mut line) {
            Ok(0) | Err(_) => Err(self
                .child
                .wait()
                .map_or_else(|error| Refusal::Failed(error.to_string()), refusal)),
            Ok(_) => Ok(line.trim().to_string()),
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + STOP_WAIT;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(script: &str, args: &[String]) -> Result<Child, Refusal> {
    let mut all: Vec<OsString> = vec!["-c".into(), script.into(), LABEL.into()];
    all.extend(args.iter().map(OsString::from));
    spawn_privileged_piped(LABEL, Path::new(SHELL), &all)
        .map_err(|error| Refusal::Failed(format!("{error:#}")))
}

fn refusal(status: ExitStatus) -> Refusal {
    match PrivilegedExit::check(status) {
        Ok(()) => Refusal::Failed("the helper stopped".into()),
        Err(error) => classify(&error),
    }
}

fn classify(error: &anyhow::Error) -> Refusal {
    if error.is::<PromptTimedOut>() {
        return Refusal::TimedOut;
    }
    match error.downcast_ref::<PrivilegedExit>() {
        Some(exit) if exit.dismissed() => Refusal::Dismissed,
        Some(exit) if exit.not_authorized() => Refusal::Denied,
        _ => Refusal::Failed(format!("{error:#}")),
    }
}

pub(super) fn refused(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("not authorized")
        || stderr.contains("access denied")
        || stderr.contains("interactive authentication required")
}

pub(super) fn run_script(script: &str, args: &[String]) -> Result<(), Refusal> {
    run_privileged(LABEL, script, args).map_err(|error| classify(&error))
}

pub(super) fn install_apt(
    targets: &[String],
    log: &mut File,
    progress: &mut dyn FnMut(usize),
) -> Result<usize, Failure> {
    let failed = |done: usize, refusal: Refusal| Failure {
        done,
        reason: Reason::Refused(refusal),
    };
    let mut child = spawn(INSTALL_SH, targets).map_err(|refusal| failed(0, refusal))?;
    drop(child.stdin.take());
    wait_until_authorized(&mut child, READY).map_err(|error| failed(0, classify(&error)))?;
    progress(0);
    let mut seen = BTreeSet::new();
    let mut scan = Scan::default();
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            let _ = writeln!(log, "{line}");
            if let Some(installed) = scan.line(&line) {
                if seen.insert(installed) {
                    progress(seen.len());
                }
            }
        }
    }
    let done = seen.len();
    let status = child
        .wait()
        .map_err(|error| failed(done, Refusal::Failed(error.to_string())))?;
    if status.success() {
        return Ok(done);
    }
    let reason = match status.code() {
        Some(126 | 127) => Reason::Refused(refusal(status)),
        _ => scan.reason(),
    };
    Err(Failure { done, reason })
}

#[derive(Debug, Default)]
pub(super) struct Scan {
    holder: Option<Option<String>>,
    error: Option<String>,
}

impl Scan {
    pub(super) fn line(&mut self, line: &str) -> Option<String> {
        if line == REFRESHED {
            *self = Scan::default();
            return None;
        }
        if let Some((_, rest)) = line.split_once("held by process ") {
            let name = rest
                .split_once('(')
                .and_then(|(_, name)| name.split_once(')'))
                .map(|(name, _)| holder(name));
            self.holder = Some(name);
        }
        if let Some(error) = line.strip_prefix("E: ") {
            self.error = Some(error.trim().to_string());
        }
        let message = line.strip_prefix("pmstatus:")?;
        let (_, installed) = message.split_once(":Installed ")?;
        Some(installed.trim().to_string())
    }

    pub(super) fn reason(self) -> Reason {
        match (self.holder, self.error) {
            (Some(holder), Some(error)) if error.contains("lock") => Reason::Locked(holder),
            (_, Some(error)) => Reason::Stopped(error),
            (_, None) => Reason::Stopped("The installer stopped without saying why".into()),
        }
    }
}

fn holder(process: &str) -> String {
    let lower = process.to_ascii_lowercase();
    if lower.contains("mintupdate") {
        "Mint's Update Manager".into()
    } else if lower.contains("unattended") {
        "Unattended upgrades".into()
    } else if lower.contains("synaptic") {
        "Synaptic".into()
    } else if lower.starts_with("apt") || lower.starts_with("dpkg") {
        format!("Another {process}")
    } else {
        process.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_lines_count_once_and_locks_name_their_holder() {
        let mut scan = Scan::default();
        let lines = [
            "pmstatus:libnvidia-gl-580:12.5:Preparing to configure libnvidia-gl-580 (amd64)",
            "pmstatus:libnvidia-gl-580:20.0:Installed libnvidia-gl-580 (amd64)",
            "pmstatus:dpkg-exec:50.0:Running dpkg",
        ];
        let counted: Vec<Option<String>> = lines.iter().map(|line| scan.line(line)).collect();
        assert_eq!(
            counted,
            [None, Some("libnvidia-gl-580 (amd64)".into()), None]
        );
        let mut locked = Scan::default();
        locked.line("Waiting for cache lock: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 4242 (mintUpdate)");
        locked.line("E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 4242 (mintUpdate)");
        assert_eq!(
            locked.reason(),
            Reason::Locked(Some("Mint's Update Manager".into()))
        );
        let mut broken = Scan::default();
        broken.line("E: Packages need to be removed but remove is disabled.");
        assert_eq!(
            broken.reason(),
            Reason::Stopped("Packages need to be removed but remove is disabled.".into())
        );
        assert_eq!(
            Scan::default().reason(),
            Reason::Stopped("The installer stopped without saying why".into())
        );
    }

    #[test]
    fn a_refresh_failure_is_not_the_install_reason() {
        let mut scan = Scan::default();
        scan.line("Waiting for cache lock: Could not get lock /var/lib/apt/lists/lock. It is held by process 4242 (mintUpdate)");
        scan.line("E: Could not get lock /var/lib/apt/lists/lock. It is held by process 4242 (mintUpdate)");
        assert_eq!(scan.line(REFRESHED), None);
        scan.line("E: Packages need to be removed but remove is disabled.");
        assert_eq!(
            scan.reason(),
            Reason::Stopped("Packages need to be removed but remove is disabled.".into())
        );
        let mut quiet = Scan::default();
        quiet.line("E: Failed to fetch http://archive.ubuntu.com/ubuntu/dists/noble/InRelease");
        quiet.line(REFRESHED);
        assert_eq!(
            quiet.reason(),
            Reason::Stopped("The installer stopped without saying why".into())
        );
    }

    #[test]
    fn polkit_refusals_are_recognised() {
        assert!(refused(
            "Failed to reboot system via logind: Interactive authentication required."
        ));
        assert!(refused("Access denied"));
        assert!(!refused("Unit not found"));
        assert_eq!(holder("unattended-upgr"), "Unattended upgrades");
        assert_eq!(holder("apt-get"), "Another apt-get");
    }

    #[test]
    fn a_password_prompt_left_open_is_a_timed_out_refusal() {
        assert_eq!(
            classify(&anyhow::Error::from(PromptTimedOut)),
            Refusal::TimedOut
        );
        assert_eq!(
            Reason::Refused(Refusal::TimedOut).label(),
            "The password prompt timed out"
        );
        assert!(INSTALL_SH.starts_with("echo ready;"));
        let order: Vec<usize> = [
            "echo ready;",
            "apt-get update -o DPkg::Lock::Timeout=30 2>&1;",
            "echo refreshed;",
            "exec env DEBIAN_FRONTEND=noninteractive LANG=C LC_ALL=C apt-get install ",
        ]
        .iter()
        .filter_map(|step| INSTALL_SH.find(step))
        .collect();
        assert_eq!(order.len(), 4);
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(HELPER_SH.contains("echo ready\n"));
    }
}
