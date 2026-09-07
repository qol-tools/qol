use std::sync::mpsc::Sender;
use std::time::Duration;

mod platform;

pub enum Command {
    Screenshot,
    Preview,
    Reload,
    Theme,
    Cli(String),
    Kill,
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    platform::start_listener(tx)
}

pub fn cleanup() {
    platform::cleanup();
}

pub fn wait_and_send_action(action: &str, timeout: Duration) -> bool {
    platform::wait_and_send_action(action, timeout)
}
