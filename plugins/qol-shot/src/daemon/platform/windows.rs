use std::sync::mpsc::Sender;
use std::time::Duration;

use super::super::Command;

pub(crate) fn start_listener(_tx: Sender<Command>) -> bool {
    false
}

pub(crate) fn cleanup() {}

pub(crate) fn wait_and_send_action(_action: &str, _timeout: Duration) -> bool {
    false
}
