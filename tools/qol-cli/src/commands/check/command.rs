use anyhow::{bail, Context, Result};
use qol_process::CancellationToken;
use std::io;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(super) enum Containment {
    Preferred,
    Required,
}

pub(super) trait CancellationState {
    fn is_cancelled(&self) -> bool;
    fn escalation_requested(&self) -> bool;
}

impl CancellationState for CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }

    fn escalation_requested(&self) -> bool {
        self.escalation_requested()
    }
}

#[derive(Clone, Copy)]
enum ShutdownReason {
    Cancelled,
    ResidualGroup,
}

struct Shutdown {
    reason: ShutdownReason,
    deadline: Instant,
    forced: bool,
}

enum CommandOwner {
    Tree(qol_process::ProcessTreeGuard),
    Fallback,
}

pub(super) fn run(
    command: &mut Command,
    cancellation: &impl CancellationState,
    containment: Containment,
    verbose: bool,
) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("check cancelled before command start");
    }
    let owner = CommandOwner::acquire(containment)?;
    crate::progress::run_status_with(
        command,
        verbose,
        |command| owner.spawn(command),
        |child| {
            let outcome = wait_for_exit(child, &owner, cancellation);
            recover_wait_failure(child, &owner, outcome)
        },
    )
}

impl CommandOwner {
    fn acquire(containment: Containment) -> Result<Self> {
        Self::from_attempt(containment, crate::process_guardian::own_process_tree())
    }

    fn from_attempt(
        containment: Containment,
        attempt: Result<qol_process::ProcessTreeGuard>,
    ) -> Result<Self> {
        match attempt {
            Ok(tree) => Ok(Self::Tree(tree)),
            Err(error) if matches!(containment, Containment::Required) => {
                Err(error).context("verified process-tree containment is required")
            }
            Err(_) => Ok(Self::Fallback),
        }
    }

    fn spawn(&self, command: &mut Command) -> Result<Child> {
        match self {
            Self::Tree(tree) => spawn_owned_tree(tree, command),
            Self::Fallback => {
                qol_process::isolate_owned_command(command)
                    .context("failed to isolate command fallback")?;
                command.spawn().context("failed to spawn command")
            }
        }
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        match self {
            Self::Tree(tree) => tree
                .tree_has_exited()
                .map(|exited| !exited)
                .context("failed to inspect command tree"),
            Self::Fallback => Ok(fallback_alive(pid)),
        }
    }

    fn request_stop(&self, pid: u32) -> Result<()> {
        let result = match self {
            Self::Tree(tree) => tree.request_stop(),
            Self::Fallback => fallback_request_stop(pid),
        };
        tolerate_stopped(result, self, pid).context("failed to terminate command tree")
    }

    fn force_stop(&self, pid: u32) -> Result<()> {
        let result = match self {
            Self::Tree(tree) => tree.force_stop_and_wait(TERMINATION_GRACE).map(drop),
            Self::Fallback => fallback_force_stop(pid),
        };
        tolerate_stopped(result, self, pid).context("failed to kill command tree")
    }

    fn seal(&self, pid: u32) -> Result<()> {
        match self {
            Self::Tree(tree) => tree
                .force_stop_and_wait(TERMINATION_GRACE)
                .map(drop)
                .context("failed to seal command tree"),
            Self::Fallback if fallback_alive(pid) => {
                bail!("command fallback still has live processes")
            }
            Self::Fallback => Ok(()),
        }
    }
}

fn spawn_owned_tree(tree: &qol_process::ProcessTreeGuard, command: &mut Command) -> Result<Child> {
    qol_process::isolate_owned_session(command).context("failed to isolate command session")?;
    let command = std::mem::replace(command, Command::new("__qol_consumed_command"));
    let prepared = tree
        .prepare_command(command)
        .context("failed to prepare command tree")?;
    prepared.spawn().map_err(|error| {
        let cleanup = error.cleanup();
        anyhow::Error::new(error).context(format!(
            "failed to spawn command tree; cleanup state: {cleanup:?}"
        ))
    })
}

fn wait_for_exit(
    child: &mut Child,
    owner: &CommandOwner,
    cancellation: &impl CancellationState,
) -> Result<ExitStatus> {
    let pid = child.id();
    let mut exit = None;
    let mut shutdown = None;
    loop {
        if cancellation.is_cancelled() && shutdown.is_none() {
            shutdown = Some(begin_shutdown(owner, pid, ShutdownReason::Cancelled)?);
        }
        if exit.is_none() {
            exit = child.try_wait().context("failed waiting for command")?;
        }
        if let Some(status) = exit {
            if shutdown.is_none() && !owner.is_alive(pid)? {
                owner.seal(pid)?;
                return Ok(status);
            }
            if shutdown.is_none() {
                shutdown = Some(begin_shutdown(owner, pid, ShutdownReason::ResidualGroup)?);
            }
        }
        if let Some(state) = shutdown.as_mut() {
            if !owner.is_alive(pid)? {
                if exit.is_none() {
                    exit = Some(child.wait().context("failed to reap command")?);
                }
                owner.seal(pid)?;
                return finish_shutdown(state.reason, exit);
            }
            advance_shutdown(owner, pid, state, cancellation)?;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn begin_shutdown(owner: &CommandOwner, pid: u32, reason: ShutdownReason) -> Result<Shutdown> {
    owner.request_stop(pid)?;
    Ok(Shutdown {
        reason,
        deadline: Instant::now() + TERMINATION_GRACE,
        forced: false,
    })
}

fn advance_shutdown(
    owner: &CommandOwner,
    pid: u32,
    shutdown: &mut Shutdown,
    cancellation: &impl CancellationState,
) -> Result<()> {
    if !shutdown.forced && should_escalate(shutdown.deadline, cancellation) {
        owner.force_stop(pid)?;
        shutdown.forced = true;
    }
    Ok(())
}

fn should_escalate(deadline: Instant, cancellation: &impl CancellationState) -> bool {
    cancellation.escalation_requested() || Instant::now() >= deadline
}

fn tolerate_stopped(result: io::Result<()>, owner: &CommandOwner, pid: u32) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(_) if owner.is_alive(pid).is_ok_and(|alive| !alive) => Ok(()),
        Err(error) => Err(error),
    }
}

fn finish_shutdown(reason: ShutdownReason, exit: Option<ExitStatus>) -> Result<ExitStatus> {
    let _ = exit.context("command group exited before its leader was reaped")?;
    match reason {
        ShutdownReason::Cancelled => bail!("check cancelled"),
        ShutdownReason::ResidualGroup => {
            bail!("command exited while descendants remained in its owned process tree")
        }
    }
}

fn recover_wait_failure(
    child: &mut Child,
    owner: &CommandOwner,
    outcome: Result<ExitStatus>,
) -> Result<ExitStatus> {
    let Err(error) = outcome else {
        return outcome;
    };
    let cleanup = force_stop(child, owner);
    match cleanup {
        Ok(()) => Err(error),
        Err(cleanup) => Err(anyhow::anyhow!("{error:#}\n{cleanup:#}")),
    }
}

fn force_stop(child: &mut Child, owner: &CommandOwner) -> Result<()> {
    let pid = child.id();
    let tree = owner.force_stop(pid);
    let process = child.kill();
    let waited = child.wait().map(|_| ());
    process.or_else(ignore_exited_process)?;
    waited.context("failed to reap command")?;
    tree?;
    let deadline = Instant::now() + TERMINATION_GRACE;
    while owner.is_alive(pid)? && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
    }
    owner.seal(pid)
}

#[cfg(unix)]
fn fallback_alive(pid: u32) -> bool {
    qol_process::is_group_alive(pid)
}

#[cfg(windows)]
fn fallback_alive(pid: u32) -> bool {
    qol_process::is_pid_alive(pid)
}

#[cfg(unix)]
fn fallback_request_stop(pid: u32) -> io::Result<()> {
    qol_process::signal_term_group(pid)
}

#[cfg(windows)]
fn fallback_request_stop(pid: u32) -> io::Result<()> {
    qol_process::signal_term_pid(pid)
}

#[cfg(unix)]
fn fallback_force_stop(pid: u32) -> io::Result<()> {
    qol_process::kill_group(pid)
}

#[cfg(windows)]
fn fallback_force_stop(pid: u32) -> io::Result<()> {
    qol_process::kill_pid(pid)
}

fn ignore_exited_process(error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::InvalidInput {
        return Ok(());
    }
    Err(error)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct FakeCancellation {
        cancelled: Arc<AtomicBool>,
        escalated: Arc<AtomicBool>,
    }

    impl CancellationState for FakeCancellation {
        fn is_cancelled(&self) -> bool {
            self.cancelled.load(Ordering::Acquire)
        }

        fn escalation_requested(&self) -> bool {
            self.escalated.load(Ordering::Acquire)
        }
    }

    #[test]
    fn required_containment_rejects_unavailable_ownership() {
        let error = CommandOwner::from_attempt(
            Containment::Required,
            Err(anyhow::anyhow!("unsupported containment")),
        )
        .err()
        .unwrap();

        assert!(error
            .to_string()
            .contains("verified process-tree containment is required"));
    }

    #[test]
    fn preferred_containment_retains_the_worktree_fallback() {
        assert!(matches!(
            CommandOwner::from_attempt(
                Containment::Preferred,
                Err(anyhow::anyhow!("unsupported containment")),
            )
            .unwrap(),
            CommandOwner::Fallback
        ));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_escalates_and_reaps_the_owned_group() {
        let root = tempfile::tempdir().unwrap();
        let leader = root.path().join("leader");
        let descendant = root.path().join("descendant");
        let cancellation = FakeCancellation::default();
        let trigger = cancellation.clone();
        let leader_for_trigger = leader.clone();
        let trigger_thread = thread::spawn(move || {
            wait_for_path(&leader_for_trigger);
            trigger.cancelled.store(true, Ordering::Release);
            thread::sleep(Duration::from_millis(50));
            trigger.escalated.store(true, Ordering::Release);
        });
        let mut command = stubborn_group_command(&leader, &descendant);

        let error = run(&mut command, &cancellation, Containment::Preferred, false).unwrap_err();

        trigger_thread.join().unwrap();
        assert!(
            error.to_string().contains("cancelled"),
            "unexpected error: {error:#}"
        );
        let leader = read_pid(&leader);
        let descendant = read_pid(&descendant);
        assert!(!qol_process::is_group_alive(leader));
        assert!(!qol_process::is_pid_alive(descendant));
    }

    #[cfg(unix)]
    #[test]
    fn signal_cancellation_runner_helper() {
        let Some(root) = std::env::var_os("QOL_CHECK_CANCELLATION_TEST_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let token = CancellationToken::install().unwrap();
        let mut command = stubborn_group_command(&root.join("leader"), &root.join("descendant"));
        let result = run(&mut command, &token, Containment::Preferred, false);
        fs::write(root.join("terminal"), format!("{result:?}")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sigterm_reaches_the_runner_and_allows_terminal_finalization() {
        let root = tempfile::tempdir().unwrap();
        let mut helper = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::check::command::tests::signal_cancellation_runner_helper",
            ])
            .env("QOL_CHECK_CANCELLATION_TEST_ROOT", root.path())
            .spawn()
            .unwrap();
        wait_for_path(&root.path().join("leader"));
        qol_process::signal_term_pid(helper.id()).unwrap();
        thread::sleep(Duration::from_millis(50));
        qol_process::signal_term_pid(helper.id()).unwrap();
        let status = helper.wait().unwrap();

        assert!(status.success());
        let terminal = fs::read_to_string(root.path().join("terminal")).unwrap();
        assert!(
            terminal.contains("cancelled"),
            "unexpected terminal result: {terminal}"
        );
        assert!(!qol_process::is_group_alive(read_pid(
            &root.path().join("leader")
        )));
        assert!(!qol_process::is_pid_alive(read_pid(
            &root.path().join("descendant")
        )));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn escaped_session_descendant_helper() {
        let Some(marker) = std::env::var_os("QOL_CHECK_ESCAPED_SESSION_MARKER") else {
            return;
        };
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "trap '' TERM; echo $$ > \"$1\"; exec sleep 30",
                "qol-check-escaped",
            ])
            .arg(marker);
        qol_process::isolate_owned_session(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let _ = child.wait();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn required_containment_reaps_a_descendant_that_escapes_its_session() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("escaped");
        let cancellation = FakeCancellation::default();
        let trigger = cancellation.clone();
        let marker_for_trigger = marker.clone();
        let trigger_thread = thread::spawn(move || {
            wait_for_path(&marker_for_trigger);
            trigger.cancelled.store(true, Ordering::Release);
            trigger.escalated.store(true, Ordering::Release);
        });
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "commands::check::command::tests::escaped_session_descendant_helper",
            ])
            .env("QOL_CHECK_ESCAPED_SESSION_MARKER", &marker);

        let error = run(&mut command, &cancellation, Containment::Required, false).unwrap_err();

        trigger_thread.join().unwrap();
        assert!(
            error.to_string().contains("cancelled"),
            "unexpected error: {error:#}"
        );
        assert!(!qol_process::is_pid_alive(read_pid(&marker)));
    }

    #[cfg(unix)]
    fn stubborn_group_command(leader: &Path, descendant: &Path) -> Command {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "trap '' TERM; echo $$ > \"$1\"; sleep 30 & echo $! > \"$2\"; wait",
                "qol-check-test",
            ])
            .arg(leader)
            .arg(descendant);
        command
    }

    #[cfg(unix)]
    fn wait_for_path(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !path.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(path.exists(), "timed out waiting for {}", path.display());
    }

    #[cfg(unix)]
    fn read_pid(path: &Path) -> u32 {
        fs::read_to_string(path).unwrap().trim().parse().unwrap()
    }
}
