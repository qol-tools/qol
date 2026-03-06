#![cfg(feature = "dev")]

mod platform;
mod sampling;
mod snapshot;
mod state;

use crate::daemon::EventBus;
use crate::plugins::PluginManager;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) use snapshot::{PluginCpuEntry, PluginCpuResponse};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const HISTORY_LIMIT: usize = 60;

pub(super) struct DevPluginCpuService {
    state: Arc<Mutex<state::PluginCpuState>>,
}

impl DevPluginCpuService {
    pub(super) fn start(
        plugin_manager: Arc<Mutex<PluginManager>>,
        events: Arc<EventBus>,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            state: Arc::new(Mutex::new(state::PluginCpuState::default())),
        });
        start_sampler(service.state.clone(), plugin_manager, events);
        service
    }

    pub(super) fn snapshot(&self) -> PluginCpuResponse {
        snapshot::build_snapshot(self.state.as_ref(), SAMPLE_INTERVAL, HISTORY_LIMIT)
    }

    pub(super) fn set_monitored_plugins(&self, plugin_ids: Vec<String>) {
        sampling::set_monitored_plugins(self.state.as_ref(), plugin_ids);
    }
}

fn start_sampler(
    state: Arc<Mutex<state::PluginCpuState>>,
    plugin_manager: Arc<Mutex<PluginManager>>,
    events: Arc<EventBus>,
) {
    tokio::spawn(async move {
        loop {
            sampling::sample_once(&state, &plugin_manager, HISTORY_LIMIT);
            sampling::broadcast_snapshot(&state, &events);
            tokio::time::sleep(SAMPLE_INTERVAL).await;
        }
    });
}
