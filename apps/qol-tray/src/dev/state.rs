#![cfg(feature = "dev")]

use std::sync::RwLock;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredPluginInfo {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildResultInfo {
    pub plugin_id: String,
    pub success: bool,
    pub output: String,
    pub skipped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryStatus {
    Idle,
    Discovering,
    Complete,
}

#[derive(Debug, Clone)]
pub struct DiscoveryState {
    pub status: DiscoveryStatus,
    pub plugins: Vec<DiscoveredPluginInfo>,
}

impl Default for DiscoveryState {
    fn default() -> Self {
        Self {
            status: DiscoveryStatus::Idle,
            plugins: vec![],
        }
    }
}

pub struct DevState {
    pub discovery: RwLock<DiscoveryState>,
}

impl DevState {
    pub fn new() -> Self {
        Self {
            discovery: RwLock::new(DiscoveryState::default()),
        }
    }
}

impl Default for DevState {
    fn default() -> Self {
        Self::new()
    }
}

fn try_begin_discovery(state: &DevState) -> bool {
    let mut guard = state.discovery.write().unwrap();
    if guard.status == DiscoveryStatus::Discovering {
        return false;
    }
    guard.status = DiscoveryStatus::Discovering;
    true
}

fn run_discovery(
    state: std::sync::Arc<DevState>,
    events: std::sync::Arc<crate::daemon::EventBus>,
    plugins_dir: std::path::PathBuf,
) {
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
    events.send(crate::daemon::DaemonEvent::DiscoveryComplete { plugins });
}

pub fn start_discovery(
    state: &std::sync::Arc<DevState>,
    events: &std::sync::Arc<crate::daemon::EventBus>,
    plugins_dir: std::path::PathBuf,
) {
    let state = std::sync::Arc::clone(state);
    let events = std::sync::Arc::clone(events);
    if !try_begin_discovery(&state) {
        return;
    }
    events.send(crate::daemon::DaemonEvent::DiscoveryStarted);
    std::thread::spawn(move || run_discovery(state, events, plugins_dir));
}
