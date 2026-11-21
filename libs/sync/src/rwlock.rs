use std::panic::Location;
use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use crate::poison::{acquire, try_acquire, WouldBlock};

#[derive(Debug, Default)]
pub struct RwLock<T: ?Sized>(std::sync::RwLock<T>);

impl<T> RwLock<T> {
    pub const fn new(value: T) -> Self {
        Self(std::sync::RwLock::new(value))
    }
}

impl<T: ?Sized> RwLock<T> {
    #[track_caller]
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        acquire(
            Location::caller(),
            || self.0.read(),
            || self.0.clear_poison(),
        )
    }

    #[track_caller]
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        acquire(
            Location::caller(),
            || self.0.write(),
            || self.0.clear_poison(),
        )
    }

    #[track_caller]
    pub fn try_read(&self) -> Result<RwLockReadGuard<'_, T>, WouldBlock> {
        try_acquire(
            Location::caller(),
            || self.0.try_read(),
            || self.0.clear_poison(),
        )
    }

    #[track_caller]
    pub fn try_write(&self) -> Result<RwLockWriteGuard<'_, T>, WouldBlock> {
        try_acquire(
            Location::caller(),
            || self.0.try_write(),
            || self.0.clear_poison(),
        )
    }
}
