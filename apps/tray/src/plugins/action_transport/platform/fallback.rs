use super::ActionTransportPlatform;
use qol_runtime::local_ipc::LocalStream;
use std::path::Path;
use std::time::Duration;

pub(super) struct Platform;

impl ActionTransportPlatform for Platform {
    fn default_io_timeout() -> Duration {
        Duration::from_secs(10)
    }

    fn connect(_endpoint: &Path, _timeout: Duration) -> Result<LocalStream, ()> {
        Err(())
    }
}
