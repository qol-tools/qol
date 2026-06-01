use std::path::Path;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use qol_runtime::protocol::RuntimeEvent;
use qol_runtime::MonitorBounds;

use super::server::shared::SharedState;
use super::server::socket;

#[derive(Clone)]
pub struct TestRuntime {
    shared: Arc<SharedState>,
}

impl TestRuntime {
    pub fn new(monitors: Vec<MonitorBounds>) -> Self {
        Self {
            shared: Arc::new(SharedState::new(monitors)),
        }
    }

    pub fn spawn_listener(&self, path: &Path) -> JoinHandle<()> {
        let shared = Arc::clone(&self.shared);
        let path = path.to_path_buf();
        thread::Builder::new()
            .name("test-runtime-sock".into())
            .spawn(move || socket::run_at(shared, &path))
            .expect("failed to spawn test runtime socket thread")
    }

    pub fn publish(&self, events: &[RuntimeEvent]) {
        self.shared.publish(events);
    }
}
