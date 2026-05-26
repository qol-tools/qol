use std::sync::Arc;

use qol_runtime::protocol::RuntimeEvent;

use super::super::channels::window_list::WindowListChannel;
use super::super::Channel;
use super::shared::SharedState;
use crate::desktop_state::SharedPlatform;

pub(super) fn run(shared: Arc<SharedState>, platform: SharedPlatform) {
    let mut channel = WindowListChannel::new(platform);
    let interval = channel.min_interval();
    loop {
        std::thread::sleep(interval);
        if !channel.poll() {
            continue;
        }
        if !shared.has_subscribers() {
            continue;
        }
        shared.publish(&[RuntimeEvent::WindowListChanged]);
    }
}
