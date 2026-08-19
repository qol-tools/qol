#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::Platform;
#[cfg(target_os = "linux")]
pub(crate) use linux::Platform;
#[cfg(target_os = "macos")]
pub(crate) use macos::Platform;
#[cfg(target_os = "windows")]
pub(crate) use windows::Platform;

pub(crate) trait BuildPlatform {
    fn name(&self) -> &'static str;
    fn walk_pace_sleep(&self) -> std::time::Duration {
        std::time::Duration::from_millis(10)
    }
    fn tray_dev_features(&self) -> &'static str;
    fn executable_suffix(&self) -> &'static str;

    fn executable_name(&self, name: &str) -> String {
        let suffix = self.executable_suffix();
        if suffix.is_empty() || name.ends_with(suffix) {
            return name.to_string();
        }
        format!("{name}{suffix}")
    }

    fn plugin_support(&self, manifest: &qol_plugin_api::manifest::PluginManifest) -> PluginSupport {
        if manifest.plugin.supports_current_platform() {
            return PluginSupport {
                supported: true,
                reason: String::new(),
            };
        }
        let declared = manifest
            .plugin
            .platforms
            .as_ref()
            .map(|platforms| platforms.join(", "))
            .unwrap_or_else(|| "none".to_string());
        PluginSupport {
            supported: false,
            reason: format!("Not supported on {} (requires {})", self.name(), declared),
        }
    }
}

pub(crate) struct PluginSupport {
    pub(crate) supported: bool,
    pub(crate) reason: String,
}

#[cfg(test)]
mod tests {
    use super::{BuildPlatform, Platform};

    #[test]
    fn active_platform_has_name() {
        assert!(!Platform.name().is_empty());
    }

    #[test]
    fn executable_name_is_idempotent() {
        let executable = Platform.executable_name("qol-tray");

        assert_eq!(Platform.executable_name(&executable), executable);
    }

    #[test]
    fn tray_dev_features_include_dev() {
        assert!(Platform
            .tray_dev_features()
            .split(',')
            .any(|feature| feature == "dev"));
    }
}
