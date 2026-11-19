use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::Instant;

use super::ops::{Held, MoveRecord, MoveState};
use super::ops_text::{self, Outcome};
use super::Browser;
use crate::ops::history::StepId;
use crate::ops::{self, Done, Job, Progress, Survey, Verb};

pub(super) struct Trip {
    pub verb: Verb,
    pub into: PathBuf,
    pub label: String,
    pub bytes: u64,
    pub crosses_volume: bool,
    pub from_folder: String,
    pub from_volume: String,
    pub kept: String,
    pub step: StepId,
    pub slot: usize,
}

pub(super) struct Running {
    pub job: Job,
    pub trip: Trip,
    pub done: u64,
    pub total: u64,
    pub arriving: Option<PathBuf>,
    pub started: Instant,
}

impl Browser {
    pub(super) fn enqueue(&mut self, survey: Survey, step: StepId) {
        let transfer = &survey.transfer;
        let label = ops_text::label_of(&transfer.sources);
        let first = transfer.sources.first();
        let from_folder = first
            .and_then(|source| source.parent())
            .map(ops_text::name_of)
            .unwrap_or_default();
        let from_volume = match (survey.crosses_volume, first) {
            (true, Some(source)) => volume_label(source).unwrap_or_else(|| from_folder.clone()),
            _ => from_folder.clone(),
        };
        let queued = self.ops.running.is_some();
        let title = ops_text::title_of(&label, &transfer.into);
        let slot = self.ops.open_record(MoveRecord {
            title,
            detail: if queued {
                "Waiting for the move before it.".to_string()
            } else {
                String::new()
            },
            bytes: Some(survey.bytes),
            state: if queued {
                MoveState::NotStarted
            } else {
                MoveState::Running
            },
        });
        let trip = Trip {
            verb: transfer.verb,
            into: transfer.into.clone(),
            label,
            bytes: survey.bytes,
            crosses_volume: survey.crosses_volume,
            from_folder,
            from_volume,
            kept: ops_text::kept_sentence(&survey.clashes, survey.same_inside),
            step,
            slot,
        };
        if queued {
            self.ops.queued.push_back((survey, trip));
        } else {
            self.run(survey, trip);
        }
    }

    fn run(&mut self, survey: Survey, trip: Trip) {
        let total = survey.bytes;
        let job = ops::start(survey, self.ops.tools.clone(), self.waker());
        if let Some(record) = self.ops.record_at(trip.slot) {
            record.state = MoveState::Running;
            record.detail = ops_text::running_detail(
                trip.verb,
                &trip.from_volume,
                0,
                total,
                Default::default(),
            );
        }
        self.ops.running = Some(Running {
            job,
            trip,
            done: 0,
            total,
            arriving: None,
            started: Instant::now(),
        });
    }

    pub(super) fn drain_job(&mut self) -> bool {
        let Some(running) = self.ops.running.as_mut() else {
            return false;
        };
        let mut changed = false;
        let mut finished = None;
        let mut lost = false;
        loop {
            match running.job.progress.try_recv() {
                Ok(Progress::Bytes { done, total }) => {
                    running.done = done;
                    running.total = total;
                    changed = true;
                }
                Ok(Progress::Arriving(path)) => {
                    running.arriving = Some(path);
                    changed = true;
                }
                Ok(Progress::Placed(change)) => {
                    self.ops.history.record(running.trip.step, *change);
                    changed = true;
                }
                Ok(Progress::Amended { to, source }) => {
                    self.ops.history.amend(running.trip.step, &to, source);
                }
                Ok(Progress::Retracted(at)) => {
                    self.ops.history.retract(running.trip.step, &at);
                }
                Ok(Progress::Finished(done)) => {
                    finished = Some(done);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    lost = true;
                    break;
                }
            }
        }
        if let Some(done) = finished {
            self.finish(Some(done));
            return true;
        }
        if lost {
            self.finish(None);
            return true;
        }
        if changed {
            let (verb, from, slot, done, total, elapsed) = (
                running.trip.verb,
                running.trip.from_volume.clone(),
                running.trip.slot,
                running.done,
                running.total,
                running.started.elapsed(),
            );
            if let Some(record) = self.ops.record_at(slot) {
                record.bytes = Some(total);
                record.detail = ops_text::running_detail(verb, &from, done, total, elapsed);
            }
        }
        changed
    }

    fn finish(&mut self, done: Option<Done>) {
        let Some(running) = self.ops.running.take() else {
            return;
        };
        let trip = running.trip;
        self.ops.history.close(trip.step);
        let recorded = self
            .ops
            .history
            .step(trip.step)
            .map_or(0, |step| step.items.len());
        let Some(done) = done else {
            let arrived = self
                .ops
                .history
                .step(trip.step)
                .map_or(0, ops_text::arrived_items);
            if let Some(record) = self.ops.record_at(trip.slot) {
                record.state = MoveState::Failed;
                record.detail = ops_text::stopped_detail(arrived);
            }
            self.post(ops_text::stopped_before_the_end(&trip.label));
            self.reload();
            self.start_next();
            return;
        };
        let outcome = Outcome {
            verb: trip.verb,
            bytes: trip.bytes,
            crosses_volume: trip.crosses_volume,
            from_volume: &trip.from_volume,
            from_folder: &trip.from_folder,
            kept_originals: done.kept_originals.len(),
            failures: done.failures.len(),
            kept: &trip.kept,
        };
        let detail = ops_text::done_detail(&outcome);
        let all_failed = done.placed.is_empty() && !done.failures.is_empty();
        if let Some(record) = self.ops.record_at(trip.slot) {
            record.state = if all_failed {
                MoveState::Failed
            } else {
                MoveState::Done
            };
            record.detail = detail;
            record.bytes = Some(trip.bytes);
        }
        let here = trip.into == self.folder();
        if recorded == 0 && done.failures.is_empty() {
            self.post(ops_text::nothing_changed(&trip.into, self.folder()));
        } else if !done.placed.is_empty() {
            self.post(ops_text::arrived_note(
                trip.verb,
                done.placed.len(),
                here,
                &ops_text::name_of(&trip.into),
                &trip.from_folder,
                recorded > 0,
            ));
        }
        self.failure = None;
        let verb = ops_text::verb_noun(trip.verb);
        if let Some(message) = ops_text::failure_text(verb, "", &done.failures) {
            self.post(message);
        }
        self.release_cut(&done);
        self.reload();
        self.start_next();
    }

    fn release_cut(&mut self, done: &Done) {
        if done.verb != Verb::Move {
            return;
        }
        let Some(holding) = self.ops.holding.as_mut() else {
            return;
        };
        if holding.held != Held::Cut {
            return;
        }
        holding
            .paths
            .retain(|path| !done.placed.iter().any(|(from, _)| from == path));
        if holding.paths.is_empty() {
            self.ops.holding = None;
        }
    }

    fn start_next(&mut self) {
        if self.ops.running.is_some() {
            return;
        }
        if let Some((survey, trip)) = self.ops.queued.pop_front() {
            self.run(survey, trip);
        }
    }
}

fn volume_label(path: &Path) -> Option<String> {
    crate::places::device_label(&crate::places::devices(), path)
}
