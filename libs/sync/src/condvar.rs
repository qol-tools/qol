use std::panic::Location;
use std::sync::WaitTimeoutResult;
use std::time::{Duration, Instant};

use crate::mutex::{relock, MutexGuard};
use crate::poison::release;

#[derive(Debug, Default)]
pub struct Condvar(std::sync::Condvar);

impl Condvar {
    pub const fn new() -> Self {
        Self(std::sync::Condvar::new())
    }

    #[track_caller]
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let caller = Location::caller();
        let MutexGuard { lock, guard } = guard;
        let guard = match self.0.wait(guard) {
            Ok(guard) => guard,
            Err(poisoned) => {
                release(caller, poisoned, || lock.clear_poison());
                relock(lock, caller)
            }
        };
        MutexGuard { lock, guard }
    }

    #[track_caller]
    pub fn wait_while<'a, T, F>(
        &self,
        guard: MutexGuard<'a, T>,
        mut condition: F,
    ) -> MutexGuard<'a, T>
    where
        F: FnMut(&mut T) -> bool,
    {
        let caller = Location::caller();
        let MutexGuard { lock, mut guard } = guard;
        loop {
            match self.0.wait_while(guard, &mut condition) {
                Ok(guard) => return MutexGuard { lock, guard },
                Err(poisoned) => {
                    release(caller, poisoned, || lock.clear_poison());
                    guard = relock(lock, caller);
                }
            }
        }
    }

    #[track_caller]
    pub fn wait_timeout<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
    ) -> (MutexGuard<'a, T>, WaitTimeoutResult) {
        let caller = Location::caller();
        let MutexGuard { lock, guard } = guard;
        let (guard, result) = match self.0.wait_timeout(guard, timeout) {
            Ok(woken) => woken,
            Err(poisoned) => {
                let result = poisoned.get_ref().1;
                release(caller, poisoned, || lock.clear_poison());
                (relock(lock, caller), result)
            }
        };
        (MutexGuard { lock, guard }, result)
    }

    #[track_caller]
    pub fn wait_timeout_while<'a, T, F>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
        mut condition: F,
    ) -> (MutexGuard<'a, T>, WaitTimeoutResult)
    where
        F: FnMut(&mut T) -> bool,
    {
        let caller = Location::caller();
        let started = Instant::now();
        let MutexGuard { lock, mut guard } = guard;
        loop {
            let remaining = timeout.saturating_sub(started.elapsed());
            match self.0.wait_timeout_while(guard, remaining, &mut condition) {
                Ok((guard, result)) => return (MutexGuard { lock, guard }, result),
                Err(poisoned) => {
                    release(caller, poisoned, || lock.clear_poison());
                    guard = relock(lock, caller);
                }
            }
        }
    }

    pub fn notify_one(&self) {
        self.0.notify_one();
    }

    pub fn notify_all(&self) {
        self.0.notify_all();
    }
}
