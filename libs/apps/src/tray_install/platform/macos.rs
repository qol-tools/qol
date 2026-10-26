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
        InstallProfile::Development => development_dir(),
    }
}

pub(crate) fn install_dir_for_home(profile: InstallProfile, home: &Path) -> io::Result<PathBuf> {
    match profile {
        InstallProfile::Production => Ok(home
            .join("Applications")
            .join(format!("{}.app", qol_conventions::TRAY_DISPLAY_NAME))
            .join("Contents")
            .join("MacOS")),
        InstallProfile::Development => Ok(development_dir_at(
            &home
                .join("Library")
                .join("Application Support")
                .join(qol_config::NAMESPACE),
        )),
    }
}

fn development_dir() -> io::Result<PathBuf> {
    let data = qol_config::data_dir()
        .ok_or_else(|| io::Error::other("Could not determine qol data directory"))?;
    Ok(development_dir_at(&data))
}

fn development_dir_at(data: &Path) -> PathBuf {
    data.join("dev").join("bin")
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
