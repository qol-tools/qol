use std::fmt;
use std::ops::{Deref, DerefMut};
use std::panic::Location;
use std::sync::{PoisonError, TryLockError};

use crate::poison::{recover, report, WouldBlock};

#[derive(Debug, Default)]
pub struct Mutex<T: ?Sized>(std::sync::Mutex<T>);

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    pub(crate) lock: &'a std::sync::Mutex<T>,
    pub(crate) guard: std::sync::MutexGuard<'a, T>,
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }

    #[track_caller]
    pub fn into_inner(self) -> T {
        let caller = Location::caller();
        self.0
            .into_inner()
            .unwrap_or_else(|poisoned| recover(caller, poisoned, || {}))
    }
}

impl<T: ?Sized> Mutex<T> {
    #[track_caller]
    pub fn lock(&self) -> MutexGuard<'_, T> {
        let caller = Location::caller();
        let guard = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| recover(caller, poisoned, || self.0.clear_poison()));
        MutexGuard {
            lock: &self.0,
            guard,
        }
    }

    #[track_caller]
    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>, WouldBlock> {
        let caller = Location::caller();
        let guard = match self.0.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => {
                recover(caller, poisoned, || self.0.clear_poison())
            }
            Err(TryLockError::WouldBlock) => return Err(WouldBlock),
        };
        Ok(MutexGuard {
            lock: &self.0,
            guard,
        })
    }

    #[track_caller]
    pub fn get_mut(&mut self) -> &mut T {
        if self.0.is_poisoned() {
            report(Location::caller());
            self.0.clear_poison();
        }
        self.0.get_mut().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for MutexGuard<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.guard, formatter)
    }
}
