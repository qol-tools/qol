use super::{unix, ActionTransportPlatform};
use qol_runtime::local_ipc::LocalStream;
use std::path::Path;
use std::time::Duration;

pub(super) struct Platform;

impl ActionTransportPlatform for Platform {
    fn default_io_timeout() -> Duration {
        unix::DEFAULT_IO_TIMEOUT
    }

    fn connect(endpoint: &Path, timeout: Duration) -> Result<LocalStream, ()> {
        unix::connect(endpoint, timeout)
    }
}
