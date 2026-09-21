mod backends;

use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::bluetooth::normalize_address;
use backends::pulse_streams;
use qol_audio::attempts::lease::{self, Scope};

pub const RECLAIM_SUPPORTED: bool = true;

const RECLAIM_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

enum ReclaimAttempt {
    Done,
    Failed(anyhow::Error),
    Skipped(String),
}

fn reclaim_locked(sink: &str) -> ReclaimAttempt {
    let _global = match lease::acquire_within(Scope::GlobalDefault, RECLAIM_LOCK_TIMEOUT) {
        Ok(lease) => lease,
        Err(_) => return ReclaimAttempt::Skipped(lock_holder(&Scope::GlobalDefault)),
    };
    match pulse_streams::suspend_resume(sink) {
        Ok(()) => ReclaimAttempt::Done,
        Err(error) => ReclaimAttempt::Failed(error),
    }
}

fn lock_holder(scope: &Scope) -> String {
    match lease::holder(scope) {
        Ok(Some(holder)) => format!("{}:{}", holder.owner, holder.pid),
        Ok(None) | Err(_) => "unknown".to_string(),
    }
}

pub fn reclaim_output(address: &str) -> Result<()> {
    let address = normalize_address(address)?;
    let sink = match pulse_streams::bluetooth_sink(&address) {
        Ok(sink) => sink,
        Err(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual address={address} outcome=failed reason=no_output"
            );
            return Err(error);
        }
    };
    match reclaim_locked(&sink) {
        ReclaimAttempt::Done => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual sink={sink} outcome=ok"
            );
            Ok(())
        }
        ReclaimAttempt::Failed(error) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual sink={sink} outcome=failed"
            );
            Err(error)
        }
        ReclaimAttempt::Skipped(holder) => {
            qol_runtime::probe!(
                "BLUETOOTH_AUDIO_CLAIM",
                "event=reclaim trigger=manual sink={sink} outcome=skipped reason=sound_lock_busy holder={}",
                qol_runtime::probe::token(&holder)
            );
            Err(anyhow!("the sound lock for {sink} is held by {holder}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::path::Path;
    use std::process::{Child, ChildStdout, Command, Stdio};

    use super::*;

    const CHILD_ENV: &str = "QOL_BLUETOOTH_RECLAIM_CHILD";
    const CHILD_RAN: &str = "QOL_BLUETOOTH_RECLAIM_CHILD_RAN";
    const CHILD_SKIPPED: &str = "QOL_BLUETOOTH_RECLAIM_CHILD_SKIPPED";
    const CHILD_HOLDER_PID: &str = "QOL_BLUETOOTH_RECLAIM_CHILD_HOLDER_PID";
    const HOLDER_ENV: &str = "QOL_BLUETOOTH_RECLAIM_HOLDER";
    const HOLDER_HELD: &str = "QOL_BLUETOOTH_RECLAIM_HOLDER_HELD";

    struct HolderChild {
        child: Option<Child>,
        output: BufReader<ChildStdout>,
    }

    impl HolderChild {
        fn spawn(root: &Path) -> Self {
            let mut command = Command::new(std::env::current_exe().expect("the test binary"));
            let mut child = command
                .arg("--exact")
                .arg("--nocapture")
                .arg("audio_claim::platform::linux::tests::reclaim_holder_child")
                .env(HOLDER_ENV, "1")
                .env("XDG_DATA_HOME", root)
                .env("HOME", root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("the holder probe runs");
            let output = BufReader::new(child.stdout.take().expect("the holder stdout"));
            Self {
                child: Some(child),
                output,
            }
        }

        fn pid(&mut self) -> u32 {
            let mut seen = String::new();
            for line in self.output.by_ref().lines() {
                let line = line.expect("the holder output");
                if let Some(index) = line.find(HOLDER_HELD) {
                    let raw = line[index + HOLDER_HELD.len()..].trim();
                    return raw.parse().expect("the holder pid is numeric");
                }
                seen.push_str(&line);
                seen.push('\n');
            }
            panic!("the holder probe never reported {HOLDER_HELD}: {seen:?}");
        }

        fn finish(mut self) {
            let mut child = self.child.take().expect("the holder child");
            drop(child.stdin.take());
            let _ = child.wait();
        }
    }

    impl Drop for HolderChild {
        fn drop(&mut self) {
            if let Some(child) = self.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    #[test]
    fn a_held_global_lock_skips_the_reclaim_and_names_the_holder() {
        let root = tempfile::tempdir().expect("an isolated sound state root");
        let mut holder = HolderChild::spawn(root.path());
        let holder_pid = holder.pid();
        let mut command =
            std::process::Command::new(std::env::current_exe().expect("the test binary"));
        let output = command
            .arg("--exact")
            .arg("--nocapture")
            .arg("audio_claim::platform::linux::tests::reclaim_probe_child")
            .env(CHILD_ENV, "1")
            .env(CHILD_HOLDER_PID, holder_pid.to_string())
            .env("XDG_DATA_HOME", root.path())
            .env("HOME", root.path())
            .output()
            .expect("the child probe runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains(CHILD_RAN),
            "the child probe never ran: {stdout:?} {stderr:?}"
        );
        assert!(
            stdout.contains(CHILD_SKIPPED),
            "the child probe never reached the skip verdict: {stdout:?} {stderr:?}"
        );
        assert!(
            output.status.success(),
            "the child probe failed: {stdout:?} {stderr:?}"
        );
        holder.finish();
    }

    #[test]
    fn reclaim_holder_child() {
        if std::env::var_os(HOLDER_ENV).is_none() {
            return;
        }
        let _lease = lease::acquire(Scope::GlobalDefault).expect("the global lock");
        println!("{HOLDER_HELD} {}", std::process::id());
        std::io::stdout()
            .flush()
            .expect("the holder sentinel flushes");
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .expect("the holder waits for stdin");
    }

    #[test]
    fn reclaim_probe_child() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        println!("{CHILD_RAN}");
        let holder_pid = std::env::var(CHILD_HOLDER_PID)
            .expect("the holder pid")
            .parse::<u32>()
            .expect("the holder pid is numeric");
        match reclaim_locked("bluez_output.AA_BB_CC_DD_EE_FF.1") {
            ReclaimAttempt::Skipped(holder) => {
                let (owner, pid) = holder.split_once(':').expect("the holder is owner:pid");
                assert!(!owner.is_empty(), "the holder names its owner");
                assert_eq!(
                    pid.parse::<u32>().expect("the holder pid is numeric"),
                    holder_pid,
                    "the holder names the process holding the lock"
                );
                println!("{CHILD_SKIPPED} {holder}");
            }
            ReclaimAttempt::Done => {
                panic!("the reclaim must not reach the audio server while the lock is held")
            }
            ReclaimAttempt::Failed(error) => panic!(
                "the reclaim must not reach the audio server while the lock is held: {error:#}"
            ),
        }
    }
}
