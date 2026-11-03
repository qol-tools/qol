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
        InstallProfile::Production => Ok(bundle_executables(&home.join("Applications"))),
        InstallProfile::Development => Ok(development_dir_at(
            &home
                .join("Library")
                .join("Application Support")
                .join(qol_config::NAMESPACE),
        )),
    }
}

pub(crate) fn system_install_dirs() -> Vec<PathBuf> {
    vec![bundle_executables(Path::new("/Applications"))]
}

fn bundle_executables(applications: &Path) -> PathBuf {
    applications
        .join(format!("{}.app", qol_conventions::TRAY_DISPLAY_NAME))
        .join("Contents")
        .join("MacOS")
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

    #[test]
    fn the_system_applications_folder_is_the_other_production_install() {
        assert_eq!(
            system_install_dirs(),
            [PathBuf::from("/Applications/QoL Tray.app/Contents/MacOS")]
        );
    }
}
