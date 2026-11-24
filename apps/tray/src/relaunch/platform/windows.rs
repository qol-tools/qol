use std::ffi::OsString;
use std::path::Path;

use super::ReplacePlatformOps;

pub(super) struct Platform;

impl ReplacePlatformOps for Platform {
    fn replace_process(binary: &Path, args: &[OsString]) -> String {
        super::super::spawn_successor_and_exit(binary, args).to_string()
    }
}
