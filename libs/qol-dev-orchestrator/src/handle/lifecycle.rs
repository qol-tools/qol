use std::collections::VecDeque;
use std::io::{self, BufWriter, Write};
use std::process::{Child, ExitStatus};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

const CLEANUP_RETRY_INTERVAL: Duration = Duration::from_millis(100);

pub(super) struct TerminationAttempt {
    pub(super) proof: Result<qol_process::TerminatedProcessTree>,
    pub(super) root_stop: Result<()>,
}

pub(super) type TerminationFn = Box<
    dyn FnOnce(&qol_process::ProcessTreeGuard, Duration) -> TerminationAttempt + Send + 'static,
>;
pub(super) type CleanupFn = Arc<
    dyn Fn(
            &qol_process::ProcessTreeGuard,
            Duration,
        ) -> io::Result<qol_process::TerminatedProcessTree>
        + Send
        + Sync
        + 'static,
>;

pub(super) enum LifecycleEvent {
    Completed {
        proof: qol_process::TerminatedProcessTree,
        status: ExitStatus,
    },
    Failed(String),
    ReapedAfterFailure {
        status: Option<ExitStatus>,
        failure: String,
    },
}

pub(super) struct LifecycleRegistration {
    shared: Arc<LifecycleShared>,
    attached: bool,
}

pub(super) struct LifecycleHandle {
    shared: Arc<LifecycleShared>,
    active: bool,
}

struct LifecycleShared {
    state: Mutex<LifecycleState>,
    wake: Condvar,
}

enum LifecycleState {
    AwaitingWorker,
    Running(OwnedWorker),
    Scheduled {
        worker: OwnedWorker,
        action: Option<LifecycleAction>,
    },
    Closed,
}

struct OwnedWorker {
    child: Child,
    process_tree: qol_process::ProcessTreeGuard,
}

enum LifecycleAction {
    Finalize {
        timeout: Duration,
        known_status: Option<ExitStatus>,
        events: Option<Sender<LifecycleEvent>>,
        cleanup: CleanupFn,
    },
    Terminate {
        timeout: Duration,
        terminate: TerminationFn,
        events: Sender<LifecycleEvent>,
        cleanup: CleanupFn,
    },
}

struct RecoveryContext {
    timeout: Duration,
    events: Option<Sender<LifecycleEvent>>,
    cleanup: CleanupFn,
}

struct PendingRecovery {
    timeout: Duration,
    status: RootStatus,
    failure: Option<String>,
    events: Option<Sender<LifecycleEvent>>,
    cleanup: CleanupFn,
}

struct RecoveryJob {
    worker: OwnedWorker,
    pending: PendingRecovery,
}

struct RecoveryQueue {
    jobs: Mutex<VecDeque<RecoveryJob>>,
    wake: Condvar,
}

enum RootStatus {
    Pending,
    Reaped(Option<ExitStatus>),
}

static RECOVERY_QUEUE: Mutex<Option<Arc<RecoveryQueue>>> = Mutex::new(None);

impl LifecycleRegistration {
    pub(super) fn new(run_id: &str) -> Result<Self> {
        let shared = Arc::new(LifecycleShared {
            state: Mutex::new(LifecycleState::AwaitingWorker),
            wake: Condvar::new(),
        });
        let owner = Arc::clone(&shared);
        thread::Builder::new()
            .name(format!("qol-worker-owner-{run_id}"))
            .spawn(move || run_owner(owner))
            .context("failed to start worker lifecycle owner")?;
        Ok(Self {
            shared,
            attached: false,
        })
    }

    pub(super) fn attach(
        mut self,
        child: Child,
        process_tree: qol_process::ProcessTreeGuard,
    ) -> LifecycleHandle {
        let mut state = lock_state(&self.shared);
        assert!(matches!(*state, LifecycleState::AwaitingWorker));
        *state = LifecycleState::Running(OwnedWorker {
            child,
            process_tree,
        });
        drop(state);
        self.attached = true;
        LifecycleHandle {
            shared: Arc::clone(&self.shared),
            active: true,
        }
    }
}

impl Drop for LifecycleRegistration {
    fn drop(&mut self) {
        if self.attached {
            return;
        }
        let mut state = lock_state(&self.shared);
        *state = LifecycleState::Closed;
        drop(state);
        self.shared.wake.notify_one();
    }
}

impl LifecycleHandle {
    pub(super) fn write_input(&mut self, input: &[u8]) -> io::Result<()> {
        let mut state = lock_state(&self.shared);
        let worker = running_worker(&mut state)?;
        let stdin = worker
            .child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("worker did not expose its typed input"))?;
        let mut stdin = BufWriter::new(stdin);
        stdin.write_all(input)?;
        stdin.flush()
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let mut state = lock_state(&self.shared);
        running_worker(&mut state)?.child.try_wait()
    }

    pub(super) fn finalize(
        mut self,
        timeout: Duration,
        known_status: Option<ExitStatus>,
    ) -> Receiver<LifecycleEvent> {
        let (events, receiver) = mpsc::channel();
        let action = LifecycleAction::Finalize {
            timeout,
            known_status,
            events: Some(events),
            cleanup: process_tree_cleanup(),
        };
        self.active = !schedule_action(&self.shared, action);
        receiver
    }

    pub(super) fn terminate(
        mut self,
        timeout: Duration,
        terminate: TerminationFn,
    ) -> Receiver<LifecycleEvent> {
        self.terminate_with_cleanup(timeout, terminate, process_tree_cleanup())
    }

    pub(super) fn terminate_with_cleanup(
        &mut self,
        timeout: Duration,
        terminate: TerminationFn,
        cleanup: CleanupFn,
    ) -> Receiver<LifecycleEvent> {
        let (events, receiver) = mpsc::channel();
        let action = LifecycleAction::Terminate {
            timeout,
            terminate,
            events,
            cleanup,
        };
        self.active = !schedule_action(&self.shared, action);
        receiver
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "windows")))]
    pub(super) fn pid(&self) -> u32 {
        let mut state = lock_state(&self.shared);
        running_worker(&mut state)
            .map(|worker| worker.child.id())
            .unwrap_or_default()
    }
}

impl Drop for LifecycleHandle {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let action = LifecycleAction::Finalize {
            timeout: super::BACKGROUND_CLEANUP_TIMEOUT,
            known_status: None,
            events: None,
            cleanup: process_tree_cleanup(),
        };
        self.active = !schedule_action(&self.shared, action);
    }
}

impl TerminationAttempt {
    pub(super) fn into_result(self) -> Result<qol_process::TerminatedProcessTree> {
        match (self.proof, self.root_stop) {
            (Ok(proof), Ok(())) => Ok(proof),
            (Err(tree), Ok(())) => Err(tree),
            (Ok(_), Err(root)) => Err(root),
            (Err(tree), Err(root)) => Err(anyhow!("{tree:#}; {root:#}")),
        }
    }
}

pub(super) fn terminate_process_tree(
    process_tree: &qol_process::ProcessTreeGuard,
    timeout: Duration,
) -> TerminationAttempt {
    let proof = process_tree
        .terminate_and_wait(timeout)
        .context("typed worker process tree survived termination");
    let root_stop = proof.as_ref().err().map_or(Ok(()), |_| {
        process_tree
            .terminate_root_and_wait(timeout)
            .context("failed to stop the exact typed worker root after tree proof failed")
    });
    TerminationAttempt { proof, root_stop }
}

fn running_worker(state: &mut LifecycleState) -> io::Result<&mut OwnedWorker> {
    match state {
        LifecycleState::Running(worker) => Ok(worker),
        _ => Err(io::Error::other(
            "worker lifecycle is no longer directly controlled",
        )),
    }
}

fn schedule_action(shared: &LifecycleShared, action: LifecycleAction) -> bool {
    let mut state = lock_state(shared);
    let previous = std::mem::replace(&mut *state, LifecycleState::Closed);
    let LifecycleState::Running(worker) = previous else {
        *state = previous;
        return false;
    };
    *state = LifecycleState::Scheduled {
        worker,
        action: Some(action),
    };
    drop(state);
    shared.wake.notify_one();
    true
}

fn lock_state(shared: &LifecycleShared) -> MutexGuard<'_, LifecycleState> {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn wait_for_action(shared: &LifecycleShared) -> MutexGuard<'_, LifecycleState> {
    let mut state = lock_state(shared);
    loop {
        if matches!(
            *state,
            LifecycleState::Scheduled { .. } | LifecycleState::Closed
        ) {
            return state;
        }
        state = shared
            .wake
            .wait(state)
            .unwrap_or_else(|error| error.into_inner());
    }
}

fn run_owner(shared: Arc<LifecycleShared>) {
    let Some((mut worker, action)) = take_scheduled_action(&shared) else {
        return;
    };
    let recovery = action.recovery_context();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_action(&mut worker, action)
    }));
    let pending = match result {
        Ok(pending) => pending,
        Err(_) => recover_owner_panic(&mut worker, recovery),
    };
    if let Some(pending) = pending {
        transfer_recovery(RecoveryJob { worker, pending });
    }
}

fn take_scheduled_action(shared: &LifecycleShared) -> Option<(OwnedWorker, LifecycleAction)> {
    let mut state = wait_for_action(shared);
    let previous = std::mem::replace(&mut *state, LifecycleState::Closed);
    let LifecycleState::Scheduled { worker, action } = previous else {
        return None;
    };
    let action = action.unwrap_or_else(background_finalize_action);
    Some((worker, action))
}

fn background_finalize_action() -> LifecycleAction {
    LifecycleAction::Finalize {
        timeout: super::BACKGROUND_CLEANUP_TIMEOUT,
        known_status: None,
        events: None,
        cleanup: process_tree_cleanup(),
    }
}

impl LifecycleAction {
    fn recovery_context(&self) -> RecoveryContext {
        match self {
            Self::Finalize {
                timeout,
                events,
                cleanup,
                ..
            } => RecoveryContext {
                timeout: *timeout,
                events: events.clone(),
                cleanup: Arc::clone(cleanup),
            },
            Self::Terminate {
                timeout: _,
                events,
                cleanup,
                ..
            } => RecoveryContext {
                timeout: super::BACKGROUND_CLEANUP_TIMEOUT,
                events: Some(events.clone()),
                cleanup: Arc::clone(cleanup),
            },
        }
    }
}

fn run_action(worker: &mut OwnedWorker, action: LifecycleAction) -> Option<PendingRecovery> {
    match action {
        LifecycleAction::Finalize {
            timeout,
            known_status,
            events,
            cleanup,
        } => finalize_worker(worker, timeout, known_status, events, cleanup),
        LifecycleAction::Terminate {
            timeout,
            terminate,
            events,
            cleanup,
        } => terminate_worker(worker, timeout, terminate, events, cleanup),
    }
}

fn finalize_worker(
    worker: &mut OwnedWorker,
    timeout: Duration,
    known_status: Option<ExitStatus>,
    events: Option<Sender<LifecycleEvent>>,
    cleanup: CleanupFn,
) -> Option<PendingRecovery> {
    let mut failure = None;
    let status = known_status.or_else(|| wait_for_root(worker, &mut failure, events.as_ref()));
    finish_or_defer(
        worker,
        PendingRecovery {
            timeout,
            status: RootStatus::Reaped(status),
            failure,
            events,
            cleanup,
        },
    )
}

fn terminate_worker(
    worker: &mut OwnedWorker,
    timeout: Duration,
    terminate: TerminationFn,
    events: Sender<LifecycleEvent>,
    cleanup: CleanupFn,
) -> Option<PendingRecovery> {
    match terminate(&worker.process_tree, timeout).into_result() {
        Ok(proof) => {
            publish_terminated_worker(worker, proof, None, events);
            None
        }
        Err(error) => recover_termination(
            worker,
            super::BACKGROUND_CLEANUP_TIMEOUT,
            format!("{error:#}"),
            events,
            cleanup,
        ),
    }
}

fn publish_terminated_worker(
    worker: &mut OwnedWorker,
    proof: qol_process::TerminatedProcessTree,
    mut failure: Option<String>,
    events: Sender<LifecycleEvent>,
) {
    let status = wait_for_root(worker, &mut failure, Some(&events));
    publish_completion(proof, status, failure, Some(events));
}

fn recover_termination(
    worker: &mut OwnedWorker,
    timeout: Duration,
    message: String,
    events: Sender<LifecycleEvent>,
    cleanup: CleanupFn,
) -> Option<PendingRecovery> {
    let _ = events.send(LifecycleEvent::Failed(message.clone()));
    finish_or_defer(
        worker,
        PendingRecovery {
            timeout,
            status: RootStatus::Pending,
            failure: Some(message),
            events: Some(events),
            cleanup,
        },
    )
}

fn recover_owner_panic(
    worker: &mut OwnedWorker,
    recovery: RecoveryContext,
) -> Option<PendingRecovery> {
    let message = "typed worker lifecycle owner panicked".to_string();
    if let Some(events) = recovery.events.as_ref() {
        let _ = events.send(LifecycleEvent::Failed(message.clone()));
    }
    finish_or_defer(
        worker,
        PendingRecovery {
            timeout: recovery.timeout,
            status: RootStatus::Pending,
            failure: Some(message),
            events: recovery.events,
            cleanup: recovery.cleanup,
        },
    )
}

fn wait_for_root(
    worker: &mut OwnedWorker,
    failure: &mut Option<String>,
    events: Option<&Sender<LifecycleEvent>>,
) -> Option<ExitStatus> {
    match worker.child.wait() {
        Ok(status) => Some(status),
        Err(error) => {
            record_failure(
                failure,
                format!("failed to reap typed worker root: {error}"),
                events,
            );
            None
        }
    }
}

fn finish_or_defer(
    worker: &mut OwnedWorker,
    mut pending: PendingRecovery,
) -> Option<PendingRecovery> {
    match attempt_tree_exit(worker, pending.timeout, &pending.cleanup) {
        Ok(proof) => {
            finish_recovery(worker, pending, proof);
            None
        }
        Err(message) => {
            record_failure(&mut pending.failure, message, pending.events.as_ref());
            Some(pending)
        }
    }
}

fn attempt_tree_exit(
    worker: &OwnedWorker,
    timeout: Duration,
    cleanup: &CleanupFn,
) -> Result<qol_process::TerminatedProcessTree, String> {
    match cleanup(&worker.process_tree, timeout) {
        Ok(proof) => Ok(proof),
        Err(tree_error) => {
            let root_error = worker.process_tree.terminate_root_and_wait(timeout).err();
            Err(cleanup_failure_message(&tree_error, root_error.as_ref()))
        }
    }
}

fn finish_recovery(
    worker: &mut OwnedWorker,
    mut pending: PendingRecovery,
    proof: qol_process::TerminatedProcessTree,
) {
    let status = match pending.status {
        RootStatus::Pending => wait_for_root(worker, &mut pending.failure, pending.events.as_ref()),
        RootStatus::Reaped(status) => status,
    };
    publish_completion(proof, status, pending.failure, pending.events);
}

fn process_tree_cleanup() -> CleanupFn {
    Arc::new(|process_tree, timeout| process_tree.terminate_and_wait(timeout))
}

fn transfer_recovery(job: RecoveryJob) {
    send_recovery_state(
        job.pending.events.as_ref(),
        "typed worker cleanup remains pending under the background recovery coordinator",
    );
    let queue = match recovery_queue() {
        Ok(queue) => queue,
        Err(error) => {
            send_recovery_state(
                job.pending.events.as_ref(),
                &format!(
                    "background recovery coordinator is unavailable; lifecycle owner retained cleanup: {error}"
                ),
            );
            run_inline_recovery(job);
            return;
        }
    };
    queue.push(job);
}

fn recovery_queue() -> io::Result<Arc<RecoveryQueue>> {
    let mut slot = RECOVERY_QUEUE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(queue) = slot.as_ref() {
        return Ok(Arc::clone(queue));
    }
    let queue = Arc::new(RecoveryQueue {
        jobs: Mutex::new(VecDeque::new()),
        wake: Condvar::new(),
    });
    let coordinator = Arc::clone(&queue);
    thread::Builder::new()
        .name("qol-worker-recovery".into())
        .spawn(move || run_background_recovery(coordinator))?;
    *slot = Some(Arc::clone(&queue));
    Ok(queue)
}

fn run_background_recovery(queue: Arc<RecoveryQueue>) {
    loop {
        let job = queue.take();
        let Some(job) = run_recovery_attempt(job) else {
            continue;
        };
        thread::sleep(CLEANUP_RETRY_INTERVAL);
        queue.push(job);
    }
}

fn run_inline_recovery(mut job: RecoveryJob) {
    loop {
        let Some(pending) = run_recovery_attempt(job) else {
            return;
        };
        job = pending;
        thread::sleep(CLEANUP_RETRY_INTERVAL);
    }
}

fn run_recovery_attempt(mut job: RecoveryJob) -> Option<RecoveryJob> {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        attempt_tree_exit(&job.worker, job.pending.timeout, &job.pending.cleanup)
    }));
    match attempt {
        Ok(Ok(proof)) => {
            finish_recovery(&mut job.worker, job.pending, proof);
            None
        }
        Ok(Err(message)) => {
            record_failure(
                &mut job.pending.failure,
                message,
                job.pending.events.as_ref(),
            );
            Some(job)
        }
        Err(_) => {
            send_recovery_state(
                job.pending.events.as_ref(),
                "background typed worker recovery panicked and will retry",
            );
            Some(job)
        }
    }
}

fn send_recovery_state(events: Option<&Sender<LifecycleEvent>>, message: &str) {
    if let Some(events) = events {
        let _ = events.send(LifecycleEvent::Failed(message.to_string()));
    }
}

impl RecoveryQueue {
    fn push(&self, job: RecoveryJob) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        jobs.push_back(job);
        drop(jobs);
        self.wake.notify_one();
    }

    fn take(&self) -> RecoveryJob {
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(job) = jobs.pop_front() {
                return job;
            }
            jobs = self
                .wake
                .wait(jobs)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

fn cleanup_failure_message(tree: &io::Error, root: Option<&io::Error>) -> String {
    root.map_or_else(
        || format!("typed worker residual process tree is not yet clean: {tree}"),
        |root| {
            format!(
                "typed worker residual process tree is not yet clean: {tree}; exact root stop also failed: {root}"
            )
        },
    )
}

fn record_failure(
    failure: &mut Option<String>,
    message: String,
    events: Option<&Sender<LifecycleEvent>>,
) {
    if failure.is_some() {
        return;
    }
    if let Some(events) = events {
        let _ = events.send(LifecycleEvent::Failed(message.clone()));
    }
    *failure = Some(message);
}

fn publish_completion(
    proof: qol_process::TerminatedProcessTree,
    status: Option<ExitStatus>,
    failure: Option<String>,
    events: Option<Sender<LifecycleEvent>>,
) {
    let Some(events) = events else {
        return;
    };
    match (failure, status) {
        (None, Some(status)) => {
            let _ = events.send(LifecycleEvent::Completed { proof, status });
        }
        (failure, status) => {
            let failure =
                failure.unwrap_or_else(|| "typed worker exit status is unavailable".into());
            let _ = events.send(LifecycleEvent::ReapedAfterFailure { status, failure });
        }
    }
}
