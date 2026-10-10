mod bounded_output;
mod observation;
mod platform;

pub use observation::{
    MemberObservation, NodeObservation, Observation, ProcessProvenance, ProcessStat,
    ProcessTreeObservation, ScopeIdentity,
};

pub use bounded_output::{
    run_guarded_with_output_timeout, run_owned_with_output_timeout, BoundedCommandOutput,
    CapturedOutput, CompletedCommandOutput,
};

use std::io;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub const PROCESS_TREE_GUARDIAN_COMMAND: &str = "__process-tree-guardian";

pub fn process_tree_guardian_requested() -> bool {
    is_process_tree_guardian_argument(std::env::args_os().nth(1).as_deref())
}

fn is_process_tree_guardian_argument(argument: Option<&std::ffi::OsStr>) -> bool {
    argument == Some(std::ffi::OsStr::new(PROCESS_TREE_GUARDIAN_COMMAND))
}

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

pub struct OwnedProcessTree {
    root_pid: u32,
    guard: Option<ProcessTreeGuard>,
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
    pub fn containment_backend(&self) -> &'static str {
        self._inner.containment_backend()
    }

    pub fn membership_observation_supported(&self) -> bool {
        self._inner.membership_observation_supported()
    }

    pub fn observe_residual(&self) -> ProcessTreeObservation {
        self._inner.observe_residual()
    }

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

    pub fn request_stop(&self) -> io::Result<()> {
        self._inner.request_stop()
    }

    pub fn force_stop_and_wait(&self, timeout: Duration) -> io::Result<TerminatedProcessTree> {
        self._inner.force_stop_and_wait(timeout)?;
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

    pub fn tree_has_exited(&self) -> io::Result<bool> {
        self._inner.tree_has_exited()
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

pub fn spawn_owned(command: Command) -> io::Result<(Child, OwnedProcessTree)> {
    let (child, guard) = platform::spawn_owned(command)?;
    let root_pid = child.id();
    Ok((
        child,
        OwnedProcessTree {
            root_pid,
            guard: guard.map(|_inner| ProcessTreeGuard { _inner }),
        },
    ))
}

impl OwnedProcessTree {
    pub fn tree_has_exited(&self) -> io::Result<bool> {
        match self.guard.as_ref() {
            Some(guard) => guard.tree_has_exited(),
            None => Ok(!platform::is_group_alive(self.root_pid)),
        }
    }

    pub fn terminate_and_wait(&self, child: &mut Child, timeout: Duration) -> io::Result<()> {
        let terminated = match self.guard.as_ref() {
            Some(guard) => guard.terminate_and_wait(timeout).map(|_| ()),
            None => {
                let root_result = platform::terminate_owned(child, timeout);
                if platform::is_group_alive(self.root_pid) {
                    platform::terminate_group(self.root_pid, timeout);
                }
                root_result
            }
        };
        match terminated {
            Ok(()) => child.wait().map(|_| ()),
            Err(error) => {
                if !self.tree_has_exited().unwrap_or(false) {
                    return Err(error);
                }
                match child.wait() {
                    Ok(_) => Err(error),
                    Err(reap) => Err(io::Error::other(format!(
                        "{error}; root-process reap failed: {reap}"
                    ))),
                }
            }
        }
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

pub fn is_pid_gone(pid: u32) -> bool {
    !platform::is_pid_alive(pid) || platform::is_pid_zombie(pid)
}

pub fn process_identity(pid: u32) -> io::Result<String> {
    platform::process_identity(pid)
}

pub fn process_identity_matches(pid: u32, expected: &str) -> bool {
    process_identity(pid).is_ok_and(|actual| platform::process_identity_matches(&actual, expected))
}

pub fn signal_term_pid(pid: u32) -> io::Result<()> {
    platform::signal_term_pid(pid)
}

pub fn signal_term_group(pid: u32) -> io::Result<()> {
    platform::signal_term_group(pid)
}

pub fn kill_pid(pid: u32) -> io::Result<()> {
    platform::kill_pid(pid)
}

pub fn kill_group(pid: u32) -> io::Result<()> {
    platform::kill_group(pid)
}

pub fn wait_for_stop_request() -> io::Result<()> {
    platform::wait_for_stop_request()
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

pub fn reload_group(pid: u32, grace: Duration) {
    platform::reload_group(pid, grace);
}

pub fn terminate_owned(child: &mut Child, grace: Duration) -> io::Result<()> {
    platform::terminate_owned(child, grace)
}

pub fn wait_for_exit_or_terminate(child: &mut Child, timeout: Duration) -> io::Result<ExitStatus> {
    wait_for_exit_or_terminate_with(
        child,
        timeout,
        |child| child.try_wait(),
        |child| terminate_owned(child, Duration::ZERO),
    )
}

fn wait_for_exit_or_terminate_with<T, TryWait, Terminate>(
    child: &mut T,
    timeout: Duration,
    mut try_wait: TryWait,
    mut terminate: Terminate,
) -> io::Result<ExitStatus>
where
    TryWait: FnMut(&mut T) -> io::Result<Option<ExitStatus>>,
    Terminate: FnMut(&mut T) -> io::Result<()>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = try_wait(child)? {
            return Ok(status);
        }

        let now = Instant::now();
        if now >= deadline {
            let message = match terminate(child) {
                Ok(()) => format!("process did not exit within {} ms", timeout.as_millis()),
                Err(error) => format!(
                    "process did not exit within {} ms; termination failed: {error}",
                    timeout.as_millis()
                ),
            };
            return Err(io::Error::new(io::ErrorKind::TimedOut, message));
        }

        std::thread::sleep(WAIT_POLL_INTERVAL.min(deadline.duration_since(now)));
    }
}

/// Ends `pid` together with this process: a kill-on-close job on Windows, whose
/// own children break away so apps a daemon launches outlive it. Unix daemons
/// watch their host themselves, so this is a no-op there.
pub fn bind_to_host_lifetime(pid: u32) -> io::Result<()> {
    platform::bind_to_host_lifetime(pid)
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
    fn guardian_request_requires_the_exact_first_argument() {
        assert!(is_process_tree_guardian_argument(Some(
            std::ffi::OsStr::new(PROCESS_TREE_GUARDIAN_COMMAND)
        )));
        assert!(!is_process_tree_guardian_argument(None));
        assert!(!is_process_tree_guardian_argument(Some(
            std::ffi::OsStr::new("--process-tree-guardian")
        )));
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

    #[cfg(target_os = "linux")]
    #[test]
    fn process_identity_matches_the_persisted_linux_legacy_generation() {
        let pid = std::process::id();
        let identity = process_identity(pid).unwrap();
        let fields = identity.split(':').collect::<Vec<_>>();
        let legacy = format!("{}:{}:{}", fields[0], fields[1], fields[4]);
        let wrong_boot = format!("linux:wrong-boot:{}", fields[4]);
        let wrong_start_ticks = fields[4].parse::<u64>().unwrap() + 1;
        let wrong_start = format!("linux:{}:{wrong_start_ticks}", fields[1]);

        assert!(process_identity_matches(pid, &legacy));
        assert!(!process_identity_matches(pid, &wrong_boot));
        assert!(!process_identity_matches(pid, &wrong_start));
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

    #[cfg(any(unix, windows))]
    #[test]
    fn cancellation_signal_child_helper() {
        let Some(root) = std::env::var_os("QOL_PROCESS_CANCELLATION_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let token = CancellationToken::install().unwrap();
        std::fs::write(root.join("ready"), "ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !token.is_cancelled() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(token.is_cancelled());
        std::fs::write(root.join("cancelled"), "cancelled").unwrap();
        if std::env::var_os("QOL_PROCESS_EXPECT_ESCALATION").is_none() {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !token.escalation_requested() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(token.escalation_requested());
        std::fs::write(root.join("escalated"), "escalated").unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn installed_handler_turns_sigterm_into_observable_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::cancellation_signal_child_helper"])
            .env("QOL_PROCESS_CANCELLATION_TEST_ROOT", temp.path())
            .spawn()
            .unwrap();
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !temp.path().join("ready").exists() && Instant::now() < ready_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(temp.path().join("ready").exists());
        signal_term_pid(child.id()).unwrap();
        let exit_deadline = Instant::now() + Duration::from_secs(5);
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

    #[cfg(any(unix, windows))]
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

    #[cfg(any(unix, windows))]
    fn wait_for_path(path: &std::path::Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {}", path.display());
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::zombie_processes)]
    fn stop_request_child_helper() {
        let Some(root) = std::env::var_os("QOL_PROCESS_STOP_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        if let Some(mode) = std::env::var_os("QOL_PROCESS_STOP_TEST_DESCENDANT") {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "tests::stop_request_child_helper"])
                .env("QOL_PROCESS_STOP_TEST_ROOT", root.join("descendant"))
                .env_remove("QOL_PROCESS_STOP_TEST_DESCENDANT");
            if mode == "ignore" {
                command.env("QOL_PROCESS_STOP_TEST_IGNORE", "1");
            }
            let descendant = command.spawn().unwrap();
            wait_for_path(&root.join("descendant").join("ready"));
            std::fs::write(root.join("descendant-pid"), descendant.id().to_string()).unwrap();
        }
        if std::env::var_os("QOL_PROCESS_STOP_TEST_IGNORE").is_some() {
            CancellationToken::install().unwrap();
            std::fs::write(root.join("ready"), "ready").unwrap();
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        std::fs::write(root.join("ready"), "ready").unwrap();
        wait_for_stop_request().unwrap();
        std::fs::write(root.join("stopped"), "stopped").unwrap();
    }

    #[cfg(windows)]
    fn spawn_stop_helper(root: &std::path::Path, mode: &[(&str, &str)]) -> std::process::Child {
        std::fs::create_dir_all(root.join("descendant")).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "tests::stop_request_child_helper"])
            .env("QOL_PROCESS_STOP_TEST_ROOT", root);
        for (key, value) in mode {
            command.env(key, value);
        }
        let child = command.spawn().unwrap();
        wait_for_path(&root.join("ready"));
        child
    }

    #[cfg(windows)]
    fn descendant_pid(root: &std::path::Path) -> u32 {
        wait_for_path(&root.join("descendant-pid"));
        std::fs::read_to_string(root.join("descendant-pid"))
            .unwrap()
            .parse()
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn graceful_stop_cases() {
        struct Case {
            name: &'static str,
            mode: &'static [(&'static str, &'static str)],
            stop: fn(&mut std::process::Child, Duration),
            grace: Duration,
            exits_cleanly: bool,
            waits_for_grace: bool,
        }
        let cases = [
            Case {
                name: "terminate_pid lets a listening child exit cleanly",
                mode: &[],
                stop: |child, grace| terminate_pid(child.id(), grace),
                grace: Duration::from_secs(10),
                exits_cleanly: true,
                waits_for_grace: false,
            },
            Case {
                name: "terminate_pid kills a child that ignores the request after the grace",
                mode: &[("QOL_PROCESS_STOP_TEST_IGNORE", "1")],
                stop: |child, grace| terminate_pid(child.id(), grace),
                grace: Duration::from_millis(700),
                exits_cleanly: false,
                waits_for_grace: true,
            },
            Case {
                name: "terminate_group lets a listening root exit cleanly",
                mode: &[],
                stop: |child, grace| terminate_group(child.id(), grace),
                grace: Duration::from_secs(10),
                exits_cleanly: true,
                waits_for_grace: false,
            },
            Case {
                name: "terminate_group kills a root that ignores the request after the grace",
                mode: &[("QOL_PROCESS_STOP_TEST_IGNORE", "1")],
                stop: |child, grace| terminate_group(child.id(), grace),
                grace: Duration::from_millis(700),
                exits_cleanly: false,
                waits_for_grace: true,
            },
            Case {
                name: "signal_term_pid reaches a listening child",
                mode: &[],
                stop: |child, _| signal_term_pid(child.id()).unwrap(),
                grace: Duration::ZERO,
                exits_cleanly: true,
                waits_for_grace: false,
            },
        ];
        for case in cases {
            let temp = tempfile::tempdir().unwrap();
            let mut child = spawn_stop_helper(temp.path(), case.mode);
            let started = Instant::now();
            (case.stop)(&mut child, case.grace);
            let status = child.wait().unwrap();
            let elapsed = started.elapsed();
            assert_eq!(status.success(), case.exits_cleanly, "{}", case.name);
            assert_eq!(
                temp.path().join("stopped").exists(),
                case.exits_cleanly,
                "{}",
                case.name
            );
            if case.waits_for_grace {
                assert!(
                    elapsed + Duration::from_millis(100) >= case.grace,
                    "{}: {elapsed:?}",
                    case.name
                );
            } else {
                assert!(
                    elapsed < Duration::from_secs(5),
                    "{}: {elapsed:?}",
                    case.name
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn terminate_owned_waits_for_a_listening_child_and_kills_one_that_ignores_it() {
        let temp = tempfile::tempdir().unwrap();
        let mut child = spawn_stop_helper(temp.path(), &[]);
        terminate_owned(&mut child, Duration::from_secs(10)).unwrap();
        assert!(temp.path().join("stopped").exists());

        let temp = tempfile::tempdir().unwrap();
        let mut child = spawn_stop_helper(temp.path(), &[("QOL_PROCESS_STOP_TEST_IGNORE", "1")]);
        let started = Instant::now();
        terminate_owned(&mut child, Duration::from_millis(700)).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(700));
        assert!(!is_pid_alive(child.id()));
    }

    #[cfg(windows)]
    #[test]
    fn signal_term_pid_without_a_listener_terminates_at_once() {
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 31 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let started = Instant::now();
        signal_term_pid(child.id()).unwrap();
        assert!(!child.wait().unwrap().success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(windows)]
    #[test]
    fn terminate_pid_without_a_listener_waits_the_grace_then_kills() {
        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 31 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        let grace = Duration::from_millis(700);
        let started = Instant::now();
        terminate_pid(child.id(), grace);
        assert!(!child.wait().unwrap().success());
        let elapsed = started.elapsed();
        assert!(elapsed + Duration::from_millis(100) >= grace, "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[cfg(windows)]
    #[test]
    fn group_stops_reach_verified_descendants() {
        struct Case {
            name: &'static str,
            descendant: &'static str,
            stop: fn(u32),
            root_exits_cleanly: bool,
            descendant_exits_cleanly: bool,
        }
        let cases = [
            Case {
                name: "terminate_group kills an ignoring descendant after the grace",
                descendant: "ignore",
                stop: |pid| terminate_group(pid, Duration::from_millis(700)),
                root_exits_cleanly: true,
                descendant_exits_cleanly: false,
            },
            Case {
                name: "terminate_group lets a listening descendant exit cleanly",
                descendant: "listen",
                stop: |pid| terminate_group(pid, Duration::from_secs(10)),
                root_exits_cleanly: true,
                descendant_exits_cleanly: true,
            },
            Case {
                name: "kill_group kills the root and its descendant",
                descendant: "ignore",
                stop: |pid| kill_group(pid).unwrap(),
                root_exits_cleanly: false,
                descendant_exits_cleanly: false,
            },
            Case {
                name: "signal_term_group reaches a listening descendant",
                descendant: "listen",
                stop: |pid| signal_term_group(pid).unwrap(),
                root_exits_cleanly: true,
                descendant_exits_cleanly: true,
            },
        ];
        for case in cases {
            let temp = tempfile::tempdir().unwrap();
            let mut root = spawn_stop_helper(
                temp.path(),
                &[("QOL_PROCESS_STOP_TEST_DESCENDANT", case.descendant)],
            );
            let descendant = descendant_pid(temp.path());
            let identity = process_identity(descendant).unwrap();
            (case.stop)(root.id());
            assert_eq!(
                root.wait().unwrap().success(),
                case.root_exits_cleanly,
                "{}",
                case.name
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while process_identity_matches(descendant, &identity) && is_pid_alive(descendant) {
                assert!(
                    Instant::now() < deadline,
                    "{}: descendant survived",
                    case.name
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                temp.path().join("descendant").join("stopped").exists(),
                case.descendant_exits_cleanly,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn zero_pid_is_never_a_process_target() {
        assert!(!is_pid_alive(0));
        assert!(!is_group_alive(0));
        assert!(!is_pid_zombie(0));
        assert!(signal_term_pid(0).is_err());
        assert!(signal_term_group(0).is_err());
        assert!(kill_pid(0).is_err());
        assert!(kill_group(0).is_err());
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

    #[cfg(unix)]
    #[test]
    #[allow(clippy::zombie_processes)]
    fn reload_group_stops_the_leader_on_sighup() {
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

        reload_group(pid, Duration::from_secs(1));

        assert!(!is_group_alive(pid));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_unreaped_process_is_a_zombie_until_waited() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(5);
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

        let deadline = Instant::now() + Duration::from_secs(5);
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

    #[test]
    fn bounded_wait_terminates_on_timeout() {
        let mut terminated = false;
        let error = wait_for_exit_or_terminate_with(
            &mut terminated,
            Duration::ZERO,
            |_| Ok(None),
            |terminated| {
                *terminated = true;
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(terminated);
    }
}
