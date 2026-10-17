use crate::normalized::{ResolvedField, ResolvedSection};
use crate::validation::ValidationError;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::current_platform;
#[cfg(target_os = "linux")]
pub use linux::current_platform;
#[cfg(target_os = "macos")]
pub use macos::current_platform;
#[cfg(target_os = "windows")]
pub use windows::current_platform;

/// Operating system names a `platforms` list may hold, in `plugin.toml` and
/// in `qol-config.toml` fields and sections.
pub const PLATFORMS: [&str; 3] = ["linux", "macos", "windows"];

/// Whether a `platforms` list includes the running OS. Omitted means every OS.
pub fn supports_current_platform(platforms: Option<&[String]>) -> bool {
    platforms.is_none_or(|platforms| {
        platforms
            .iter()
            .any(|platform| platform == current_platform())
    })
}

impl ResolvedField {
    /// Whether the settings panel shows this field on the running OS.
    pub fn on_this_platform(&self) -> bool {
        supports_current_platform(self.platforms.as_deref())
    }
}

impl ResolvedSection {
    /// Whether the settings panel shows this section on the running OS.
    pub fn on_this_platform(&self) -> bool {
        supports_current_platform(self.platforms.as_deref())
    }
}

pub(crate) fn validate_platforms(
    path: &str,
    platforms: Option<&[String]>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(platforms) = platforms else {
        return;
    };
    if platforms.is_empty() {
        errors.push(ValidationError::new(
            format!("{path}.platforms"),
            "must name at least one platform",
        ));
    }
    for (index, platform) in platforms.iter().enumerate() {
        if PLATFORMS.contains(&platform.as_str()) {
            continue;
        }
        errors.push(ValidationError::new(
            format!("{path}.platforms[{index}]"),
            format!("unknown platform {platform}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms_list_gates_on_the_running_os() {
        let here = vec![current_platform().to_string()];
        let elsewhere: Vec<String> = PLATFORMS
            .iter()
            .filter(|platform| **platform != current_platform())
            .map(|platform| platform.to_string())
            .collect();
        assert!(supports_current_platform(None));
        assert!(supports_current_platform(Some(&here)));
        assert!(!supports_current_platform(Some(&elsewhere)));
    }
}
