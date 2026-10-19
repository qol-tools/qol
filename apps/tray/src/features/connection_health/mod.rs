use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use qol_connection_health::{ConnectionHealth, Outcome, Thresholds, Ticket, Tracker};
use tokio::sync::Notify;

mod body;
mod probe;
mod service;
#[cfg(test)]
mod tests;

pub use body::{observe_body, ObservedBody};

const THRESHOLDS: Thresholds = Thresholds {
    slow: Duration::from_millis(1500),
    bad: Duration::from_millis(4000),
    memory: Duration::from_secs(180),
};
const INTEREST_WINDOW: Duration = Duration::from_secs(10);

struct Shared {
    tracker: Mutex<Tracker>,
    interest_until: Mutex<Option<Instant>>,
    wake: Notify,
}

impl Shared {
    fn new() -> Self {
        Self {
            tracker: Mutex::new(Tracker::new(THRESHOLDS)),
            interest_until: Mutex::new(None),
            wake: Notify::new(),
        }
    }

    fn tracker(&self) -> MutexGuard<'_, Tracker> {
        lock(&self.tracker)
    }

    fn note_interest(&self, now: Instant) {
        *lock(&self.interest_until) = Some(now + INTEREST_WINDOW);
        self.wake.notify_one();
    }

    fn watching(&self, now: Instant) -> bool {
        lock(&self.interest_until).is_some_and(|until| until > now)
    }
}

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn spawn() {
    tokio::spawn(service::run());
}

pub fn current() -> ConnectionHealth {
    shared().tracker().health(Instant::now())
}

pub fn note_interest() {
    shared().note_interest(Instant::now());
}

pub fn is_unreachable(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request() || lost_connection(error)
}

fn lost_connection(error: &reqwest::Error) -> bool {
    std::iter::successors(std::error::Error::source(error), |cause| cause.source()).any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            )
        })
    })
}

pub trait SendObserved {
    fn send_observed(
        self,
    ) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send;
}

impl SendObserved for reqwest::RequestBuilder {
    fn send_observed(
        self,
    ) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send {
        observe(shared(), self)
    }
}

async fn observe(
    shared: &'static Shared,
    request: reqwest::RequestBuilder,
) -> reqwest::Result<reqwest::Response> {
    let mut watch = Watch::begin(shared);
    let result = request.send().await;
    watch.end(outcome(&result));
    result
}

fn outcome(result: &reqwest::Result<reqwest::Response>) -> Option<Outcome> {
    match result {
        Ok(_) => Some(Outcome::Answered),
        Err(error) => failure(error),
    }
}

fn failure(error: &reqwest::Error) -> Option<Outcome> {
    is_unreachable(error).then_some(Outcome::Unreachable)
}

struct Watch {
    shared: &'static Shared,
    ticket: Option<Ticket>,
}

impl Watch {
    fn begin(shared: &'static Shared) -> Self {
        let ticket = shared.tracker().begin(Instant::now());
        shared.wake.notify_one();
        Self {
            shared,
            ticket: Some(ticket),
        }
    }

    fn progress(&self) {
        if let Some(ticket) = &self.ticket {
            self.shared.tracker().progress(ticket, Instant::now());
        }
    }

    fn end(&mut self, outcome: Option<Outcome>) {
        let Some(ticket) = self.ticket.take() else {
            return;
        };
        {
            let mut tracker = self.shared.tracker();
            match outcome {
                Some(outcome) => tracker.finish(ticket, outcome, Instant::now()),
                None => tracker.abandon(ticket),
            }
        }
        self.shared.wake.notify_one();
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.end(None);
    }
}

fn shared() -> &'static Shared {
    SHARED.get_or_init(Shared::new)
}

fn interested(now: Instant) -> bool {
    shared().watching(now)
        || crate::runtime::is_subscribed(
            qol_runtime::protocol::RuntimeEventKind::ConnectionHealthChanged,
        )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
