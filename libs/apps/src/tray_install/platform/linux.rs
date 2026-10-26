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
        InstallProfile::Production => Ok(home.join(".local").join("bin")),
        InstallProfile::Development => Ok(development_dir_at(
            &home
                .join(".local")
                .join("share")
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
    fn the_development_profile_keeps_its_own_binary_directory() {
        let home = Path::new("/home/u");
        assert_eq!(
            install_dir_for_home(InstallProfile::Production, home).unwrap(),
            PathBuf::from("/home/u/.local/bin")
        );
        assert_eq!(
            install_dir_for_home(InstallProfile::Development, home).unwrap(),
            PathBuf::from("/home/u/.local/share/qol-tray/dev/bin")
        );
    }
}
