use anyhow::Result;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod imp {
    use anyhow::{anyhow, Result};
    use std::path::{Path, PathBuf};

    pub fn install_dir() -> Result<PathBuf> {
        Err(anyhow!("Unsupported platform"))
    }

    pub fn autostart_path() -> Result<PathBuf> {
        Err(anyhow!("Unsupported platform"))
    }

    pub fn write_autostart_entry(_: &Path) -> Result<()> {
        Err(anyhow!("Unsupported platform"))
    }

    pub fn start_now(_: &Path) -> Result<()> {
        Ok(())
    }
}

pub fn binary_filename() -> String {
    if cfg!(target_os = "windows") {
        "qol-tray.exe".to_string()
    } else {
        "qol-tray".to_string()
    }
}

pub fn install_dir() -> Result<PathBuf> {
    imp::install_dir()
}

pub fn autostart_path() -> Result<PathBuf> {
    imp::autostart_path()
}

pub fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    imp::write_autostart_entry(binary_path)
}

pub fn start_now(binary_path: &Path) -> Result<()> {
    imp::start_now(binary_path)
}
