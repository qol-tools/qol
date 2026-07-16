mod platform;

use std::io;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const PROCESS_TREE_GUARDIAN_COMMAND: &str = "__process-tree-guardian";

#[derive(Clone, Debug)]
pub struct CancellationToken {
    local: Arc<AtomicBool>,
    observe_process_signals: bool,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            local: Arc::new(AtomicBool::new(false)),
            observe_process_signals: false,
        }
    }

    pub fn install() -> io::Result<Self> {
        platform::install_cancellation_handler()?;
        Ok(Self {
            local: Arc::new(AtomicBool::new(false)),
            observe_process_signals: true,
        })
    }

    pub fn cancel(&self) {
        self.local.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.local.load(Ordering::Acquire)
            || self.observe_process_signals && platform::cancellation_requested()
    }

    pub fn escalation_requested(&self) -> bool {
        self.observe_process_signals && platform::cancellation_signal_count() >= 2
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ProcessTreeGuard {
    _inner: platform::ProcessTreeGuard,
}

#[must_use]
pub struct PreparedCommand<'guard> {
    guard: &'guard ProcessTreeGuard,
    command: Option<Command>,
    prepared: Option<platform::PreparedSpawn>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedSpawnCleanup {
    NotStarted,
    Verified,
    RecoveryPending,
}

#[derive(Debug)]
pub struct PreparedSpawnError {
    source: io::Error,
    cleanup: PreparedSpawnCleanup,
}

pub(crate) struct PlatformSpawnFailure {
    pub(crate) source: io::Error,
    pub(crate) cleanup: PreparedSpawnCleanup,
}

#[derive(Debug)]
#[must_use]
pub struct TerminatedProcessTree {
    _private: (),
}

pub struct CurrentProcessTreeGuard {
    inner: platform::CurrentProcessTreeGuard,
}

impl ProcessTreeGuard {
    pub fn prepare_command(&self, mut command: Command) -> io::Result<PreparedCommand<'_>> {
        let prepared = self._inner.prepare_command(&mut command)?;
        Ok(PreparedCommand {
            guard: self,
            command: Some(command),
            prepared: Some(prepared),
        })
    }

    pub fn terminate_and_wait(&self, timeout: Duration) -> io::Result<TerminatedProcessTree> {
        self._inner.terminate_and_wait(timeout)?;
        Ok(TerminatedProcessTree { _private: () })
    }

    pub fn recover_pending_spawn(&self, timeout: Duration) -> io::Result<TerminatedProcessTree> {
        self._inner.recover_pending_spawn(timeout)?;
        Ok(TerminatedProcessTree { _private: () })
    }

    pub fn terminate_root_and_wait(&self, timeout: Duration) -> io::Result<()> {
        self._inner.terminate_root_and_wait(timeout)
    }

    pub fn root_has_exited(&self) -> io::Result<bool> {
        self._inner.root_has_exited()
    }
}

impl PreparedCommand<'_> {
    pub fn spawn(mut self) -> Result<Child, PreparedSpawnError> {
        let command = self.command.as_mut().ok_or_else(|| PreparedSpawnError {
            source: io::Error::other("prepared command was already consumed"),
            cleanup: PreparedSpawnCleanup::RecoveryPending,
        })?;
        let prepared = self.prepared.take().ok_or_else(|| PreparedSpawnError {
            source: io::Error::other("prepared process ownership was already consumed"),
            cleanup: PreparedSpawnCleanup::RecoveryPending,
        })?;
        self.guard
            ._inner
            .spawn_prepared(command, prepared)
            .map_err(PreparedSpawnError::from)
    }
}

impl PreparedSpawnError {
    pub fn cleanup(&self) -> PreparedSpawnCleanup {
        self.cleanup
    }
}

impl From<PlatformSpawnFailure> for PreparedSpawnError {
    fn from(failure: PlatformSpawnFailure) -> Self {
        Self {
            source: failure.source,
            cleanup: failure.cleanup,
        }
    }
}

impl std::fmt::Display for PreparedSpawnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for PreparedSpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl Drop for PreparedCommand<'_> {
    fn drop(&mut self) {
        if self.prepared.take().is_some() {
            self.guard._inner.abort_prepared();
        }
    }
}

impl CurrentProcessTreeGuard {
    pub fn disarm(&mut self) -> io::Result<()> {
        self.inner.disarm()
    }
}

pub fn own_current_process_tree_with_guardian(
    guardian_command: Command,
) -> io::Result<ProcessTreeGuard> {
    Ok(ProcessTreeGuard {
        _inner: platform::own_current_process_tree_with_guardian(guardian_command)?,
    })
}

pub fn process_tree_guardian_command(executable: &std::path::Path) -> Command {
    let mut command = Command::new(executable);
    command.arg(PROCESS_TREE_GUARDIAN_COMMAND);
    command
}

pub fn run_process_tree_guardian_entry() -> io::Result<()> {
    platform::run_process_tree_guardian_entry()
}

pub fn process_tree_containment_support() -> io::Result<()> {
    platform::process_tree_containment_support()
}

pub fn guard_current_process_tree() -> io::Result<CurrentProcessTreeGuard> {
    Ok(CurrentProcessTreeGuard {
        inner: platform::guard_current_process_tree()?,
    })
}

pub fn isolate_owned_command(command: &mut Command) -> io::Result<()> {
    platform::isolate_owned_command(command)
}

pub fn isolate_owned_session(command: &mut Command) -> io::Result<()> {
    platform::isolate_owned_session(command)
}

pub fn is_pid_alive(pid: u32) -> bool {
    platform::is_pid_alive(pid)
}

pub fn is_group_alive(pid: u32) -> bool {
    platform::is_group_alive(pid)
}

pub fn is_pid_zombie(pid: u32) -> bool {
    platform::is_pid_zombie(pid)
}

pub fn process_identity(pid: u32) -> io::Result<String> {
    platform::process_identity(pid)
}

pub fn process_identity_matches(pid: u32, expected: &str) -> bool {
    process_identity(pid).is_ok_and(|actual| actual == expected)
}

pub fn signal_term_pid(pid: u32) -> io::Result<()> {
    platform::signal_term_pid(pid)
}

pub fn kill_pid(pid: u32) -> io::Result<()> {
    platform::kill_pid(pid)
}

pub fn try_wait_pid(pid: u32) -> io::Result<Option<ExitStatus>> {
    platform::try_wait_pid(pid)
}

pub fn wait_pid(pid: u32) -> io::Result<ExitStatus> {
    platform::wait_pid(pid)
}

pub fn terminate_pid(pid: u32, grace: Duration) {
    platform::terminate_pid(pid, grace);
}

pub fn terminate_group(pid: u32, grace: Duration) {
    platform::terminate_group(pid, grace);
}

pub fn terminate_owned(child: &mut Child, grace: Duration) -> io::Result<()> {
    platform::terminate_owned(child, grace)
}

pub fn reap_children_nonblocking() {
    platform::reap_children_nonblocking();
}

/// Spawns a command without inherited standard streams or a parent-owned child
/// process that needs later cleanup.
pub fn spawn_detached(command: &mut Command) -> io::Result<()> {
    platform::spawn_detached(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn current_process_is_alive() {
        assert!(is_pid_alive(std::process::id()));
    }

    #[test]
    fn process_identity_distinguishes_the_current_process_from_stale_evidence() {
        let pid = std::process::id();
        let identity = process_identity(pid).unwrap();
        assert!(!identity.is_empty());
        assert!(process_identity_matches(pid, &identity));
        assert!(!process_identity_matches(pid, "stale-process-identity"));
        assert!(process_identity(0).is_err());
    }

    #[test]
    fn manual_cancellation_is_shared_and_idempotent() {
        let token = CancellationToken::new();
        let peer = token.clone();
        assert!(!token.is_cancelled());
        peer.cancel();
        peer.cancel();
        assert!(token.is_cancelled());
        assert!(!token.escalation_requested());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_signal_child_helper() {
        let Some(root) = std::env::var_os("QOL_PROCESS_CANCELLATION_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let token = CancellationToken::install().unwrap();
        std::fs::write(root.join("ready"), "ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !token.is_cancelled() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(token.is_cancelled());
        std::fs::write(root.join("cancelled"), "cancelled").unwrap();
        if std::env::var_os("QOL_PROCESS_EXPECT_ESCALATION").is_none() {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !token.escalation_requested() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(token.escalation_requested());
        std::fs::write(root.join("escalated"), "escalated").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn installed_handler_turns_sigterm_into_observable_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::cancellation_signal_child_helper"])
            .env("QOL_PROCESS_CANCELLATION_TEST_ROOT", temp.path())
            .spawn()
            .unwrap();
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while !temp.path().join("ready").exists() && Instant::now() < ready_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(temp.path().join("ready").exists());
        signal_term_pid(child.id()).unwrap();
        let exit_deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < exit_deadline);
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("cancelled")).unwrap(),
            "cancelled"
        );
    }

    #[cfg(unix)]
    #[test]
    fn second_signal_requests_escalation_after_graceful_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::cancellation_signal_child_helper"])
            .env("QOL_PROCESS_CANCELLATION_TEST_ROOT", temp.path())
            .env("QOL_PROCESS_EXPECT_ESCALATION", "1")
            .spawn()
            .unwrap();
        wait_for_path(&temp.path().join("ready"));
        signal_term_pid(child.id()).unwrap();
        wait_for_path(&temp.path().join("cancelled"));
        assert!(child.try_wait().unwrap().is_none());
        assert!(!temp.path().join("escalated").exists());
        signal_term_pid(child.id()).unwrap();
        let status = child.wait().unwrap();
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("escalated")).unwrap(),
            "escalated"
        );
    }

    #[cfg(unix)]
    fn wait_for_path(path: &std::path::Path) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {}", path.display());
    }

    #[test]
    fn zero_pid_is_never_a_process_target() {
        assert!(!is_pid_alive(0));
        assert!(!is_group_alive(0));
        assert!(!is_pid_zombie(0));
        assert!(signal_term_pid(0).is_err());
        assert!(kill_pid(0).is_err());
        assert!(try_wait_pid(0).is_err());
        assert!(wait_pid(0).is_err());
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::zombie_processes)]
    fn terminate_group_stops_the_leader_and_descendants() {
        use std::os::unix::process::CommandExt;

        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let pid = child.id();
        assert!(is_group_alive(pid));

        terminate_group(pid, Duration::from_secs(1));

        assert!(!is_group_alive(pid));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_unreaped_process_is_a_zombie_until_waited() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !is_pid_zombie(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(is_pid_zombie(pid));
        child.wait().unwrap();
        assert!(!is_pid_zombie(pid));
    }

    #[test]
    fn detached_child_helper() {
        let Some(marker) = std::env::var_os("QOL_PROCESS_DETACHED_TEST_MARKER") else {
            return;
        };
        std::fs::write(marker, "ready").unwrap();
    }

    #[test]
    fn detached_spawn_runs_after_the_owned_child_is_released() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("detached-ready");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "tests::detached_child_helper"])
            .env("QOL_PROCESS_DETACHED_TEST_MARKER", &marker);

        spawn_detached(&mut command).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "ready");
    }

    #[test]
    fn detached_spawn_reports_a_missing_program() {
        let mut command = Command::new("qol-process-command-that-does-not-exist");
        assert_eq!(
            spawn_detached(&mut command).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
