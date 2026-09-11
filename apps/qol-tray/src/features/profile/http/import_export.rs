use axum::{extract::State, http::StatusCode, response::IntoResponse, response::Response, Json};

pub(crate) async fn export_config(
    State(state): State<super::ProfileHttpState>,
) -> impl IntoResponse {
    let plugins = export_plugins(&state);
    let bundle = crate::features::profile::core::build_export_bundle(
        chrono::Local::now().to_rfc3339(),
        plugins,
    );
    match bundle {
        Ok(bundle) => Json(bundle).into_response(),
        Err(error) => export_server_error(error),
    }
}

pub(crate) async fn import_config(
    State(state): State<super::ProfileHttpState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let bundle =
        match super::parse_json_body::<crate::features::profile::core::ProfileImportBundle>(body) {
            Ok(bundle) => bundle,
            Err(response) => return *response,
        };
    match import_bundle(&state, bundle).await {
        Ok(result) => Json(result).into_response(),
        Err(response) => *response,
    }
}

async fn import_bundle(
    state: &super::ProfileHttpState,
    bundle: crate::features::profile::core::ProfileImportBundle,
) -> Result<crate::features::profile::core::ApplyProfileResult, Box<Response>> {
    let result = crate::features::profile::core::apply_import_bundle(&state.plugins_dir, &bundle)
        .await
        .map_err(|error| Box::new(import_server_error(error)))?;
    super::reload_after_profile_apply(state);
    Ok(result)
}

fn export_plugins(
    state: &super::ProfileHttpState,
) -> Vec<crate::features::profile::core::PluginLockEntry> {
    let stored = crate::features::profile::core::load_plugins_lock()
        .unwrap_or_else(|_| crate::features::profile::core::PluginsLock::empty());
    let Ok(manager) = state.plugin_manager.lock() else {
        return crate::features::profile::core::PluginsLock::for_export(
            &state.plugins_dir,
            std::iter::empty::<&crate::plugins::Plugin>(),
            &stored,
        )
        .plugins;
    };
    crate::features::profile::core::PluginsLock::for_export(
        &state.plugins_dir,
        manager.plugins(),
        &stored,
    )
    .plugins
}

fn export_server_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to export profile: {error:#}"),
    )
        .into_response()
}

fn import_server_error(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Failed to import profile: {error:#}"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::export_plugins;
    use std::sync::{Arc, Mutex};

    #[test]
    fn export_plugins_refines_stored_entries_when_manager_lock_is_poisoned() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _path_root = crate::paths::push_test_path_root(root.path());
        let plugins_dir = crate::paths::shared_config_dir().unwrap().join("plugins");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        crate::features::profile::core::ensure_profile_dirs().unwrap();
        let plugin_dir = plugins_dir.join("plugin-lights");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
[plugin]
id = "plugin-lights"
uid = "u-real-0001"
name = "plugin-lights"
description = "Test plugin"
version = "1.0.0"

[menu]
label = "plugin-lights"
items = []
"#,
        )
        .unwrap();
        crate::features::profile::core::save_plugins_lock(
            &crate::features::profile::core::PluginsLock {
                version: crate::features::profile::core::CURRENT_PROFILE_VERSION,
                plugins: vec![crate::features::profile::core::PluginLockEntry {
                    uid: crate::plugins::PluginUid::new("plugin-lights"),
                    id: "plugin-lights".to_string(),
                    repo_url: "https://example.invalid/plugin-lights.git".to_string(),
                    version: "1.0.0".to_string(),
                    platforms: None,
                }],
            },
        )
        .unwrap();

        let manager = Arc::new(Mutex::new(crate::plugins::PluginManager::new()));
        let poisoned = Arc::clone(&manager);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _held = poisoned.lock().unwrap();
            panic!("poison the plugin manager mutex");
        }));

        let state = super::super::ProfileHttpState {
            plugins_dir: plugins_dir.clone(),
            plugin_manager: manager,
            daemon: crate::daemon::Daemon::new(),
            sync_service: Arc::new(
                crate::features::profile::sync::SyncService::new(plugins_dir.clone()).unwrap(),
            ),
        };

        let exported = export_plugins(&state);

        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].id, "plugin-lights");
        assert_eq!(
            exported[0].uid,
            crate::plugins::PluginUid::new("u-real-0001")
        );
    }
}
