use anyhow::{Context, Result};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() -> Result<()> {
    let settings_url = qol_conventions::settings_url(PLUGIN_ID);
    qol_apps::desktop_integration::open_with_default_app(&settings_url)
        .context("failed to open settings URL")
}

pub fn driver_installed(driver: &str) -> bool {
    if std::path::Path::new("/sys/module").join(driver).exists() {
        return true;
    }
    let module_name = driver.replace('_', "-");
    std::process::Command::new("modinfo")
        .args(["-n", &module_name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
