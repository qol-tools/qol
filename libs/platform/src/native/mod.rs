#[cfg(windows)]
mod platform;

#[cfg(windows)]
pub use platform::windows::{com, hooks, input, registry, security, session, shell, wide};
