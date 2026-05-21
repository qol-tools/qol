mod poll;
pub(crate) mod shared;
pub(crate) mod socket;

use std::sync::Arc;

use poll::RuntimeChannels;
use shared::SharedState;

use crate::desktop_state;
use crate::paths::STATE_SOCKET_PATH;

pub struct RuntimeServer {
    _handle: (),
}

impl RuntimeServer {
    pub fn start() -> Self {
        let channels = RuntimeChannels::new(desktop_state::create_shared());
        let initial_monitors = channels.initial_monitors();

        log::info!(
            "Runtime server starting: {} monitors, socket={}",
            initial_monitors.len(),
            STATE_SOCKET_PATH,
        );

        let shared = Arc::new(SharedState::new(initial_monitors));
        super::publisher::install(shared.clone());
        spawn_poll_thread(shared.clone(), channels);
        spawn_socket_thread(shared);
        Self { _handle: () }
    }
}

fn spawn_poll_thread(shared: Arc<SharedState>, channels: RuntimeChannels) {
    std::thread::Builder::new()
        .name("runtime-poll".into())
        .spawn(move || poll::run(shared, channels))
        .expect("failed to spawn runtime poll thread");
}

fn spawn_socket_thread(shared: Arc<SharedState>) {
    std::thread::Builder::new()
        .name("runtime-sock".into())
        .spawn(move || socket::run(shared))
        .expect("failed to spawn runtime socket thread");
}
