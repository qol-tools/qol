#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeStatus {
    Ok,
    Warn,
    Error,
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
    pub outcomes: Vec<Outcome>,
}

impl Report {
    pub fn count_ok(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Ok))
            .count()
    }

    pub fn count_warn(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Warn))
            .count()
    }

    pub fn count_error(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| matches!(outcome.status, OutcomeStatus::Error))
            .count()
    }

    pub fn has_warnings(&self) -> bool {
        self.count_warn() > 0
    }

    pub fn has_errors(&self) -> bool {
        self.count_error() > 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct FixReport {
    pub before: Report,
    pub after: Report,
    pub attempted: usize,
    pub applied: usize,
    pub failures: Vec<String>,
}

pub(super) fn report(outcomes: Vec<Outcome>) -> Report {
    Report { outcomes }
}
