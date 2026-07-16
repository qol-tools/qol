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
    },
    Terminate {
        timeout: Duration,
        terminate: TerminationFn,
        events: Sender<LifecycleEvent>,
    },
}

struct RecoveryContext {
    timeout: Duration,
    events: Option<Sender<LifecycleEvent>>,
}

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
        };
        self.active = !schedule_action(&self.shared, action);
        receiver
    }

    pub(super) fn terminate(
        mut self,
        timeout: Duration,
        terminate: TerminationFn,
    ) -> Receiver<LifecycleEvent> {
        let (events, receiver) = mpsc::channel();
        let action = LifecycleAction::Terminate {
            timeout,
            terminate,
            events,
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
    let mut state = wait_for_action(&shared);
    let LifecycleState::Scheduled { worker, action } = &mut *state else {
        return;
    };
    let action = action.take().unwrap_or(LifecycleAction::Finalize {
        timeout: super::BACKGROUND_CLEANUP_TIMEOUT,
        known_status: None,
        events: None,
    });
    let recovery = action.recovery_context();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_action(worker, action)));
    if result.is_err() {
        recover_owner_panic(worker, recovery);
    }
    *state = LifecycleState::Closed;
}

impl LifecycleAction {
    fn recovery_context(&self) -> RecoveryContext {
        match self {
            Self::Finalize {
                timeout, events, ..
            } => RecoveryContext {
                timeout: *timeout,
                events: events.clone(),
            },
            Self::Terminate {
                timeout: _, events, ..
            } => RecoveryContext {
                timeout: super::BACKGROUND_CLEANUP_TIMEOUT,
                events: Some(events.clone()),
            },
        }
    }
}

fn run_action(worker: &mut OwnedWorker, action: LifecycleAction) {
    match action {
        LifecycleAction::Finalize {
            timeout,
            known_status,
            events,
        } => finalize_worker(worker, timeout, known_status, events),
        LifecycleAction::Terminate {
            timeout,
            terminate,
            events,
        } => terminate_worker(worker, timeout, terminate, events),
    }
}

fn finalize_worker(
    worker: &mut OwnedWorker,
    timeout: Duration,
    known_status: Option<ExitStatus>,
    events: Option<Sender<LifecycleEvent>>,
) {
    let mut failure = None;
    let status = known_status.or_else(|| wait_for_root(worker, &mut failure, events.as_ref()));
    let proof = prove_tree_exit(worker, timeout, &mut failure, events.as_ref());
    publish_completion(proof, status, failure, events);
}

fn terminate_worker(
    worker: &mut OwnedWorker,
    timeout: Duration,
    terminate: TerminationFn,
    events: Sender<LifecycleEvent>,
) {
    match terminate(&worker.process_tree, timeout).into_result() {
        Ok(proof) => publish_terminated_worker(worker, proof, None, events),
        Err(error) => recover_termination(
            worker,
            super::BACKGROUND_CLEANUP_TIMEOUT,
            format!("{error:#}"),
            events,
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
) {
    let _ = events.send(LifecycleEvent::Failed(message.clone()));
    let mut failure = Some(message);
    let proof = prove_tree_exit(worker, timeout, &mut failure, Some(&events));
    let status = wait_for_root(worker, &mut failure, Some(&events));
    publish_completion(proof, status, failure, Some(events));
}

fn recover_owner_panic(worker: &mut OwnedWorker, recovery: RecoveryContext) {
    let message = "typed worker lifecycle owner panicked".to_string();
    if let Some(events) = recovery.events.as_ref() {
        let _ = events.send(LifecycleEvent::Failed(message.clone()));
    }
    let mut failure = Some(message);
    let proof = prove_tree_exit(
        worker,
        recovery.timeout,
        &mut failure,
        recovery.events.as_ref(),
    );
    let status = wait_for_root(worker, &mut failure, recovery.events.as_ref());
    publish_completion(proof, status, failure, recovery.events);
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

fn prove_tree_exit(
    worker: &OwnedWorker,
    timeout: Duration,
    failure: &mut Option<String>,
    events: Option<&Sender<LifecycleEvent>>,
) -> qol_process::TerminatedProcessTree {
    loop {
        match worker.process_tree.terminate_and_wait(timeout) {
            Ok(proof) => return proof,
            Err(tree_error) => {
                let root_error = worker.process_tree.terminate_root_and_wait(timeout).err();
                let message = cleanup_failure_message(&tree_error, root_error.as_ref());
                record_failure(failure, message, events);
                thread::sleep(CLEANUP_RETRY_INTERVAL);
            }
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
