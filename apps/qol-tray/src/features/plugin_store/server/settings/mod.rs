mod hotkey_handlers;
mod http_json;
mod media_apps_handlers;
mod media_cover_handlers;
mod media_icon_handlers;
mod plugin_config_handlers;
mod shortcut_handlers;

use axum::{
    routing::{get, post},
    Router,
};

use super::types::AppState;

pub(super) use hotkey_handlers::get_hotkey_errors;
pub(super) use hotkey_handlers::get_hotkeys;
pub(super) use hotkey_handlers::open_hotkeys_file;
pub(super) use hotkey_handlers::open_shortcuts_file;
pub(super) use hotkey_handlers::set_hotkeys;
pub(super) use media_apps_handlers::list_apps;
pub(super) use media_cover_handlers::serve_cover;
pub(super) use media_icon_handlers::serve_icon;
pub(super) use plugin_config_handlers::get_plugin_config;
pub(super) use plugin_config_handlers::get_plugin_config_form;
pub(super) use plugin_config_handlers::set_plugin_config;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/cover/{id}", get(serve_cover))
        .route("/icon/{bundle_id}", get(serve_icon))
        .route("/apps", get(list_apps))
        .route("/plugins/{id}/config", get(get_plugin_config))
        .route("/plugins/{id}/config-form", get(get_plugin_config_form))
        .route(
            "/plugins/{id}/config",
            axum::routing::put(set_plugin_config),
        )
        .route("/hotkeys", get(get_hotkeys))
        .route("/hotkeys", axum::routing::put(set_hotkeys))
        .route("/hotkeys/errors", get(get_hotkey_errors))
        .route("/hotkeys/open-file", post(open_hotkeys_file))
        .route("/shortcuts/open-file", post(open_shortcuts_file))
        .route("/shortcuts", get(shortcut_handlers::list_shortcuts))
        .route("/shortcuts", post(shortcut_handlers::create_shortcut))
        .route(
            "/shortcuts/{id}",
            axum::routing::put(shortcut_handlers::update_shortcut),
        )
        .route(
            "/shortcuts/{id}",
            axum::routing::delete(shortcut_handlers::delete_shortcut),
        )
        .route("/shortcuts/{id}/run", post(shortcut_handlers::run_shortcut))
        .merge(crate::features::profile::http::routes())
}
