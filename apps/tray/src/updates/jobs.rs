use std::collections::HashMap;
use std::sync::LazyLock;

use qol_work_queue::{Entry, Run, State, WorkQueue};

use super::UPDATE_ALREADY_RUNNING;

pub(crate) const HOST_ID: &str = "qol-tray";

pub(crate) static QUEUE: LazyLock<WorkQueue<Operation>> = LazyLock::new(WorkQueue::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    Install,
    Update,
    Remove,
    Host { confirm_after_restart: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobState {
    Queued,
    Updating,
    Failed,
    Removing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateJob {
    pub(crate) state: JobState,
    pub(crate) progress: Option<u8>,
    pub(crate) reason: Option<String>,
    pub(crate) order: u64,
}

pub(crate) fn push(id: &str, operation: Operation) -> Result<(), String> {
    let run = match operation {
        Operation::Host { .. } => Run::Alone,
        Operation::Install | Operation::Update | Operation::Remove => Run::InOrder,
    };
    QUEUE
        .push(id, operation, run)
        .map_err(|_| UPDATE_ALREADY_RUNNING.to_string())
}

pub(crate) fn snapshot() -> HashMap<String, UpdateJob> {
    QUEUE
        .snapshot()
        .into_iter()
        .map(|(id, entry)| (id, update_job(entry)))
        .collect()
}

pub(crate) fn get(id: &str) -> Option<UpdateJob> {
    QUEUE.get(id).map(update_job)
}

fn update_job(entry: Entry<Operation>) -> UpdateJob {
    let state = match (entry.state, entry.task) {
        (State::Queued, _) => JobState::Queued,
        (State::Running, Operation::Remove) => JobState::Removing,
        (State::Running, _) => JobState::Updating,
        (State::Failed, _) => JobState::Failed,
    };
    UpdateJob {
        state,
        progress: entry.progress,
        reason: entry.reason,
        order: entry.order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: State, task: Operation) -> Entry<Operation> {
        Entry {
            task,
            state,
            progress: Some(7),
            reason: None,
            order: 3,
        }
    }

    #[test]
    fn a_running_removal_reads_as_removing_and_everything_else_by_its_state() {
        let host = Operation::Host {
            confirm_after_restart: true,
        };
        let cases = [
            (State::Queued, Operation::Remove, JobState::Queued),
            (State::Running, Operation::Remove, JobState::Removing),
            (State::Running, Operation::Install, JobState::Updating),
            (State::Running, Operation::Update, JobState::Updating),
            (State::Running, host, JobState::Updating),
            (State::Failed, Operation::Remove, JobState::Failed),
        ];
        for (state, task, expected) in cases {
            let job = update_job(entry(state, task));
            assert_eq!(job.state, expected, "{state:?} {task:?}");
            assert_eq!((job.progress, job.order), (Some(7), 3));
        }
    }
}
