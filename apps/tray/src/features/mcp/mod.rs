mod handlers;
mod tool_host;

use crate::plugins::PluginManager;
use axum::Router;
use std::sync::{Arc, Mutex};

pub fn router(plugin_manager: Arc<Mutex<PluginManager>>) -> Router {
    handlers::router_with_host(Arc::new(tool_host::PluginToolHost::new(plugin_manager)))
}
