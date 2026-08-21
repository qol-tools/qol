pub(in crate::settings_surface) fn show_toast(
    _title: &str,
    _body: &str,
    _level: &str,
    _action: Option<(&str, &str)>,
    _layout: Option<qol_runtime::protocol::NotificationLayout>,
) -> anyhow::Result<bool> {
    Ok(false)
}

pub(in crate::settings_surface) fn request(_plugin_id: &str) -> anyhow::Result<bool> {
    Ok(false)
}

pub(in crate::settings_surface) fn run(_boot: super::super::HostBoot) -> anyhow::Result<()> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

pub(in crate::settings_surface) fn stop() {}

pub(in crate::settings_surface) fn apply_theme(_native: &str, _accent: &str) -> bool {
    false
}

pub(in crate::settings_surface) fn prewarm() {}
