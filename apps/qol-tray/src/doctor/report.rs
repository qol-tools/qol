use super::framework::DoctorCheckResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeStatus {
    Ok,
    Warn,
    Error,
    Crash,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub id: &'static str,
    pub status: OutcomeStatus,
    pub message: String,
    pub fix_available: bool,
}

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

pub(super) fn report(results: Vec<DoctorCheckResult>) -> Report {
    Report { results }
}
