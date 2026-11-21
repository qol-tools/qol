use std::panic::Location;
use std::sync::{RwLockReadGuard, RwLockWriteGuard, TryLockError};

use crate::poison::{recover, WouldBlock};

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
        let caller = Location::caller();
        self.0
            .read()
            .unwrap_or_else(|poisoned| recover(caller, poisoned, || self.0.clear_poison()))
    }

    #[track_caller]
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        let caller = Location::caller();
        self.0
            .write()
            .unwrap_or_else(|poisoned| recover(caller, poisoned, || self.0.clear_poison()))
    }

    #[track_caller]
    pub fn try_read(&self) -> Result<RwLockReadGuard<'_, T>, WouldBlock> {
        let caller = Location::caller();
        match self.0.try_read() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => {
                Ok(recover(caller, poisoned, || self.0.clear_poison()))
            }
            Err(TryLockError::WouldBlock) => Err(WouldBlock),
        }
    }

    #[track_caller]
    pub fn try_write(&self) -> Result<RwLockWriteGuard<'_, T>, WouldBlock> {
        let caller = Location::caller();
        match self.0.try_write() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => {
                Ok(recover(caller, poisoned, || self.0.clear_poison()))
            }
            Err(TryLockError::WouldBlock) => Err(WouldBlock),
        }
    }
}
