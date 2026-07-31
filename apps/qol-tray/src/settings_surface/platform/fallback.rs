pub(in crate::settings_surface) fn request(_plugin_id: &str) -> anyhow::Result<bool> {
    Ok(false)
}

pub(in crate::settings_surface) fn run(_boot: super::super::HostBoot) -> anyhow::Result<()> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

pub(in crate::settings_surface) fn stop() {}

pub(in crate::settings_surface) fn prewarm() {}
