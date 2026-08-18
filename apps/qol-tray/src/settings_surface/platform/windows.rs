pub(in crate::settings_surface) fn show_toast(
    _title: &str,
    _body: &str,
    _level: &str,
    _action: Option<(&str, &str)>,
    _layout: Option<qol_runtime::protocol::NotificationLayout>,
) -> anyhow::Result<bool> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

pub(in crate::settings_surface) fn request(_plugin_id: &str) -> anyhow::Result<bool> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

pub(in crate::settings_surface) fn run(_boot: super::super::HostBoot) -> anyhow::Result<()> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

pub(in crate::settings_surface) fn stop() {}

pub(in crate::settings_surface) fn prewarm() {}
