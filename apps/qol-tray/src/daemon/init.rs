use std::sync::Arc;

use super::EventBus;
#[cfg(feature = "dev")]
use super::{DaemonEvent, DaemonState, DiscoveredPluginInfo, DiscoveryStatus};

#[derive(Clone)]
pub struct Daemon {
    #[cfg(feature = "dev")]
    pub state: Arc<DaemonState>,
    pub events: Arc<EventBus>,
}

impl Daemon {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "dev")]
            state: Arc::new(DaemonState::new()),
            events: Arc::new(EventBus::new()),
        }
    }

    #[cfg(feature = "dev")]
    pub fn start_discovery(&self, plugins_dir: std::path::PathBuf) {
        let state = Arc::clone(&self.state);
        let events = Arc::clone(&self.events);

        {
            let guard = state.discovery.read().unwrap();
            if guard.status == DiscoveryStatus::Discovering {
                return;
            }
        }

        std::thread::spawn(move || {
            {
                let mut guard = state.discovery.write().unwrap();
                guard.status = DiscoveryStatus::Discovering;
            }
            events.send(DaemonEvent::DiscoveryStarted);

            let config = crate::dev::DevConfig::load().unwrap_or_default();
            let discovered = crate::dev::discover_plugins(&config, &plugins_dir);

            let plugins: Vec<DiscoveredPluginInfo> = discovered
                .into_iter()
                .map(|p| DiscoveredPluginInfo {
                    id: p.id,
                    name: p.name,
                    path: p.path,
                })
                .collect();

            {
                let mut guard = state.discovery.write().unwrap();
                guard.status = DiscoveryStatus::Complete;
                guard.plugins = plugins.clone();
            }
            events.send(DaemonEvent::DiscoveryComplete { plugins });
        });
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}
