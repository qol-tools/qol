mod poll;
pub(crate) mod push_status;
pub(crate) mod socket;
pub(crate) mod state_store;
mod trace;
mod window_list_thread;

use std::sync::Arc;

use poll::RuntimeChannels;
use state_store::SharedState;

use crate::desktop_state;
pub struct RuntimeServer {
    state_socket: StateSocketStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSocketStatus {
    Listening,
    Unsupported,
    BindFailed,
}

impl StateSocketStatus {
    pub fn blocks_generation_handoff(self) -> bool {
        matches!(self, Self::BindFailed)
    }
}

impl RuntimeServer {
    pub fn state_socket(&self) -> StateSocketStatus {
        self.state_socket
    }

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
        let state_socket = bind_state_socket(shared, &state_socket_path);
        Self { state_socket }
    }

    pub fn bind_public_socket() -> bool {
        let path = crate::dev_generation::GenerationContext::stable().state_socket_path();
        let Some(shared) = super::publisher::shared() else {
            log::error!(
                "Cannot bind promoted runtime socket at {}: runtime publisher is not installed",
                path.display()
            );
            return false;
        };
        !bind_state_socket(shared, &path).blocks_generation_handoff()
    }
}

fn bind_state_socket(shared: Arc<SharedState>, path: &std::path::Path) -> StateSocketStatus {
    match socket::bind_at(path) {
        socket::BindOutcome::Bound(listener) => {
            spawn_socket_listener_thread(shared, listener);
            StateSocketStatus::Listening
        }
        socket::BindOutcome::Unsupported => StateSocketStatus::Unsupported,
        socket::BindOutcome::Failed => StateSocketStatus::BindFailed,
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

fn spawn_socket_listener_thread(shared: Arc<SharedState>, listener: socket::Listener) {
    std::thread::Builder::new()
        .name("runtime-sock-promoted".into())
        .spawn(move || socket::run_listener(shared, listener))
        .expect("failed to spawn promoted runtime socket thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_failed_bind_blocks_a_generation_handoff() {
        assert!(StateSocketStatus::BindFailed.blocks_generation_handoff());
        assert!(!StateSocketStatus::Listening.blocks_generation_handoff());
        assert!(
            !StateSocketStatus::Unsupported.blocks_generation_handoff(),
            "a platform without a state socket has nothing to bind, so the handoff \
             must not be treated as broken there",
        );
    }

    #[test]
    fn binding_an_unwritable_parent_reports_failure_not_unsupported() {
        let tmp = tempfile::TempDir::new().unwrap();
        let blocking_file = tmp.path().join("sockets");
        std::fs::write(&blocking_file, b"not a directory").unwrap();
        let path = blocking_file.join("runtime-state.sock");

        let shared = Arc::new(SharedState::new(Vec::new()));

        assert_eq!(
            bind_state_socket(shared, &path),
            StateSocketStatus::BindFailed
        );
    }

    #[test]
    fn a_missing_socket_directory_is_created_rather_than_failing_the_boot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp
            .path()
            .join("runtime")
            .join("sockets")
            .join("state.sock");

        let shared = Arc::new(SharedState::new(Vec::new()));

        assert_eq!(
            bind_state_socket(shared, &path),
            StateSocketStatus::Listening,
            "a first boot has no sockets directory yet, and refusing the handoff \
             over that would strand the dev loop",
        );
    }

    #[test]
    fn binding_a_usable_path_reports_listening() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("runtime-state.sock");

        let shared = Arc::new(SharedState::new(Vec::new()));

        assert_eq!(
            bind_state_socket(shared, &path),
            StateSocketStatus::Listening
        );
    }
}
