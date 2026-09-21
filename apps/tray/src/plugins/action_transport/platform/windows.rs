use super::ActionTransportPlatform;
use crate::plugins::action_transport::DaemonActionDispatch;
use std::path::Path;
use std::time::Duration;

pub(super) struct Platform;

impl ActionTransportPlatform for Platform {
    fn default_io_timeout() -> Duration {
        Duration::from_secs(10)
    }

    fn dispatch_action(
        _endpoint: &Path,
        _action_id: &str,
        _input: &serde_json::Value,
        _timeout: Duration,
    ) -> DaemonActionDispatch {
        DaemonActionDispatch::Unavailable
    }

    fn can_connect(_endpoint: &Path) -> bool {
        false
    }
}
