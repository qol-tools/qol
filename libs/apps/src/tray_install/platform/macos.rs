use std::io;
use std::path::{Path, PathBuf};

use crate::tray_install::InstallProfile;

pub(crate) const BINARY_FILENAME: &str = "qol-tray";

pub(crate) fn install_dir(profile: InstallProfile) -> io::Result<PathBuf> {
    match profile {
        InstallProfile::Production => {
            let home = dirs::home_dir()
                .ok_or_else(|| io::Error::other("Could not determine home directory"))?;
            install_dir_for_home(profile, &home)
        }
        InstallProfile::Development => {
            let data = dirs::data_local_dir()
                .or_else(dirs::data_dir)
                .ok_or_else(|| io::Error::other("Could not determine data directory"))?;
            Ok(development_dir(&data))
        }
    }
}

pub(crate) fn install_dir_for_home(profile: InstallProfile, home: &Path) -> io::Result<PathBuf> {
    match profile {
        InstallProfile::Production => Ok(home
            .join("Applications")
            .join(format!("{}.app", qol_conventions::TRAY_DISPLAY_NAME))
            .join("Contents")
            .join("MacOS")),
        InstallProfile::Development => Ok(development_dir(
            &home.join("Library").join("Application Support"),
        )),
    }
}

fn development_dir(data: &Path) -> PathBuf {
    data.join(qol_conventions::artifact::TRAY_PACKAGE_NAME)
        .join("dev")
        .join("bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_development_profile_skips_the_app_bundle() {
        let home = Path::new("/Users/v");
        assert_eq!(
            install_dir_for_home(InstallProfile::Production, home).unwrap(),
            PathBuf::from("/Users/v/Applications/QoL Tray.app/Contents/MacOS")
        );
        assert_eq!(
            install_dir_for_home(InstallProfile::Development, home).unwrap(),
            PathBuf::from("/Users/v/Library/Application Support/qol-tray/dev/bin")
        );
    }
}
