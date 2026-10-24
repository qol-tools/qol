use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::Instant;

use super::ops::{Held, MoveRecord, MoveState};
use super::ops_text::{self, Outcome};
use super::Browser;
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
    pub journaled: bool,
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
    pub(super) fn enqueue(&mut self, survey: Survey, journaled: bool) {
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
            journaled,
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
        let job = ops::start(survey, self.waker());
        self.ops.notice = None;
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
        let Some(done) = done else {
            if let Some(record) = self.ops.record_at(trip.slot) {
                record.state = MoveState::Failed;
                record.detail = "Stopped before it finished. Nothing was removed.".to_string();
            }
            self.failure = Some(format!("{} stopped before it finished", trip.label));
            if !trip.journaled {
                self.ops.stop_undo();
            }
            self.reload();
            self.start_next();
            return;
        };
        if trip.journaled && !done.placed.is_empty() {
            self.ops.journal.record(&done);
        }
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
        if !done.placed.is_empty() {
            let here = trip.into == self.folder();
            self.ops.notice = Some(ops_text::arrived_note(
                trip.verb,
                done.placed.len(),
                here,
                &ops_text::name_of(&trip.into),
                &trip.from_folder,
                trip.journaled,
            ));
        }
        self.failure = ops_text::failure_text(&done.failures);
        self.release_cut(&done);
        self.reload();
        self.start_next();
        if !trip.journaled {
            self.continue_undo();
        }
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
