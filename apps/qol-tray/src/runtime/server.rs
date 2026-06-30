mod poll;
pub(crate) mod shared;
pub(crate) mod socket;
mod trace;
mod window_list_thread;

use std::sync::Arc;

use poll::RuntimeChannels;
use shared::SharedState;

use crate::desktop_state;
pub struct RuntimeServer {
    _handle: (),
}

impl RuntimeServer {
    pub fn start() -> Self {
        let state_socket_path = crate::dev_generation::state_socket_path();
        let platform = desktop_state::create_shared();
        let channels = RuntimeChannels::new(platform.clone());
        let initial_monitors = channels.initial_monitors();

        log::info!(
            "Runtime server starting: {} monitors, socket={}",
            initial_monitors.len(),
            state_socket_path.display(),
        );

        let shared = Arc::new(SharedState::new(initial_monitors));
        shared.attach_platform(platform.clone());
        super::publisher::install(shared.clone());
        trace::print_monitor_legend();
        spawn_poll_thread(shared.clone(), channels);
        spawn_window_list_thread(shared.clone(), platform);
        spawn_socket_thread(shared, state_socket_path);
        Self { _handle: () }
    }
}

fn spawn_poll_thread(shared: Arc<SharedState>, channels: RuntimeChannels) {
    std::thread::Builder::new()
        .name("runtime-poll".into())
        .spawn(move || poll::run(shared, channels))
        .expect("failed to spawn runtime poll thread");
}

fn spawn_window_list_thread(shared: Arc<SharedState>, platform: desktop_state::SharedPlatform) {
    std::thread::Builder::new()
        .name("runtime-winlist".into())
        .spawn(move || window_list_thread::run(shared, platform))
        .expect("failed to spawn runtime window-list thread");
}

fn spawn_socket_thread(shared: Arc<SharedState>, path: std::path::PathBuf) {
    std::thread::Builder::new()
        .name("runtime-sock".into())
        .spawn(move || socket::run_at(shared, &path))
        .expect("failed to spawn runtime socket thread");
}
