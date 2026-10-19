use std::sync::Arc;

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

use super::super::channels::window_list::WindowListChannel;
use super::super::Channel;
use super::state_store::SharedState;
use crate::desktop_state::SharedPlatform;

pub(super) fn run(shared: Arc<SharedState>, platform: SharedPlatform) {
    let mut channel = WindowListChannel::new(platform);
    let interval = channel.min_interval();
    loop {
        shared.wait_for_window_list_subscriber();
        if !shared.has_event_subscribers(RuntimeEventKind::WindowListChanged) {
            continue;
        }
        if !channel.poll() {
            std::thread::sleep(interval);
            continue;
        }
        shared.publish(&[RuntimeEvent::WindowListChanged]);
        std::thread::sleep(interval);
    }
}
