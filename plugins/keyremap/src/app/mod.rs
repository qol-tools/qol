mod daemon;

use crate::config;
use crate::control::Host;

pub(crate) fn host() -> Host {
    Host {
        plugin_id: qol_config::plugin_id_from_env(crate::PLUGIN_ID),
        read_config: config::load_config,
        inspect_config: config::inspect_config,
        flip_enabled,
        listen: daemon::start_listener,
        send_reload: daemon::send_reload,
        send_kill: daemon::send_kill,
        cleanup: daemon::cleanup,
        open_settings,
    }
}

pub(crate) fn open_settings() -> std::io::Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(crate::PLUGIN_ID)
}

fn flip_enabled() -> Result<bool, &'static str> {
    let enabled = !config::load_config().enabled;
    let mut stored = qol_runtime::plugin_config::load_json()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(fields) = stored.as_object_mut() else {
        return Err("keyremap: stored config is not an object");
    };
    fields.insert("enabled".to_string(), serde_json::Value::Bool(enabled));
    if !qol_runtime::plugin_config::save(&stored) {
        return Err("keyremap: failed to persist the new remapping state");
    }
    Ok(enabled)
}
