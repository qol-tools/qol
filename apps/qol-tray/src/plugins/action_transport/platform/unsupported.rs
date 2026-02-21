use super::super::DaemonActionDispatch;
use std::path::Path;

pub(super) fn dispatch_action(_endpoint: &Path, _action_id: &str) -> DaemonActionDispatch {
    DaemonActionDispatch::Unavailable
}
