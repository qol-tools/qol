use std::path::Path;

use super::RestartPlatformOps;

pub(super) struct Platform;

impl RestartPlatformOps for Platform {
    fn binary_name() -> &'static str {
        "qol-tray"
    }

    fn exec_restart(_binary: &Path) -> Result<(), String> {
        Err("self-recompile restart is unavailable on this platform".to_string())
    }
}
