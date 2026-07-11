use std::io;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ExitStatus};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use super::STOP_GRACE;

pub(crate) enum TrayHandle {
    Owned(Child),
    Attached(u32),
}

impl TrayHandle {
    pub(crate) fn id(&self) -> u32 {
        match self {
            Self::Owned(child) => child.id(),
            Self::Attached(pid) => *pid,
        }
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match self {
            Self::Owned(child) => child.try_wait(),
            Self::Attached(pid) => qol_process::try_wait_pid(*pid),
        }
    }

    pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Owned(child) => child.wait(),
            Self::Attached(pid) => qol_process::wait_pid(*pid),
        }
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        match self {
            Self::Owned(child) => child.kill(),
            Self::Attached(pid) => qol_process::kill_pid(*pid),
        }
    }

    pub(crate) fn signal_term(&self) {
        match self {
            Self::Owned(child) => {
                let _ = qol_process::signal_term_pid(child.id());
            }
            Self::Attached(pid) => {
                let _ = qol_process::signal_term_pid(*pid);
            }
        }
    }
}

pub(super) fn try_wait(child: &mut TrayHandle) -> Result<Option<ExitStatus>> {
    child
        .try_wait()
        .context("failed polling qol-tray dev process")
}

pub(super) fn stop_child(child: &mut TrayHandle) -> Result<()> {
    terminate_child(child);
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if try_wait(child)?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    child.wait().context("failed to reap qol-tray after kill")?;
    Ok(())
}

pub(super) fn terminate_child(child: &mut TrayHandle) {
    child.signal_term();
}

pub(crate) fn spawn_forwarders(child: &mut Child) -> Receiver<String> {
    let (tx, rx) = channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_forwarder(stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_forwarder(stderr, tx);
    }
    rx
}

fn spawn_forwarder(reader: impl Read + Send + 'static, tx: Sender<String>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => return,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if tx.send(line.to_string()).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(format!("[qol dev] log stream error: {error}"));
                    return;
                }
            }
        }
    });
}

#[cfg(all(test, unix))]
#[allow(clippy::zombie_processes)]
mod tests {
    // These tests reap the spawned process through TrayHandle::Attached's own
    // waitpid-based wait()/kill(), not through the Child binding directly -
    // that's the exact behavior under test, but clippy can't trace it.
    use super::*;
    use std::process::Command;

    fn spawn_sleep(seconds: u32) -> Child {
        Command::new("sleep")
            .arg(seconds.to_string())
            .spawn()
            .expect("failed to spawn sleep")
    }

    #[test]
    fn attached_try_wait_reports_none_while_running() {
        let child = spawn_sleep(5);
        let pid = child.id();
        let mut handle = TrayHandle::Attached(pid);

        assert_eq!(
            handle.try_wait().unwrap(),
            None,
            "a still-running attached pid must report None, not an exit status"
        );

        handle.kill().unwrap();
        handle.wait().unwrap();
    }

    #[test]
    fn attached_wait_blocks_until_exit_and_reaps() {
        let child = spawn_sleep(0);
        let pid = child.id();
        let mut handle = TrayHandle::Attached(pid);

        let status = handle.wait().unwrap();
        assert!(
            status.success(),
            "sleep 0 should exit successfully, got: {status:?}"
        );
    }

    #[test]
    fn attached_kill_terminates_a_running_process() {
        let child = spawn_sleep(30);
        let pid = child.id();
        let mut handle = TrayHandle::Attached(pid);

        handle.kill().unwrap();
        let status = handle.wait().unwrap();
        assert!(
            !status.success(),
            "a killed process must not report a successful exit"
        );
    }

    #[test]
    fn owned_and_attached_report_the_same_pid() {
        let child = spawn_sleep(0);
        let pid = child.id();
        let mut owned = TrayHandle::Owned(child);
        assert_eq!(owned.id(), pid);
        owned.wait().unwrap();

        let attached = TrayHandle::Attached(pid);
        assert_eq!(attached.id(), pid);
    }

    #[test]
    fn forwarder_decodes_non_utf8_lossily_and_ends_on_eof() {
        let (tx, rx) = channel();
        let data = b"ok\n\xFF\xFEbad\nlast".to_vec();
        spawn_forwarder(std::io::Cursor::new(data), tx);
        let lines: Vec<String> = rx.iter().collect();
        assert_eq!(lines.len(), 3, "all lines delivered: {lines:?}");
        assert_eq!(lines[0], "ok");
        assert!(lines[1].contains("bad"), "lossy line kept: {:?}", lines[1]);
        assert_eq!(lines[2], "last", "no trailing newline still delivered");
    }
}
