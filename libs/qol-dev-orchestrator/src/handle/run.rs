use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use qol_dev_env::{CleanupState, RunSummary};

use super::lifecycle::{terminate_process_tree, LifecycleEvent, LifecycleHandle, TerminationFn};
use super::{RunTicket, BACKGROUND_CLEANUP_TIMEOUT};

const WAIT_INTERVAL: Duration = Duration::from_millis(100);
const MAX_PUBLIC_TERMINATION_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitState {
    Starting,
    Running(RunSummary),
    Terminal {
        report: RunSummary,
        worker_success: bool,
    },
    Failed {
        report: Option<RunSummary>,
        worker_exit: String,
    },
}

pub struct RunHandle {
    pub(super) ticket: RunTicket,
    pub(super) worker: Option<WorkerState>,
}

pub(super) enum WorkerState {
    Running(LifecycleHandle),
    Finalizing {
        events: Receiver<LifecycleEvent>,
    },
    Escalating {
        events: Receiver<LifecycleEvent>,
        failure: String,
    },
    Exited(ExitStatus),
    Failed {
        status: Option<ExitStatus>,
        failure: String,
    },
}

pub(super) enum WorkerObservation {
    Running,
    Exited(ExitStatus),
    Failed {
        status: Option<ExitStatus>,
        failure: String,
    },
}

impl WaitState {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Terminal { .. } | Self::Failed { .. })
    }
}

impl RunHandle {
    pub fn ticket(&self) -> &RunTicket {
        &self.ticket
    }

    pub fn cancel(&self) -> Result<PathBuf> {
        self.ticket.cancel()
    }

    pub fn poll(&mut self) -> Result<WaitState> {
        let observation = self.poll_worker()?;
        match self.ticket.read() {
            Ok(report) => Ok(wait_state(
                observation,
                report.map(|report| report.summary()),
            )),
            Err(error) => unreadable_report_state(observation, error),
        }
    }

    pub fn wait(&mut self) -> Result<WaitState> {
        loop {
            let state = self.poll()?;
            if state.is_finished() {
                return Ok(state);
            }
            thread::sleep(WAIT_INTERVAL);
        }
    }

    pub fn wait_timeout(&mut self, timeout: Duration) -> Result<Option<WaitState>> {
        let started = Instant::now();
        loop {
            let state = self.poll()?;
            if state.is_finished() {
                return Ok(Some(state));
            }
            let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                return Ok(None);
            };
            if remaining.is_zero() {
                return Ok(None);
            }
            thread::sleep(WAIT_INTERVAL.min(remaining));
        }
    }

    pub fn detach(self) -> RunTicket {
        self.ticket.clone()
    }

    pub fn terminate_worker(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<qol_process::TerminatedProcessTree>> {
        self.terminate_worker_with(timeout, Box::new(terminate_process_tree))
    }

    pub(super) fn terminate_worker_with(
        &mut self,
        timeout: Duration,
        terminate: TerminationFn,
    ) -> Result<Option<qol_process::TerminatedProcessTree>> {
        let wait_budget = termination_wait_budget(timeout)?;
        let Some(worker) = self.take_running_worker()? else {
            return Ok(None);
        };
        let events = worker.terminate(timeout, terminate);
        self.receive_termination_event(events, wait_budget)
    }

    fn take_running_worker(&mut self) -> Result<Option<LifecycleHandle>> {
        let Some(state) = self.worker.take() else {
            return Ok(None);
        };
        let WorkerState::Running(worker) = state else {
            let message = termination_state_error(&state);
            self.worker = Some(state);
            return message.map_or(Ok(None), |message| Err(anyhow!(message)));
        };
        Ok(Some(worker))
    }

    fn receive_termination_event(
        &mut self,
        events: Receiver<LifecycleEvent>,
        wait_budget: Duration,
    ) -> Result<Option<qol_process::TerminatedProcessTree>> {
        match events.recv_timeout(wait_budget) {
            Ok(LifecycleEvent::Completed { proof, status }) => {
                self.worker = Some(WorkerState::Exited(status));
                Ok(Some(proof))
            }
            Ok(LifecycleEvent::Failed(failure)) => self.escalating(events, failure),
            Ok(LifecycleEvent::ReapedAfterFailure { status, failure }) => {
                self.terminal_failure(status, failure)
            }
            Err(RecvTimeoutError::Timeout) => {
                let failure = format!(
                    "typed worker termination did not finish within {wait_budget:?}; exact ownership remains with the lifecycle owner"
                );
                self.escalating(events, failure)
            }
            Err(RecvTimeoutError::Disconnected) => self.escalating(
                events,
                "typed worker lifecycle owner stopped without process-tree proof".into(),
            ),
        }
    }

    fn escalating(
        &mut self,
        events: Receiver<LifecycleEvent>,
        failure: String,
    ) -> Result<Option<qol_process::TerminatedProcessTree>> {
        self.worker = Some(WorkerState::Escalating {
            events,
            failure: failure.clone(),
        });
        Err(anyhow!(failure))
    }

    fn terminal_failure(
        &mut self,
        status: Option<ExitStatus>,
        failure: String,
    ) -> Result<Option<qol_process::TerminatedProcessTree>> {
        self.worker = Some(WorkerState::Failed {
            status,
            failure: failure.clone(),
        });
        Err(anyhow!(failure))
    }

    pub(super) fn poll_worker(&mut self) -> Result<WorkerObservation> {
        let Some(worker) = self.worker.take() else {
            return Err(anyhow!("run worker is detached"));
        };
        match worker {
            WorkerState::Running(worker) => self.poll_running(worker),
            WorkerState::Finalizing { events } => self.poll_finalizing(events),
            WorkerState::Escalating { events, failure } => self.poll_escalating(events, failure),
            WorkerState::Exited(status) => {
                self.worker = Some(WorkerState::Exited(status));
                Ok(WorkerObservation::Exited(status))
            }
            WorkerState::Failed { status, failure } => {
                self.worker = Some(WorkerState::Failed {
                    status,
                    failure: failure.clone(),
                });
                Ok(WorkerObservation::Failed { status, failure })
            }
        }
    }

    fn poll_running(&mut self, mut worker: LifecycleHandle) -> Result<WorkerObservation> {
        let status = match worker.try_wait() {
            Ok(status) => status,
            Err(error) => {
                self.worker = Some(WorkerState::Running(worker));
                return Err(error).context("failed to poll typed worker root");
            }
        };
        let Some(status) = status else {
            self.worker = Some(WorkerState::Running(worker));
            return Ok(WorkerObservation::Running);
        };
        let events = worker.finalize(BACKGROUND_CLEANUP_TIMEOUT, Some(status));
        self.worker = Some(WorkerState::Finalizing { events });
        Ok(WorkerObservation::Running)
    }

    fn poll_finalizing(&mut self, events: Receiver<LifecycleEvent>) -> Result<WorkerObservation> {
        match events.try_recv() {
            Ok(LifecycleEvent::Completed { proof: _, status }) => {
                self.worker = Some(WorkerState::Exited(status));
                Ok(WorkerObservation::Exited(status))
            }
            Ok(LifecycleEvent::Failed(failure)) => self.active_failure(events, failure),
            Ok(LifecycleEvent::ReapedAfterFailure { status, failure }) => {
                Ok(self.complete_failure(status, failure))
            }
            Err(TryRecvError::Empty) => {
                self.worker = Some(WorkerState::Finalizing { events });
                Ok(WorkerObservation::Running)
            }
            Err(TryRecvError::Disconnected) => self.active_failure(
                events,
                "typed worker lifecycle owner stopped without residual-tree proof".into(),
            ),
        }
    }

    fn poll_escalating(
        &mut self,
        events: Receiver<LifecycleEvent>,
        failure: String,
    ) -> Result<WorkerObservation> {
        match events.try_recv() {
            Ok(LifecycleEvent::Completed { proof: _, status }) => {
                Ok(self.complete_failure(Some(status), failure))
            }
            Ok(LifecycleEvent::Failed(message)) => {
                self.active_failure(events, combine_failure(&failure, &message))
            }
            Ok(LifecycleEvent::ReapedAfterFailure {
                status,
                failure: message,
            }) => Ok(self.complete_failure(status, combine_failure(&failure, &message))),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                self.active_failure(events, failure)
            }
        }
    }

    fn active_failure(
        &mut self,
        events: Receiver<LifecycleEvent>,
        failure: String,
    ) -> Result<WorkerObservation> {
        self.worker = Some(WorkerState::Escalating {
            events,
            failure: failure.clone(),
        });
        Err(anyhow!(failure))
    }

    fn complete_failure(
        &mut self,
        status: Option<ExitStatus>,
        failure: String,
    ) -> WorkerObservation {
        self.worker = Some(WorkerState::Failed {
            status,
            failure: failure.clone(),
        });
        WorkerObservation::Failed { status, failure }
    }
}

fn termination_state_error(state: &WorkerState) -> Option<&'static str> {
    match state {
        WorkerState::Finalizing { .. } => Some("typed worker exit cleanup is already in progress"),
        WorkerState::Escalating { .. } => Some("typed worker termination is already in progress"),
        WorkerState::Failed { .. } => Some("typed worker termination previously failed"),
        WorkerState::Running(_) | WorkerState::Exited(_) => None,
    }
}

fn unreadable_report_state(
    observation: WorkerObservation,
    error: anyhow::Error,
) -> Result<WaitState> {
    match observation {
        WorkerObservation::Running => Err(error),
        observation => Ok(WaitState::Failed {
            report: None,
            worker_exit: format!(
                "{}; authoritative report is unreadable: {error:#}",
                observation_text(&observation)
            ),
        }),
    }
}

fn wait_state(observation: WorkerObservation, report: Option<RunSummary>) -> WaitState {
    match observation {
        WorkerObservation::Running => report.map_or(WaitState::Starting, WaitState::Running),
        WorkerObservation::Failed { status, failure } => WaitState::Failed {
            report,
            worker_exit: failure_text(status, &failure),
        },
        WorkerObservation::Exited(status) => exited_wait_state(status, report),
    }
}

fn exited_wait_state(status: ExitStatus, report: Option<RunSummary>) -> WaitState {
    match report {
        Some(report)
            if report.status.is_terminal() && matches!(report.cleanup, CleanupState::Complete) =>
        {
            WaitState::Terminal {
                report,
                worker_success: status.success(),
            }
        }
        report => WaitState::Failed {
            report,
            worker_exit: status.to_string(),
        },
    }
}

fn observation_text(observation: &WorkerObservation) -> String {
    match observation {
        WorkerObservation::Running => "worker is running".into(),
        WorkerObservation::Exited(status) => status.to_string(),
        WorkerObservation::Failed { status, failure } => failure_text(*status, failure),
    }
}

fn failure_text(status: Option<ExitStatus>, failure: &str) -> String {
    status.map_or_else(
        || format!("worker cleanup failed after process-tree proof: {failure}"),
        |status| {
            format!("worker exited {status}; cleanup failed after process-tree proof: {failure}")
        },
    )
}

fn combine_failure(existing: &str, additional: &str) -> String {
    if existing.contains(additional) || additional.contains(existing) {
        return existing.to_string();
    }
    format!("{existing}; {additional}")
}

fn termination_wait_budget(timeout: Duration) -> Result<Duration> {
    if timeout > MAX_PUBLIC_TERMINATION_TIMEOUT {
        return Err(anyhow!(
            "invalid typed worker termination timeout {timeout:?}; maximum is {MAX_PUBLIC_TERMINATION_TIMEOUT:?}"
        ));
    }
    timeout
        .checked_mul(2)
        .and_then(|budget| budget.checked_add(WAIT_INTERVAL))
        .ok_or_else(|| anyhow!("invalid typed worker termination timeout {timeout:?}"))
}
