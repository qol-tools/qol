use anyhow::{Context, Result};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

pub(super) const BINARY: &str = qol_conventions::artifact::TRAY_HOST_BINARY_NAME;
const STOP_WAIT: Duration = Duration::from_secs(15);
const KILL_WAIT: Duration = Duration::from_secs(3);
const START_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const INHERITED_ENV_PREFIX: &str = "QOL_";
const RUN_FLAGS: [&str; 1] = ["--headless"];
const RUN_FLAG_PREFIXES: [&str; 1] = ["--write-mode="];

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Started {
    Answering,
    Quiet,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PutBack {
    Started(Started),
    AlreadyRunning,
    NotInstalled,
}

impl PutBack {
    pub(super) fn describe(&self) -> &'static str {
        match self {
            Self::Started(Started::Answering) => "the installed tray runs again",
            Self::Started(Started::Quiet) => {
                "the installed tray started, but it is not answering yet"
            }
            Self::AlreadyRunning => "a tray already runs, so it was left alone",
            Self::NotInstalled => "no installed tray to start",
        }
    }
}

impl Started {
    pub(super) fn describe(&self) -> &'static str {
        match self {
            Self::Answering => "runs in place of the installed tray, which stays where it is",
            Self::Quiet => "started in place of the installed tray, but it is not answering yet",
        }
    }
}

pub(super) fn run_in_place(binary: &Path) -> Result<Started> {
    stop(&main_trays()?)?;
    start(binary)
}

pub(super) fn put_back(try_binary: &Path, installed: &Path) -> Result<PutBack> {
    let trying = tray_processes()?
        .into_iter()
        .filter(|pid| super::daemon::runs_binary(*pid, try_binary))
        .collect::<Vec<_>>();
    stop(&trying)?;
    if !main_trays()?.is_empty() {
        return Ok(PutBack::AlreadyRunning);
    }
    if !installed.is_file() {
        return Ok(PutBack::NotInstalled);
    }
    start(installed).map(PutBack::Started)
}

fn tray_processes() -> Result<Vec<u32>> {
    Ok(qol_process::processes()
        .context("listing processes")?
        .into_iter()
        .filter(|process| process.exe == BINARY)
        .map(|process| process.pid)
        .collect())
}

fn main_trays() -> Result<Vec<u32>> {
    let processes = qol_process::processes().context("listing processes")?;
    let names = processes
        .iter()
        .map(|process| (process.pid, process.exe.as_str()))
        .collect::<HashMap<_, _>>();
    Ok(processes
        .iter()
        .filter(|process| process.exe == BINARY && names.get(&process.parent) != Some(&BINARY))
        .filter(|process| is_tray_run(&arguments(process.pid)))
        .map(|process| process.pid)
        .collect())
}

fn arguments(pid: u32) -> Vec<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|raw| {
            raw.split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn is_tray_run(arguments: &[String]) -> bool {
    !arguments.is_empty()
        && arguments.iter().skip(1).all(|argument| {
            RUN_FLAGS.contains(&argument.as_str())
                || RUN_FLAG_PREFIXES
                    .iter()
                    .any(|prefix| argument.starts_with(prefix))
        })
}

fn stop(pids: &[u32]) -> Result<()> {
    for pid in pids {
        qol_process::signal_term_pid(*pid).with_context(|| format!("stopping tray {pid}"))?;
    }
    if wait_until_gone(pids, STOP_WAIT) {
        return Ok(());
    }
    for pid in pids.iter().filter(|pid| !qol_process::is_pid_gone(**pid)) {
        let _ = qol_process::kill_pid(*pid);
    }
    if wait_until_gone(pids, KILL_WAIT) {
        return Ok(());
    }
    Err(anyhow::anyhow!("tray {pids:?} did not stop"))
}

fn wait_until_gone(pids: &[u32], wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if pids.iter().all(|pid| qol_process::is_pid_gone(*pid)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn start(binary: &Path) -> Result<Started> {
    let mut command = Command::new(binary);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with(INHERITED_ENV_PREFIX) {
            command.env_remove(&key);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        command.current_dir(home);
    }
    qol_process::spawn_detached(&mut command)
        .with_context(|| format!("starting {}", binary.display()))?;
    Ok(if answers_within(START_WAIT) {
        Started::Answering
    } else {
        Started::Quiet
    })
}

fn answers_within(wait: Duration) -> bool {
    let address: SocketAddr = ([127, 0, 0, 1], qol_conventions::DEFAULT_PORT).into();
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_ok() {
            return true;
        }
        std::thread::sleep(POLL);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn only_a_tray_run_counts_as_the_running_tray() {
        let runs: [&[&str]; 4] = [
            &["/home/u/.local/bin/qol-tray"],
            &["qol-tray", "--headless"],
            &["qol-tray", "--write-mode=prod"],
            &["qol-tray", "--headless", "--write-mode=dev"],
        ];
        for run in runs {
            assert!(is_tray_run(&words(run)), "{run:?}");
        }
        let helpers: [&[&str]; 6] = [
            &[],
            &["qol-tray", "__qol-settings-surface-host"],
            &[
                "qol-tray",
                "exec",
                "shortcut",
                "qol-cli-sessions-switchboard",
            ],
            &["qol-tray", "doctor"],
            &["qol-tray", "--version"],
            &["qol-tray", "defaults", "open", "folder.open"],
        ];
        for helper in helpers {
            assert!(!is_tray_run(&words(helper)), "{helper:?}");
        }
    }

    #[test]
    fn stopping_nothing_succeeds_at_once() {
        stop(&[]).unwrap();
        assert!(wait_until_gone(&[], Duration::ZERO));
    }

    #[test]
    fn each_outcome_reads_as_a_line() {
        let lines = [
            Started::Answering.describe(),
            Started::Quiet.describe(),
            PutBack::Started(Started::Answering).describe(),
            PutBack::Started(Started::Quiet).describe(),
            PutBack::AlreadyRunning.describe(),
            PutBack::NotInstalled.describe(),
        ];
        assert!(lines.iter().all(|line| !line.is_empty()));
        assert!(Started::Answering.describe().contains("installed tray"));
    }
}
