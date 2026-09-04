mod core_config;
mod hotkey_handlers;
mod http_json;
mod media_apps_handlers;
mod media_cover_handlers;
mod media_icon_handlers;
mod notifications_handlers;
mod plugin_config_handlers;
mod shortcut_handlers;
mod theme_handlers;

use axum::{
    routing::{get, post},
    Router,
};

use super::types::AppState;

pub(super) use hotkey_handlers::cancel_hotkey_recording;
pub(super) use hotkey_handlers::capture_hotkey_recording;
pub(super) use hotkey_handlers::get_hotkey_errors;
pub(super) use hotkey_handlers::get_hotkeys;
pub(super) use hotkey_handlers::open_hotkeys_file;
pub(super) use hotkey_handlers::open_shortcuts_file;
pub(super) use hotkey_handlers::set_hotkeys;
pub(super) use hotkey_handlers::start_hotkey_recording;
pub(super) use media_apps_handlers::list_apps;
pub(super) use media_cover_handlers::serve_cover;
pub(super) use media_icon_handlers::serve_icon;
pub(super) use plugin_config_handlers::get_plugin_config;
pub(super) use plugin_config_handlers::get_plugin_config_form;
#[cfg(feature = "dev")]
pub(super) use plugin_config_handlers::notify_plugin_reload;
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
        .route(
            "/hotkeys/recording/{session_id}",
            post(start_hotkey_recording).delete(cancel_hotkey_recording),
        )
        .route(
            "/hotkeys/recording/{session_id}/capture",
            post(capture_hotkey_recording),
        )
        .route("/hotkeys/open-file", post(open_hotkeys_file))
        .route("/shortcuts/open-file", post(open_shortcuts_file))
        .route("/theme/accent", get(theme_handlers::get_theme_accent))
        .route(
            "/theme/accent",
            axum::routing::put(theme_handlers::set_theme_accent),
        )
        .route("/theme", get(theme_handlers::get_theme))
        .route("/theme", axum::routing::put(theme_handlers::set_theme))
        .route("/native-theme", get(theme_handlers::get_native_theme))
        .route(
            "/native-theme",
            axum::routing::put(theme_handlers::set_native_theme),
        )
        .route("/core/queries/{query}", get(theme_handlers::get_core_query))
        .route("/core/config", get(core_config::get_core_config))
        .route(
            "/core/config",
            axum::routing::put(core_config::set_core_config),
        )
        .route(
            "/notifications",
            get(notifications_handlers::get_notifications),
        )
        .route(
            "/notifications",
            axum::routing::put(notifications_handlers::set_notifications),
        )
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
