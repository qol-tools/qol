use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use qol_connection_health::{ConnectionHealth, Outcome, Thresholds, Ticket, Tracker};
use tokio::sync::Notify;

mod probe;
mod service;
#[cfg(test)]
mod tests;

pub const QUERY: &str = "connection";

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

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn spawn() {
    tokio::spawn(service::run());
}

pub fn current() -> ConnectionHealth {
    tracker().health(Instant::now())
}

pub fn note_interest() {
    *lock(&shared().interest_until) = Some(Instant::now() + INTEREST_WINDOW);
    shared().wake.notify_one();
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
        observe(self)
    }
}

async fn observe(request: reqwest::RequestBuilder) -> reqwest::Result<reqwest::Response> {
    let watch = Watch::begin();
    let result = request.send().await;
    watch.end(outcome(&result));
    result
}

fn outcome(result: &reqwest::Result<reqwest::Response>) -> Option<Outcome> {
    match result {
        Ok(_) => Some(Outcome::Answered),
        Err(error) if error.is_timeout() || error.is_connect() || error.is_request() => {
            Some(Outcome::Unreachable)
        }
        Err(_) => None,
    }
}

struct Watch(Option<Ticket>);

impl Watch {
    fn begin() -> Self {
        let ticket = tracker().begin(Instant::now());
        shared().wake.notify_one();
        Self(Some(ticket))
    }

    fn end(mut self, outcome: Option<Outcome>) {
        let Some(ticket) = self.0.take() else {
            return;
        };
        {
            let mut tracker = tracker();
            match outcome {
                Some(outcome) => tracker.finish(ticket, outcome, Instant::now()),
                None => tracker.abandon(ticket),
            }
        }
        shared().wake.notify_one();
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(ticket) = self.0.take() {
            tracker().abandon(ticket);
            shared().wake.notify_one();
        }
    }
}

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| Shared {
        tracker: Mutex::new(Tracker::new(THRESHOLDS)),
        interest_until: Mutex::new(None),
        wake: Notify::new(),
    })
}

fn tracker() -> MutexGuard<'static, Tracker> {
    lock(&shared().tracker)
}

fn interested(now: Instant) -> bool {
    lock(&shared().interest_until).is_some_and(|until| until > now)
        || crate::runtime::is_subscribed(
            qol_runtime::protocol::RuntimeEventKind::ConnectionHealthChanged,
        )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
