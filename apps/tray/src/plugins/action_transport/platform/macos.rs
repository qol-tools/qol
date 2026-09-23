use super::{unix, ActionTransportPlatform};
use crate::plugins::action_transport::DaemonActionDispatch;
use std::path::Path;
use std::time::Duration;

pub(super) struct Platform;

impl ActionTransportPlatform for Platform {
    fn default_io_timeout() -> Duration {
        unix::DEFAULT_IO_TIMEOUT
    }

    fn dispatch_action(
        endpoint: &Path,
        action_id: &str,
        input: &serde_json::Value,
        timeout: Duration,
    ) -> DaemonActionDispatch {
        unix::dispatch_action(endpoint, action_id, input, timeout)
    }

    fn can_connect(endpoint: &Path) -> bool {
        unix::can_connect(endpoint)
    }
}
