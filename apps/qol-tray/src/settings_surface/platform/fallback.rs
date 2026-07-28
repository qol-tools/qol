pub(in crate::settings_surface) fn request(_plugin_id: &str) -> anyhow::Result<bool> {
    Ok(false)
}

pub(in crate::settings_surface) fn run(_plugin_id: String) -> anyhow::Result<()> {
    anyhow::bail!("native settings surfaces are unsupported on this platform")
}

pub(in crate::settings_surface) fn stop() {}
