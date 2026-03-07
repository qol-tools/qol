mod github_token_handlers;
mod hotkey_handlers;
mod media_apps_handlers;
mod media_cover_handlers;
mod media_icon_handlers;
mod plugin_config_handlers;

use axum::{
    routing::{get, post},
    Router,
};

use super::types::AppState;

pub(super) use github_token_handlers::delete_github_token;
pub(super) use github_token_handlers::get_token_status;
pub(super) use github_token_handlers::set_github_token;
pub(super) use hotkey_handlers::get_hotkeys;
pub(super) use hotkey_handlers::set_hotkeys;
pub(super) use media_apps_handlers::list_apps;
pub(super) use media_cover_handlers::serve_cover;
pub(super) use media_icon_handlers::serve_icon;
pub(super) use plugin_config_handlers::get_plugin_config;
pub(super) use plugin_config_handlers::set_plugin_config;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/cover/{id}", get(serve_cover))
        .route("/icon/{bundle_id}", get(serve_icon))
        .route("/apps", get(list_apps))
        .route("/plugins/{id}/config", get(get_plugin_config))
        .route(
            "/plugins/{id}/config",
            axum::routing::put(set_plugin_config),
        )
        .route("/github-token", get(get_token_status))
        .route("/github-token", post(set_github_token))
        .route("/github-token", axum::routing::delete(delete_github_token))
        .route("/hotkeys", get(get_hotkeys))
        .route("/hotkeys", axum::routing::put(set_hotkeys))
}
