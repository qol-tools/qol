use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::UPDATE_ALREADY_RUNNING;

pub(crate) const HOST_ID: &str = "qol-tray";

static JOBS: OnceLock<Mutex<HashMap<String, UpdateJob>>> = OnceLock::new();
static NEXT_ORDER: AtomicU64 = AtomicU64::new(0);

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

impl UpdateJob {
    fn new(state: JobState) -> Self {
        Self {
            state,
            progress: None,
            reason: None,
            order: NEXT_ORDER.fetch_add(1, Ordering::Relaxed),
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

pub(crate) fn start_removing(id: &str) -> Result<(), String> {
    with_jobs(|jobs| begin_job(jobs, id, JobState::Removing))
        .unwrap_or_else(|| Err("The update state is unavailable".to_string()))
}

pub(crate) fn queue(id: &str) {
    with_jobs(|jobs| jobs.insert(id.to_string(), UpdateJob::new(JobState::Queued)));
}

pub(crate) fn take_queued(id: &str) -> bool {
    with_jobs(|jobs| take_queued_job(jobs, id)).unwrap_or(false)
}

pub(crate) fn next_queued() -> Option<String> {
    with_jobs(|jobs| oldest_queued(jobs)).flatten()
}

pub(crate) fn remove_queued(id: &str) -> bool {
    with_jobs(|jobs| remove_queued_job(jobs, id)).unwrap_or(false)
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
    begin_job(jobs, id, JobState::Updating)
}

fn begin_job(
    jobs: &mut HashMap<String, UpdateJob>,
    id: &str,
    state: JobState,
) -> Result<(), String> {
    if jobs.get(id).is_some_and(UpdateJob::is_active) {
        return Err(UPDATE_ALREADY_RUNNING.to_string());
    }
    jobs.insert(id.to_string(), UpdateJob::new(state));
    Ok(())
}

fn oldest_queued(jobs: &HashMap<String, UpdateJob>) -> Option<String> {
    jobs.iter()
        .filter(|(_, job)| job.state == JobState::Queued)
        .min_by_key(|(_, job)| job.order)
        .map(|(id, _)| id.clone())
}

fn remove_queued_job(jobs: &mut HashMap<String, UpdateJob>, id: &str) -> bool {
    if jobs
        .get(id)
        .is_some_and(|job| job.state == JobState::Queued)
    {
        jobs.remove(id);
        return true;
    }
    false
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

    #[test]
    fn the_oldest_queued_job_comes_next_whatever_the_ids_sort_to() {
        let mut jobs = jobs_of(&[
            ("zed", JobState::Queued),
            ("alpha", JobState::Updating),
            ("mid", JobState::Queued),
            ("beta", JobState::Queued),
        ]);
        let mut taken = Vec::new();
        while let Some(id) = oldest_queued(&jobs) {
            assert!(take_queued_job(&mut jobs, &id));
            taken.push(id);
        }
        assert_eq!(taken, ["zed", "mid", "beta"]);
    }

    #[test]
    fn a_requeued_job_goes_to_the_back_of_the_queue() {
        let mut jobs = jobs_of(&[("first", JobState::Failed), ("second", JobState::Queued)]);
        jobs.insert("first".to_string(), UpdateJob::new(JobState::Queued));
        assert_eq!(oldest_queued(&jobs).as_deref(), Some("second"));
    }

    #[test]
    fn nothing_comes_next_without_a_queued_job() {
        let jobs = jobs_of(&[("cli", JobState::Updating), ("bt", JobState::Failed)]);
        assert_eq!(oldest_queued(&jobs), None);
        assert_eq!(oldest_queued(&HashMap::new()), None);
    }

    #[test]
    fn only_a_queued_job_is_removed() {
        let mut jobs = jobs_of(&[
            ("ln", JobState::Queued),
            ("cli", JobState::Updating),
            ("bt", JobState::Failed),
            ("rm", JobState::Removing),
        ]);
        assert!(remove_queued_job(&mut jobs, "ln"));
        assert!(!jobs.contains_key("ln"));
        for id in ["cli", "bt", "rm", "missing"] {
            assert!(!remove_queued_job(&mut jobs, id), "id: {id}");
        }
        assert_eq!(jobs.len(), 3);
    }

    #[test]
    fn removing_is_refused_while_another_job_runs_and_blocks_new_ones() {
        let mut jobs = jobs_of(&[("ln", JobState::Queued), ("bt", JobState::Failed)]);
        assert!(begin_job(&mut jobs, "ln", JobState::Removing).is_err());
        assert!(begin_job(&mut jobs, "bt", JobState::Removing).is_ok());
        assert_eq!(jobs["bt"].state, JobState::Removing);
        assert!(start_job(&mut jobs, "bt").is_err());
    }
}
