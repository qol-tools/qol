#[cfg(not(windows))]
mod port_probe;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use port_probe as active;
#[cfg(windows)]
use windows as active;

pub use active::{claim, InstanceLock};
