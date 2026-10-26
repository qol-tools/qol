mod platform;

use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallProfile {
    #[default]
    Production,
    Development,
}

impl InstallProfile {
    pub fn binary_filename(self) -> &'static str {
        platform::BINARY_FILENAME
    }

    pub fn install_dir(self) -> io::Result<PathBuf> {
        platform::install_dir(self)
    }

    pub fn install_dir_for_home(self, home: &Path) -> io::Result<PathBuf> {
        platform::install_dir_for_home(self, home)
    }

    pub fn installed_binary(self) -> io::Result<PathBuf> {
        Ok(self.install_dir()?.join(self.binary_filename()))
    }
}

pub fn binary_filename() -> &'static str {
    platform::BINARY_FILENAME
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
mod tests {
    use super::*;

    #[test]
    fn the_development_profile_never_shares_the_production_directory() {
        let home = Path::new("/home/u");
        let production = InstallProfile::Production
            .install_dir_for_home(home)
            .unwrap();
        let development = InstallProfile::Development
            .install_dir_for_home(home)
            .unwrap();
        assert_ne!(production, development);
        assert!(!development.starts_with(&production));
        assert!(!production.starts_with(&development));
    }

    #[test]
    fn both_profiles_share_the_binary_filename() {
        assert_eq!(
            InstallProfile::Production.binary_filename(),
            InstallProfile::Development.binary_filename()
        );
        assert_eq!(
            InstallProfile::Production.binary_filename(),
            binary_filename()
        );
    }

    #[test]
    fn the_development_profile_stays_inside_the_home_directory() {
        let home = Path::new("/home/u");
        let development = InstallProfile::Development
            .install_dir_for_home(home)
            .unwrap();
        assert!(development.starts_with(home), "{}", development.display());
        assert!(development.ends_with("bin"), "{}", development.display());
    }
}
