use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const PRIVILEGED_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) fn available() -> bool {
    Command::new("pkexec")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn run(label: &str, script: &str, args: &[String]) -> Result<()> {
    let mut command = Command::new("pkexec");
    command.args(["sh", "-c", script, label]);
    command.args(args);
    command.process_group(0);
    let mut child = command.spawn().context("failed to launch pkexec")?;
    let status = wait_for_privileged(&mut child, PRIVILEGED_COMMAND_TIMEOUT)
        .context("privileged pkexec command failed or timed out")?;
    if !status.success() {
        bail!("pkexec exited with {status}");
    }
    Ok(())
}

pub(crate) fn spawn(label: &str, program: &Path, args: &[OsString]) -> Result<Child> {
    let mut command = Command::new("pkexec");
    command.arg(program).args(args);
    command
        .spawn()
        .with_context(|| format!("failed to launch privileged {label}"))
}

fn wait_for_privileged(child: &mut Child, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let pid = child.id();
            let _ = qol_process::kill_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "pkexec did not finish within {}s; the privileged process tree was killed",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    #[test]
    fn a_hanging_privileged_child_is_killed_with_its_tree_after_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let root_pid_file = dir.path().join("root.pid");
        let child_pid_file = dir.path().join("child.pid");
        let script = dir.path().join("hang.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho $$ > \"{}\"\nsleep 30 &\necho $! > \"{}\"\nwait\n",
                root_pid_file.display(),
                child_pid_file.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut command = Command::new(script);
        command.process_group(0);
        let mut child = command.spawn().unwrap();
        let started = std::time::Instant::now();
        let error = wait_for_privileged(&mut child, Duration::from_millis(400)).unwrap_err();
        assert!(
            format!("{error:#}").contains("did not finish within"),
            "{error:#}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the timeout must fire within the short test bound"
        );
        let root_pid: u32 = std::fs::read_to_string(&root_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let child_pid: u32 = std::fs::read_to_string(&child_pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline
            && !(qol_process::is_pid_gone(root_pid) && qol_process::is_pid_gone(child_pid))
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            qol_process::is_pid_gone(root_pid) && qol_process::is_pid_gone(child_pid),
            "neither the privileged root nor its descendant may survive the timeout kill"
        );
    }
}
