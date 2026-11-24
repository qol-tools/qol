use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::Path;

use super::ReplacePlatformOps;

pub(super) struct Platform;

impl ReplacePlatformOps for Platform {
    fn replace_process(binary: &Path, args: &[OsString]) -> String {
        std::process::Command::new(binary)
            .args(args)
            .exec()
            .to_string()
    }
}
