use std::time::{Duration, Instant};

use crate::{Cause, ConnectionHealth, Level};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub slow: Duration,
    pub bad: Duration,
    pub memory: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    Unreachable,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

#[derive(Debug, Clone, Copy)]
struct Sample {
    started: Instant,
    at: Instant,
    outcome: Outcome,
}

#[derive(Debug)]
pub struct Tracker {
    thresholds: Thresholds,
    next_ticket: u64,
    in_flight: Vec<(u64, Instant)>,
    last: Option<Sample>,
    smoothed: Option<Duration>,
}

impl Tracker {
    pub fn new(thresholds: Thresholds) -> Self {
        Self {
            thresholds,
            next_ticket: 0,
            in_flight: Vec::new(),
            last: None,
            smoothed: None,
        }
    }

    pub fn begin(&mut self, now: Instant) -> Ticket {
        let id = self.next_ticket;
        self.next_ticket = self.next_ticket.wrapping_add(1);
        self.in_flight.push((id, now));
        Ticket(id)
    }

    pub fn finish(&mut self, ticket: Ticket, outcome: Outcome, now: Instant) {
        let Some(started) = self.take(ticket) else {
            return;
        };
        if self.superseded(started) {
            return;
        }
        if outcome == Outcome::Answered {
            let latency = now.saturating_duration_since(started);
            let previous = self.smoothed.filter(|_| self.remembers_last(now));
            self.smoothed = Some(previous.map_or(latency, |previous| (previous + latency) / 2));
        }
        self.last = Some(Sample {
            started,
            at: now,
            outcome,
        });
    }

    pub fn progress(&mut self, ticket: &Ticket, now: Instant) {
        if let Some(entry) = self.in_flight.iter_mut().find(|(id, _)| *id == ticket.0) {
            entry.1 = now;
        }
    }

    pub fn abandon(&mut self, ticket: Ticket) {
        self.take(ticket);
    }

    pub fn health(&self, now: Instant) -> ConnectionHealth {
        let settled = self.settled(now);
        match self.waiting(now) {
            Some(waiting) if waiting.level > settled.level => ConnectionHealth {
                latency_ms: settled.latency_ms,
                ..waiting
            },
            _ => settled,
        }
    }

    pub fn next_change(&self, now: Instant) -> Option<Instant> {
        let waiting = self.waiting_since().into_iter().flat_map(|started| {
            [
                started + self.thresholds.slow,
                started + self.thresholds.bad,
            ]
        });
        let forgotten = self.last.map(|sample| sample.at + self.thresholds.memory);
        waiting
            .chain(forgotten)
            .filter(|moment| *moment > now)
            .min()
    }

    pub fn last_observed(&self) -> Option<Instant> {
        self.last.map(|sample| sample.at)
    }

    pub fn waiting_since(&self) -> Option<Instant> {
        self.in_flight
            .iter()
            .map(|(_, started)| *started)
            .filter(|started| !self.superseded(*started))
            .min()
    }

    fn superseded(&self, started: Instant) -> bool {
        self.last.is_some_and(|sample| started < sample.started)
    }

    fn take(&mut self, ticket: Ticket) -> Option<Instant> {
        let index = self.in_flight.iter().position(|(id, _)| *id == ticket.0)?;
        Some(self.in_flight.swap_remove(index).1)
    }

    fn remembers_last(&self, now: Instant) -> bool {
        self.last
            .is_some_and(|sample| now.saturating_duration_since(sample.at) < self.thresholds.memory)
    }

    fn settled(&self, now: Instant) -> ConnectionHealth {
        if !self.remembers_last(now) {
            return ConnectionHealth::default();
        }
        if self
            .last
            .is_some_and(|sample| sample.outcome == Outcome::Unreachable)
        {
            return ConnectionHealth {
                level: Level::Bad,
                cause: Some(Cause::Unreachable),
                latency_ms: None,
            };
        }
        let Some(latency) = self.smoothed else {
            return ConnectionHealth::default();
        };
        let level = self.grade(latency);
        ConnectionHealth {
            level,
            cause: (level != Level::Good).then_some(Cause::SlowAnswers),
            latency_ms: Some(u64::try_from(latency.as_millis()).unwrap_or(u64::MAX)),
        }
    }

    fn waiting(&self, now: Instant) -> Option<ConnectionHealth> {
        let waited = now.saturating_duration_since(self.waiting_since()?);
        let level = self.grade(waited);
        (level != Level::Good).then_some(ConnectionHealth {
            level,
            cause: Some(Cause::Waiting),
            latency_ms: None,
        })
    }

    fn grade(&self, elapsed: Duration) -> Level {
        if elapsed >= self.thresholds.bad {
            Level::Bad
        } else if elapsed >= self.thresholds.slow {
            Level::Slow
        } else {
            Level::Good
        }
    }
}
