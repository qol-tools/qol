#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::windows::{com, registry, wide};
