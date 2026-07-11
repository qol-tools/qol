use super::framework::DoctorCheckResult;
use qol_conventions::doctor_wire::{FixReport as WireFixReport, Report as WireReport};

pub use qol_conventions::doctor_wire::{Outcome, OutcomeStatus};

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub(super) results: Vec<DoctorCheckResult>,
}

impl Report {
    pub fn outcomes(&self) -> impl Iterator<Item = &Outcome> {
        self.results.iter().map(|result| &result.outcome)
    }

    pub fn count_ok(&self) -> usize {
        self.outcomes()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Ok))
            .count()
    }

    pub fn count_warn(&self) -> usize {
        self.outcomes()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Warn))
            .count()
    }

    pub fn count_error(&self) -> usize {
        self.outcomes()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Error))
            .count()
    }

    pub fn count_crash(&self) -> usize {
        self.outcomes()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Crash))
            .count()
    }

    pub fn has_warnings(&self) -> bool {
        self.count_warn() > 0
    }

    pub fn has_errors(&self) -> bool {
        self.count_error() > 0
    }

    pub fn has_crashes(&self) -> bool {
        self.count_crash() > 0
    }

    pub(crate) fn to_wire(&self) -> WireReport {
        WireReport::new(self.outcomes().cloned().collect())
    }
}

#[derive(Clone, Debug, Default)]
pub struct FixReport {
    pub before: Report,
    pub after: Report,
    pub attempted: usize,
    pub applied: usize,
    pub skipped: usize,
    pub failures: Vec<String>,
}

impl FixReport {
    pub(crate) fn to_wire(&self) -> WireFixReport {
        WireFixReport {
            before: self.before.to_wire(),
            after: self.after.to_wire(),
            attempted: self.attempted,
            applied: self.applied,
            skipped: self.skipped,
            failures: self.failures.clone(),
        }
    }
}

pub(super) fn report(results: Vec<DoctorCheckResult>) -> Report {
    Report { results }
}
