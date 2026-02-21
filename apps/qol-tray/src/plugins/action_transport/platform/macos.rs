use super::super::DaemonActionDispatch;
use std::path::Path;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    super::unix_common::dispatch_action(endpoint, action_id)
}
