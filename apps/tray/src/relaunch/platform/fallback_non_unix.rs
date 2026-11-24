use std::ffi::OsString;
use std::path::Path;

use super::ReplacePlatformOps;

pub(super) struct Platform;

impl ReplacePlatformOps for Platform {
    fn replace_process(_binary: &Path, _args: &[OsString]) -> String {
        "restarting qol-tray is unavailable on this platform".to_string()
    }
}
