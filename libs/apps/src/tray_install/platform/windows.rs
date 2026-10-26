use std::io;
use std::path::{Path, PathBuf};

use crate::tray_install::InstallProfile;

pub(crate) const BINARY_FILENAME: &str = "qol-tray.exe";

pub(crate) fn install_dir(profile: InstallProfile) -> io::Result<PathBuf> {
    match profile {
        InstallProfile::Production => {
            let local_data = dirs::data_local_dir()
                .ok_or_else(|| io::Error::other("Could not determine local data directory"))?;
            Ok(production_dir(&local_data))
        }
        InstallProfile::Development => development_dir(),
    }
}

pub(crate) fn install_dir_for_home(profile: InstallProfile, home: &Path) -> io::Result<PathBuf> {
    let local_data = home.join("AppData").join("Local");
    match profile {
        InstallProfile::Production => Ok(production_dir(&local_data)),
        InstallProfile::Development => {
            Ok(development_dir_at(&local_data.join(qol_config::NAMESPACE)))
        }
    }
}

fn production_dir(local_data: &Path) -> PathBuf {
    local_data.join("Programs").join("qol-tray").join("bin")
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
