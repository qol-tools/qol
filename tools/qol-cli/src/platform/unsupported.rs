use super::{OpenPathOutcome, PlatformOps};
use anyhow::{anyhow, Result};
use std::env;
use std::path::{Path, PathBuf};

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn exe_name(&self, name: &str) -> String {
        name.to_string()
    }

    fn stop_qol_tray(&self) -> Result<()> {
        Ok(())
    }

    fn qol_tray_running(&self) -> bool {
        false
    }

    fn copy_to_clipboard(&self, _text: &str) -> Result<()> {
        Err(anyhow!("clipboard not supported on this platform"))
    }

    fn available_memory_mb(&self) -> Option<u64> {
        None
    }

    fn home_dir(&self) -> Option<PathBuf> {
        env::var_os("HOME").map(PathBuf::from)
    }

    fn open_path(&self, _path: &Path) -> Result<OpenPathOutcome> {
        Err(anyhow!("opening paths is not supported on this platform"))
    }

    fn supports_immutable_payload_build(&self) -> bool {
        false
    }

    fn open_text_file(&self, path: &Path) -> bool {
        qol_apps::desktop_integration::open_with_default_app(path).is_ok()
    }
}
