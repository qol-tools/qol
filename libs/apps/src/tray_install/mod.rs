mod platform;

use std::io;
use std::path::{Path, PathBuf};

pub fn binary_filename() -> &'static str {
    platform::BINARY_FILENAME
}

pub fn install_dir() -> io::Result<PathBuf> {
    platform::install_dir()
}

pub fn install_dir_for_home(home: &Path) -> io::Result<PathBuf> {
    platform::install_dir_for_home(home)
}

pub fn installed_binary() -> io::Result<PathBuf> {
    Ok(install_dir()?.join(binary_filename()))
}
