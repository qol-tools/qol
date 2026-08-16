use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const NOTIFICATIONS_SETTINGS_FILE: &str = "notifications.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct NotificationSettings {
    #[serde(default)]
    use_system_notifications: bool,
}

fn update_settings(update: impl FnOnce(&mut NotificationSettings)) -> Result<()> {
    let path = settings_path()?;
    let mut settings: NotificationSettings = crate::file_io::load_json_or_default(&path)
        .context("failed to load notification settings")?;
    update(&mut settings);
    crate::file_io::write_pretty_json(&path, &settings)
        .context("failed to save notification settings")
}

pub fn set_use_system_notifications(enabled: bool) -> Result<()> {
    update_settings(|settings| settings.use_system_notifications = enabled)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn system_notification_choice_round_trips_and_defaults_to_toast() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(!use_system_notifications());
        set_use_system_notifications(true).unwrap();
        assert!(use_system_notifications());
        set_use_system_notifications(false).unwrap();
        assert!(!use_system_notifications());
    }
}
