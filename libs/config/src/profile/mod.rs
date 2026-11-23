mod settings;

use std::path::{Path, PathBuf};

pub use settings::{Scope, SettingsFile, NOTIFICATIONS, THEME};

pub const PROFILE_DIR: &str = "profile";
pub const ACTIVE_PROFILE_FILE: &str = "active";
pub const DEFAULT_PROFILE_NAME: &str = "default";
pub const CORE_SUBDIR: &str = "core";
pub const OS_SUBDIR: &str = "os";
pub const DEVICE_SUBDIR: &str = "device";

pub fn active_profile_name(profile_root: &Path) -> String {
    profile_name_from_marker(std::fs::read_to_string(profile_root.join(ACTIVE_PROFILE_FILE)).ok())
}

pub fn active_profile_dir(profile_root: &Path) -> PathBuf {
    profile_root.join(active_profile_name(profile_root))
}

pub fn active_core_dir(profile_root: &Path) -> PathBuf {
    active_profile_dir(profile_root).join(CORE_SUBDIR)
}

fn profile_name_from_marker(marker: Option<String>) -> String {
    marker
        .map(|raw| raw.trim().to_owned())
        .filter(|name| crate::validate_safe_identifier(name).is_ok())
        .unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_names_the_active_profile_and_a_bad_or_missing_one_means_default() {
        assert_eq!(profile_name_from_marker(None), DEFAULT_PROFILE_NAME);
        for (marker, expected) in [
            ("work\n", "work"),
            ("  work  ", "work"),
            ("../escape", DEFAULT_PROFILE_NAME),
            ("a/b", DEFAULT_PROFILE_NAME),
            ("-work", DEFAULT_PROFILE_NAME),
            ("work profile", DEFAULT_PROFILE_NAME),
            ("", DEFAULT_PROFILE_NAME),
        ] {
            assert_eq!(
                profile_name_from_marker(Some(marker.to_owned())),
                expected,
                "{marker:?}"
            );
        }
    }

    #[test]
    fn the_core_dir_sits_in_the_active_profile() {
        assert_eq!(
            active_core_dir(Path::new("/nowhere/profile")),
            PathBuf::from("/nowhere/profile/default/core")
        );
    }
}
