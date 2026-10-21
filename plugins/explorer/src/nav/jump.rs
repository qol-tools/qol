use std::time::{Duration, Instant};

use super::prefix::starts_with_ignoring_case;

pub const RUN_RESET: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Default)]
pub struct Jump {
    run: String,
    last: Option<Instant>,
}

impl Jump {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run(&self, now: Instant) -> &str {
        if self.expired(now) {
            return "";
        }
        &self.run
    }

    pub fn reset(&mut self) {
        self.run.clear();
        self.last = None;
    }

    pub fn type_char<T>(
        &mut self,
        typed: char,
        now: Instant,
        rows: &[T],
        cursor: usize,
        name: impl Fn(&T) -> &str,
    ) -> Option<usize> {
        if self.expired(now) {
            self.run.clear();
        }
        self.last = Some(now);
        let run = self.extended(typed);
        if let Some(index) = first_starting_with(rows, &run, &name) {
            self.run = run;
            return Some(index);
        }
        let letter = repeated_letter(&run)?;
        self.run = run;
        next_starting_with(rows, cursor, letter, name)
    }

    pub fn continues<T>(
        &self,
        typed: char,
        now: Instant,
        rows: &[T],
        name: impl Fn(&T) -> &str,
    ) -> bool {
        !self.run(now).is_empty()
            && first_starting_with(rows, &self.extended(typed), &name).is_some()
    }

    fn extended(&self, typed: char) -> String {
        let mut run = self.run.clone();
        run.extend(typed.to_lowercase());
        run
    }

    fn expired(&self, now: Instant) -> bool {
        self.last
            .is_some_and(|last| now.saturating_duration_since(last) >= RUN_RESET)
    }
}

fn repeated_letter(run: &str) -> Option<char> {
    let mut letters = run.chars();
    let first = letters.next()?;
    let mut rest = letters.peekable();
    rest.peek()?;
    rest.all(|letter| letter == first).then_some(first)
}

fn first_starting_with<T>(rows: &[T], run: &str, name: impl Fn(&T) -> &str) -> Option<usize> {
    rows.iter()
        .position(|row| starts_with_ignoring_case(name(row), run))
}

fn next_starting_with<T>(
    rows: &[T],
    cursor: usize,
    letter: char,
    name: impl Fn(&T) -> &str,
) -> Option<usize> {
    let letter = letter.to_string();
    let len = rows.len();
    (1..=len)
        .map(|step| (cursor + step) % len)
        .find(|&index| starts_with_ignoring_case(name(&rows[index]), &letter))
}
