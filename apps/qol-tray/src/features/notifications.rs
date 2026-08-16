use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const NOTIFICATIONS_SETTINGS_FILE: &str = "notifications.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct NotificationSettings {
    #[serde(default)]
    use_system_notifications: bool,
}

pub fn use_system_notifications() -> bool {
    settings()
        .map(|settings| settings.use_system_notifications)
        .unwrap_or(false)
}

fn settings() -> Result<NotificationSettings> {
    let path = settings_path()?;
    crate::file_io::load_json_or_default(&path).context("failed to load notification settings")
}

fn settings_path() -> Result<std::path::PathBuf> {
    crate::paths::shared_config_dir().map(|dir| dir.join(NOTIFICATIONS_SETTINGS_FILE))
}
