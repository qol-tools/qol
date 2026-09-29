use anyhow::{bail, Context, Result};
use qol_process::CancellationToken;
use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATION_GRACE: Duration = Duration::from_secs(2);
const CAPTURE_STDERR_LIMIT: usize = 64 * 1024;
const CAPTURE_BUFFER: usize = 8192;

mod platform;
pub(super) use platform::exit_signal;

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug)]
pub(super) enum ShutdownReason {
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
    Fallback { acquisition_failure: String },
}

#[derive(Debug)]
pub(super) struct CommandResult {
    pub(super) leader: Option<ExitStatus>,
    pub(super) result: Result<()>,
    pub(super) lifecycle: Lifecycle,
}

#[derive(Debug)]
pub(super) struct Lifecycle {
    pub(super) requested: Containment,
    pub(super) backend: &'static str,
    pub(super) acquisition_failure: Option<String>,
    pub(super) membership_observation_supported: Option<bool>,
    pub(super) first_post_leader_liveness: Option<Result<bool, String>>,
    pub(super) shutdown_reason: Option<ShutdownReason>,
    pub(super) observation: Option<qol_process::ProcessTreeObservation>,
    pub(super) stop: Option<Result<(), String>>,
    pub(super) force: Option<Result<(), String>>,
    pub(super) seal: Option<Result<(), String>>,
    pub(super) recovery_force: Option<Result<(), String>>,
    pub(super) recovery_kill: Option<Result<(), String>>,
    pub(super) recovery_reap: Option<Result<(), String>>,
    pub(super) recovery_liveness: Option<Result<bool, String>>,
    pub(super) recovery_seal: Option<Result<(), String>>,
}

impl CommandResult {
    fn new(containment: Containment) -> Self {
        Self {
            leader: None,
            result: Ok(()),
            lifecycle: Lifecycle {
                requested: containment,
                backend: "not_acquired",
                acquisition_failure: None,
                membership_observation_supported: None,
                first_post_leader_liveness: None,
                shutdown_reason: None,
                observation: None,
                stop: None,
                force: None,
                seal: None,
                recovery_force: None,
                recovery_kill: None,
                recovery_reap: None,
                recovery_liveness: None,
                recovery_seal: None,
            },
        }
    }

    fn acquire(&mut self) -> Result<CommandOwner> {
        let owner = CommandOwner::acquire(self.lifecycle.requested).inspect_err(|error| {
            self.lifecycle.acquisition_failure = Some(error_category(error));
        })?;
        match &owner {
            CommandOwner::Tree(tree) => {
                self.lifecycle.backend = tree.containment_backend();
                self.lifecycle.membership_observation_supported =
                    Some(tree.membership_observation_supported());
            }
            CommandOwner::Fallback {
                acquisition_failure,
            } => {
                self.lifecycle.backend = platform::FALLBACK_BACKEND;
                self.lifecycle.membership_observation_supported = Some(false);
                self.lifecycle.acquisition_failure = Some(acquisition_failure.clone());
            }
        }
        Ok(owner)
    }
}

fn error_category(error: &anyhow::Error) -> String {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<io::Error>())
        .map(|error| format!("{:?}", error.kind()))
        .unwrap_or_else(|| "unclassified".into())
}

fn record_operation<T>(result: &Result<T>) -> Result<(), String> {
    result.as_ref().map(|_| ()).map_err(error_category)
}

pub(super) fn run(
    command: &mut Command,
    cancellation: &impl CancellationState,
    containment: Containment,
    verbose: bool,
) -> CommandResult {
    let mut output = CommandResult::new(containment);
    if cancellation.is_cancelled() {
        output.lifecycle.shutdown_reason = Some(ShutdownReason::Cancelled);
        output.result = Err(anyhow::anyhow!("check cancelled before command start"));
        return output;
    }
    let owner = match output.acquire() {
        Ok(owner) => owner,
        Err(error) => {
            output.result = Err(error);
            return output;
        }
    };
    output.result = crate::progress::run_status_with(
        command,
        verbose,
        |command| owner.spawn(command),
        |child| {
            let outcome = wait_for_exit(child, &owner, cancellation, &mut output);
            recover_wait_failure(child, &owner, outcome, &mut output)
        },
    );
    output
}

pub(super) struct CapturedOutput {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) command: CommandResult,
}

pub(super) fn run_captured(
    command: &mut Command,
    input: &[u8],
    cancellation: &impl CancellationState,
    containment: Containment,
) -> CapturedOutput {
    let mut output = CapturedOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        command: CommandResult::new(containment),
    };
    output.command.result = capture(command, input, cancellation, &mut output);
    output
}

fn capture(
    command: &mut Command,
    input: &[u8],
    cancellation: &impl CancellationState,
    output: &mut CapturedOutput,
) -> Result<()> {
    if cancellation.is_cancelled() {
        output.command.lifecycle.shutdown_reason = Some(ShutdownReason::Cancelled);
        bail!("check cancelled before command start");
    }
    let owner = output.command.acquire()?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = owner.spawn(command)?;
    let stdout_reader = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(move || read_capture(pipe, None)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || read_capture(pipe, Some(CAPTURE_STDERR_LIMIT))));
    let payload = input.to_vec();
    let stdin_writer = child
        .stdin
        .take()
        .map(|mut pipe| thread::spawn(move || pipe.write_all(&payload)));
    let outcome = wait_for_exit(&mut child, &owner, cancellation, &mut output.command);
    let outcome = recover_wait_failure(&mut child, &owner, outcome, &mut output.command);
    output.stdout = join_capture(stdout_reader);
    output.stderr = join_capture(stderr_reader);
    if let Some(writer) = stdin_writer {
        let _ = writer.join();
    }
    let status = outcome?;
    if !status.success() {
        bail!("command failed with {status}");
    }
    Ok(())
}

fn read_capture<R: Read>(mut reader: R, limit: Option<usize>) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut buffer = vec![0_u8; CAPTURE_BUFFER];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        match limit {
            Some(limit) if collected.len() >= limit => {}
            Some(limit) => {
                let remaining = limit - collected.len();
                collected.extend_from_slice(&buffer[..count.min(remaining)]);
            }
            None => collected.extend_from_slice(&buffer[..count]),
        }
    }
    collected
}

fn join_capture(handle: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

impl CommandOwner {
    fn observe_residual(&self) -> qol_process::ProcessTreeObservation {
        match self {
            Self::Tree(tree) => tree.observe_residual(),
            Self::Fallback { .. } => qol_process::ProcessTreeObservation::unsupported(),
        }
    }

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
            Err(error) => Ok(Self::Fallback {
                acquisition_failure: error_category(&error),
            }),
        }
    }

    fn spawn(&self, command: &mut Command) -> Result<Child> {
        match self {
            Self::Tree(tree) => spawn_owned_tree(tree, command),
            Self::Fallback { .. } => {
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
            Self::Fallback { .. } => Ok(platform::fallback_alive(pid)),
        }
    }

    fn request_stop(&self, pid: u32) -> Result<()> {
        let result = match self {
            Self::Tree(tree) => tree.request_stop(),
            Self::Fallback { .. } => platform::fallback_request_stop(pid),
        };
        tolerate_stopped(result, self, pid).context("failed to terminate command tree")
    }

    fn force_stop(&self, pid: u32) -> Result<()> {
        let result = match self {
            Self::Tree(tree) => tree.force_stop_and_wait(TERMINATION_GRACE).map(drop),
            Self::Fallback { .. } => platform::fallback_force_stop(pid),
        };
        tolerate_stopped(result, self, pid).context("failed to kill command tree")
    }

    fn seal(&self, pid: u32) -> Result<()> {
        match self {
            Self::Tree(tree) => tree
                .force_stop_and_wait(TERMINATION_GRACE)
                .map(drop)
                .context("failed to seal command tree"),
            Self::Fallback { .. } if platform::fallback_alive(pid) => {
                bail!("command fallback still has live processes")
            }
            Self::Fallback { .. } => Ok(()),
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
    output: &mut CommandResult,
) -> Result<ExitStatus> {
    wait_with_observation(
        child,
        owner,
        cancellation,
        output,
        CommandOwner::observe_residual,
    )
}

fn wait_with_observation(
    child: &mut Child,
    owner: &CommandOwner,
    cancellation: &impl CancellationState,
    output: &mut CommandResult,
    observe: impl FnOnce(&CommandOwner) -> qol_process::ProcessTreeObservation,
) -> Result<ExitStatus> {
    let pid = child.id();
    let mut shutdown = None;
    let mut observe = Some(observe);
    loop {
        if cancellation.is_cancelled() && shutdown.is_none() {
            shutdown = Some(begin_shutdown(
                owner,
                pid,
                ShutdownReason::Cancelled,
                output,
                &mut observe,
            )?);
        }
        if output.leader.is_none() {
            output.leader = child.try_wait().context("failed waiting for command")?;
        }
        if let Some(status) = output.leader {
            if shutdown.is_none() && !inspect_liveness(owner, pid, output)? {
                let sealed = owner.seal(pid);
                output.lifecycle.seal = Some(record_operation(&sealed));
                sealed?;
                return Ok(status);
            }
            if shutdown.is_none() {
                shutdown = Some(begin_shutdown(
                    owner,
                    pid,
                    ShutdownReason::ResidualGroup,
                    output,
                    &mut observe,
                )?);
            }
        }
        if let Some(state) = shutdown.as_mut() {
            if !inspect_liveness(owner, pid, output)? {
                if output.leader.is_none() {
                    output.leader = Some(child.wait().context("failed to reap command")?);
                }
                let sealed = owner.seal(pid);
                output.lifecycle.seal = Some(record_operation(&sealed));
                sealed?;
                return finish_shutdown(state.reason, output.leader);
            }
            advance_shutdown(owner, pid, state, cancellation, output)?;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn inspect_liveness(owner: &CommandOwner, pid: u32, output: &mut CommandResult) -> Result<bool> {
    let alive = owner.is_alive(pid);
    if output.leader.is_some() && output.lifecycle.first_post_leader_liveness.is_none() {
        output.lifecycle.first_post_leader_liveness =
            Some(alive.as_ref().copied().map_err(error_category));
    }
    alive
}

fn begin_shutdown(
    owner: &CommandOwner,
    pid: u32,
    reason: ShutdownReason,
    output: &mut CommandResult,
    observe: &mut Option<impl FnOnce(&CommandOwner) -> qol_process::ProcessTreeObservation>,
) -> Result<Shutdown> {
    let deadline = Instant::now() + TERMINATION_GRACE;
    output.lifecycle.shutdown_reason = Some(reason);
    if matches!(reason, ShutdownReason::ResidualGroup) {
        if let Some(observe) = observe.take() {
            output.lifecycle.observation = Some(observe(owner));
        }
    }
    let stopped = owner.request_stop(pid);
    output.lifecycle.stop = Some(record_operation(&stopped));
    stopped?;
    Ok(Shutdown {
        reason,
        deadline,
        forced: false,
    })
}

fn advance_shutdown(
    owner: &CommandOwner,
    pid: u32,
    shutdown: &mut Shutdown,
    cancellation: &impl CancellationState,
    output: &mut CommandResult,
) -> Result<()> {
    if !shutdown.forced && should_escalate(shutdown.deadline, cancellation) {
        let forced = owner.force_stop(pid);
        output.lifecycle.force = Some(record_operation(&forced));
        forced?;
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
    output: &mut CommandResult,
) -> Result<ExitStatus> {
    let Err(error) = outcome else {
        return outcome;
    };
    let error = match output.lifecycle.shutdown_reason {
        Some(reason) => {
            let message = match reason {
                ShutdownReason::Cancelled => "check cancelled",
                ShutdownReason::ResidualGroup => {
                    "command exited while descendants remained in its owned process tree"
                }
            };
            error.context(message)
        }
        None => error,
    };
    let cleanup = force_stop(child, owner, output);
    match cleanup {
        Ok(()) => Err(error),
        Err(cleanup) => Err(anyhow::anyhow!("{error:#}\n{cleanup:#}")),
    }
}

fn force_stop(child: &mut Child, owner: &CommandOwner, output: &mut CommandResult) -> Result<()> {
    let pid = child.id();
    let tree = owner.force_stop(pid);
    output.lifecycle.recovery_force = Some(record_operation(&tree));
    let process = child
        .kill()
        .or_else(ignore_exited_process)
        .map_err(anyhow::Error::from);
    output.lifecycle.recovery_kill = Some(record_operation(&process));
    let waited = child.wait().context("failed to reap command");
    output.lifecycle.recovery_reap = Some(record_operation(&waited));
    if let Ok(status) = waited.as_ref() {
        output.leader.get_or_insert(*status);
    }
    let settled = wait_until_stopped(owner, pid);
    output.lifecycle.recovery_liveness = Some(settled.as_ref().copied().map_err(error_category));
    let sealed = owner.seal(pid);
    output.lifecycle.recovery_seal = Some(record_operation(&sealed));
    let tree = if matches!(settled, Ok(false)) {
        Ok(())
    } else {
        tree
    };
    let errors = [tree, process, waited.map(drop), settled.map(drop), sealed]
        .into_iter()
        .filter_map(Result::err)
        .map(|error| format!("{error:#}"))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    Ok(())
}

fn wait_until_stopped(owner: &CommandOwner, pid: u32) -> Result<bool> {
    let deadline = Instant::now() + TERMINATION_GRACE;
    loop {
        let alive = owner.is_alive(pid)?;
        if !alive || Instant::now() >= deadline {
            return Ok(alive);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn ignore_exited_process(error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::InvalidInput {
        return Ok(());
    }
    Err(error)
}

#[cfg(all(test, unix))]
mod tests;
