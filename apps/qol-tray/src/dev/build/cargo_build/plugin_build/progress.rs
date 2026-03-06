use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::dev::core::progress_estimator::CargoProgressEstimator;
use crate::dev::core::progress_parser::CargoProgressSnapshot;

pub(super) fn emit_progress<F>(rx: &Receiver<CargoProgressSnapshot>, on_progress: &mut F)
where
    F: FnMut(u8, String),
{
    ProgressDriver::default().run(rx, on_progress);
}

struct ProgressDriver {
    estimator: CargoProgressEstimator,
    latest_snapshot: Option<CargoProgressSnapshot>,
    last_percent: u8,
    last_phase: String,
    last_emit_at: Instant,
    started_at: Instant,
}

impl Default for ProgressDriver {
    fn default() -> Self {
        Self {
            estimator: CargoProgressEstimator::default(),
            latest_snapshot: None,
            last_percent: 3,
            last_phase: String::new(),
            last_emit_at: Instant::now(),
            started_at: Instant::now(),
        }
    }
}

impl ProgressDriver {
    fn run<F>(mut self, rx: &Receiver<CargoProgressSnapshot>, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        loop {
            match rx.recv_timeout(Duration::from_millis(220)) {
                Ok(snapshot) => self.on_snapshot(snapshot, on_progress),
                Err(RecvTimeoutError::Timeout) => self.on_timeout(on_progress),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn on_snapshot<F>(&mut self, snapshot: CargoProgressSnapshot, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        self.latest_snapshot = Some(snapshot);
        self.emit(true, on_progress);
    }

    fn on_timeout<F>(&mut self, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        self.emit(false, on_progress);
    }

    fn emit<F>(&mut self, allow_phase_only_emit: bool, on_progress: &mut F)
    where
        F: FnMut(u8, String),
    {
        let Some(snapshot) = self.latest_snapshot.clone() else {
            return;
        };
        let (percent, phase) = self.next_progress(&snapshot);
        if !self.should_emit(percent, &phase, allow_phase_only_emit) {
            return;
        }
        on_progress(percent, phase.clone());
        self.last_percent = percent;
        self.last_phase = phase;
        self.last_emit_at = Instant::now();
    }

    fn next_progress(&mut self, snapshot: &CargoProgressSnapshot) -> (u8, String) {
        let elapsed_secs = self.started_at.elapsed().as_secs_f64();
        let (percent, done, total) =
            self.estimator
                .update(snapshot.done, snapshot.total, elapsed_secs);
        (
            percent.max(self.last_percent),
            progress_phase(snapshot, done, total),
        )
    }

    fn should_emit(&self, percent: u8, phase: &str, allow_phase_only_emit: bool) -> bool {
        if percent > self.last_percent {
            return true;
        }
        allow_phase_only_emit
            && phase != self.last_phase
            && self.last_emit_at.elapsed() >= Duration::from_millis(120)
    }
}

fn progress_phase(snapshot: &CargoProgressSnapshot, done: u32, total: u32) -> String {
    if snapshot.phase.is_empty() {
        return format!("{}/{}", done, total);
    }
    format!("{}/{} {}", done, total, snapshot.phase)
}
