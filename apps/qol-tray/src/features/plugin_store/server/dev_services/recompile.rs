#![cfg(feature = "dev")]

mod restart_schedule;
mod result;
mod start;

use std::sync::{Arc, Mutex};

use crate::dev::BuildResult;

use super::super::dev_runtime::DevRuntimeService;
use super::super::restart::RestartPort;
use super::super::types::AppState;

pub(super) fn queue_self_recompile(state: &AppState) -> Result<(), &'static str> {
    start::queue_self_recompile(state)
}

type RecompileResult = Result<BuildResult, tokio::task::JoinError>;

struct SelfRecompileTask {
    events: Arc<crate::daemon::EventBus>,
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: Arc<DevRuntimeService>,
    restart: Arc<dyn RestartPort>,
}

impl SelfRecompileTask {
    fn from_state(state: &AppState) -> Self {
        Self {
            events: state.daemon.events.clone(),
            plugin_manager: state.plugin_manager.clone(),
            runtime: state.runtime.clone(),
            restart: state.restart.clone(),
        }
    }
}
