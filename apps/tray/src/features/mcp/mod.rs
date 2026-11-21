mod handlers;
mod tool_host;

use crate::plugins::SharedPluginManager;
use axum::Router;
use std::sync::Arc;

pub fn router(plugin_manager: SharedPluginManager) -> Router {
    handlers::router_with_host(Arc::new(tool_host::PluginToolHost::new(plugin_manager)))
}
