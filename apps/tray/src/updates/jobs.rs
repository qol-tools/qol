use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::UPDATE_ALREADY_RUNNING;

pub(crate) const HOST_ID: &str = "qol-tray";

static JOBS: OnceLock<Mutex<HashMap<String, UpdateJob>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobState {
    Queued,
    Updating,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateJob {
    pub(crate) state: JobState,
    pub(crate) progress: Option<u8>,
    pub(crate) reason: Option<String>,
}

impl UpdateJob {
    fn new(state: JobState) -> Self {
        Self {
            state,
            progress: None,
            reason: None,
        }
    }

    fn is_active(&self) -> bool {
        self.state != JobState::Failed
    }
}

pub(crate) fn snapshot() -> HashMap<String, UpdateJob> {
    with_jobs(|jobs| jobs.clone()).unwrap_or_default()
}

pub(crate) fn get(id: &str) -> Option<UpdateJob> {
    with_jobs(|jobs| jobs.get(id).cloned()).flatten()
}

pub(crate) fn any_active() -> bool {
    with_jobs(|jobs| jobs.values().any(UpdateJob::is_active)).unwrap_or(false)
}

pub(crate) fn start(id: &str) -> Result<(), String> {
    with_jobs(|jobs| start_job(jobs, id))
        .unwrap_or_else(|| Err("The update state is unavailable".to_string()))
}

pub(crate) fn queue(id: &str) {
    with_jobs(|jobs| jobs.insert(id.to_string(), UpdateJob::new(JobState::Queued)));
}

pub(crate) fn take_queued(id: &str) -> bool {
    with_jobs(|jobs| take_queued_job(jobs, id)).unwrap_or(false)
}

pub(crate) fn stop_queued() -> usize {
    with_jobs(remove_queued_jobs).unwrap_or(0)
}

pub(crate) fn set_progress(id: &str, percent: u8) {
    with_jobs(|jobs| {
        if let Some(job) = jobs.get_mut(id) {
            job.progress = Some(percent);
        }
    });
}

pub(crate) fn fail(id: &str, reason: String) {
    with_jobs(|jobs| {
        let mut job = UpdateJob::new(JobState::Failed);
        job.reason = Some(reason);
        jobs.insert(id.to_string(), job)
    });
}

pub(crate) fn finish(id: &str) {
    with_jobs(|jobs| jobs.remove(id));
}

pub(crate) fn release(id: &str) {
    with_jobs(|jobs| {
        if jobs
            .get(id)
            .is_some_and(|job| job.state == JobState::Updating)
        {
            jobs.remove(id);
        }
    });
}

fn start_job(jobs: &mut HashMap<String, UpdateJob>, id: &str) -> Result<(), String> {
    if jobs.get(id).is_some_and(UpdateJob::is_active) {
        return Err(UPDATE_ALREADY_RUNNING.to_string());
    }
    jobs.insert(id.to_string(), UpdateJob::new(JobState::Updating));
    Ok(())
}

fn take_queued_job(jobs: &mut HashMap<String, UpdateJob>, id: &str) -> bool {
    match jobs.get_mut(id) {
        Some(job) if job.state == JobState::Queued => {
            job.state = JobState::Updating;
            true
        }
        _ => false,
    }
}

fn remove_queued_jobs(jobs: &mut HashMap<String, UpdateJob>) -> usize {
    let before = jobs.len();
    jobs.retain(|_, job| job.state != JobState::Queued);
    before - jobs.len()
}

fn with_jobs<T>(f: impl FnOnce(&mut HashMap<String, UpdateJob>) -> T) -> Option<T> {
    lock_jobs().map(|mut jobs| f(&mut jobs))
}

fn lock_jobs() -> Option<MutexGuard<'static, HashMap<String, UpdateJob>>> {
    match JOBS.get_or_init(|| Mutex::new(HashMap::new())).lock() {
        Ok(guard) => Some(guard),
        Err(error) => {
            log::error!("Update job state lock is poisoned: {}", error);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jobs_of(entries: &[(&str, JobState)]) -> HashMap<String, UpdateJob> {
        entries
            .iter()
            .map(|(id, state)| (id.to_string(), UpdateJob::new(*state)))
            .collect()
    }

    #[test]
    fn starting_is_refused_while_queued_or_updating_and_allowed_after_failure() {
        let mut jobs = jobs_of(&[
            ("ln", JobState::Queued),
            ("cli", JobState::Updating),
            ("bt", JobState::Failed),
        ]);

        assert!(start_job(&mut jobs, "ln").is_err());
        assert!(start_job(&mut jobs, "cli").is_err());
        assert!(start_job(&mut jobs, "bt").is_ok());
        assert!(start_job(&mut jobs, "new").is_ok());
        assert_eq!(jobs["bt"].state, JobState::Updating);
        assert_eq!(jobs["new"].state, JobState::Updating);
    }

    #[test]
    fn stopping_drops_queued_jobs_and_leaves_the_running_one() {
        let mut jobs = jobs_of(&[
            ("ln", JobState::Queued),
            ("cli", JobState::Updating),
            ("bt", JobState::Failed),
        ]);

        assert_eq!(remove_queued_jobs(&mut jobs), 1);

        assert!(!jobs.contains_key("ln"));
        assert_eq!(jobs["cli"].state, JobState::Updating);
        assert_eq!(jobs["bt"].state, JobState::Failed);
    }

    #[test]
    fn only_a_still_queued_job_is_taken() {
        let mut jobs = jobs_of(&[("ln", JobState::Queued), ("cli", JobState::Updating)]);

        assert!(take_queued_job(&mut jobs, "ln"));
        assert_eq!(jobs["ln"].state, JobState::Updating);
        assert!(!take_queued_job(&mut jobs, "cli"));
        assert!(!take_queued_job(&mut jobs, "stopped"));
        assert!(!jobs.contains_key("stopped"));
    }
}
