use std::io;
use std::path::{Path, PathBuf};

use crate::tray_install::InstallProfile;

pub(crate) const BINARY_FILENAME: &str = "qol-tray.exe";

pub(crate) fn install_dir(profile: InstallProfile) -> io::Result<PathBuf> {
    let local_data = dirs::data_local_dir()
        .ok_or_else(|| io::Error::other("Could not determine local data directory"))?;
    Ok(install_dir_in(profile, &local_data))
}

pub(crate) fn install_dir_for_home(profile: InstallProfile, home: &Path) -> io::Result<PathBuf> {
    Ok(install_dir_in(profile, &home.join("AppData").join("Local")))
}

fn install_dir_in(profile: InstallProfile, local_data: &Path) -> PathBuf {
    match profile {
        InstallProfile::Production => local_data.join("Programs").join("qol-tray").join("bin"),
        InstallProfile::Development => local_data
            .join(qol_conventions::artifact::TRAY_PACKAGE_NAME)
            .join("dev")
            .join("bin"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_development_profile_keeps_its_own_binary_directory() {
        let home = Path::new(r"C:\Users\v");
        assert_eq!(
            install_dir_for_home(InstallProfile::Production, home).unwrap(),
            PathBuf::from(r"C:\Users\v\AppData\Local\Programs\qol-tray\bin")
        );
        assert_eq!(
            install_dir_for_home(InstallProfile::Development, home).unwrap(),
            PathBuf::from(r"C:\Users\v\AppData\Local\qol-tray\dev\bin")
        );
    }
}
