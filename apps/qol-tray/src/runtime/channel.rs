use std::time::Duration;

/// A single data source polled by the runtime.
pub(crate) trait Channel: Send {
    /// Poll the OS and update internal state. Returns true if data changed.
    fn poll(&mut self) -> bool;

    /// Minimum poll interval for this channel.
    fn min_interval(&self) -> Duration;

}
