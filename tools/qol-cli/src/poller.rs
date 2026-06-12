use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

pub(crate) struct Poller<T> {
    rx: Receiver<T>,
    poke_tx: Sender<()>,
}

impl<T: Send + 'static> Poller<T> {
    pub(crate) fn spawn(interval: Duration, work: impl FnMut() -> T + Send + 'static) -> Self {
        Self::spawn_with_backoff(interval, interval, work, |_| true)
    }

    #[allow(dead_code)]
    pub(crate) fn spawn_adaptive(
        base: Duration,
        cap: Duration,
        work: impl FnMut() -> T + Send + 'static,
    ) -> Self
    where
        T: Clone + PartialEq,
    {
        let mut prev: Option<T> = None;
        Self::spawn_with_backoff(base, cap, work, move |next| {
            let dirty = prev.as_ref() != Some(next);
            prev = Some(next.clone());
            dirty
        })
    }

    fn spawn_with_backoff(
        base: Duration,
        cap: Duration,
        mut work: impl FnMut() -> T + Send + 'static,
        mut changed: impl FnMut(&T) -> bool + Send + 'static,
    ) -> Self {
        let (result_tx, rx) = channel();
        let (poke_tx, poke_rx) = channel();
        std::thread::spawn(move || {
            let mut wait = base;
            loop {
                let result = work();
                let dirty = changed(&result);
                if result_tx.send(result).is_err() {
                    return;
                }
                match poke_rx.recv_timeout(wait) {
                    Ok(()) => {
                        while poke_rx.try_recv().is_ok() {}
                        wait = base;
                    }
                    Err(RecvTimeoutError::Timeout) => wait = next_wait(dirty, wait, base, cap),
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Self { rx, poke_tx }
    }

    pub(crate) fn latest(&self) -> Option<T> {
        let mut newest = None;
        while let Ok(value) = self.rx.try_recv() {
            newest = Some(value);
        }
        newest
    }

    pub(crate) fn poke(&self) {
        let _ = self.poke_tx.send(());
    }
}

fn next_wait(dirty: bool, wait: Duration, base: Duration, cap: Duration) -> Duration {
    if dirty {
        base
    } else {
        cap.min(wait.saturating_mul(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn periodic_poller_delivers_repeated_results() {
        let count = Arc::new(AtomicUsize::new(0));
        let work_count = count.clone();
        let poller = Poller::spawn(Duration::from_millis(5), move || {
            work_count.fetch_add(1, Ordering::SeqCst) + 1
        });
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) >= 3),
            "expected repeated periodic runs"
        );
        assert!(wait_for(|| poller.latest().is_some()), "expected a result");
    }

    #[test]
    fn poke_wakes_a_long_interval_poller() {
        let count = Arc::new(AtomicUsize::new(0));
        let work_count = count.clone();
        let poller = Poller::spawn(Duration::from_secs(3600), move || {
            work_count.fetch_add(1, Ordering::SeqCst) + 1
        });
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) == 1),
            "first run fires immediately on spawn"
        );
        poller.poke();
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) == 2),
            "poke wakes the parked worker"
        );
    }

    #[test]
    fn drop_terminates_worker_thread() {
        let alive = Arc::new(());
        let held = alive.clone();
        let poller = Poller::spawn(Duration::from_millis(5), move || Arc::strong_count(&held));
        assert!(
            wait_for(|| poller.latest().is_some()),
            "poller produced a result"
        );
        drop(poller);
        assert!(
            wait_for(|| Arc::strong_count(&alive) == 1),
            "worker thread exited and released its closure"
        );
    }

    #[test]
    fn next_wait_resets_on_dirty_and_doubles_to_cap() {
        let cases = [
            (true, 40, 10, 60, 10),
            (false, 10, 10, 60, 20),
            (false, 20, 10, 60, 40),
            (false, 40, 10, 60, 60),
            (false, 60, 10, 60, 60),
        ];
        for (dirty, wait, base, cap, expected) in cases {
            assert_eq!(
                next_wait(
                    dirty,
                    Duration::from_secs(wait),
                    Duration::from_secs(base),
                    Duration::from_secs(cap),
                ),
                Duration::from_secs(expected),
                "dirty: {dirty} wait: {wait}"
            );
        }
    }
}
