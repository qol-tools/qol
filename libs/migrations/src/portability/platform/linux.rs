use super::PathPlatform;

pub(super) struct Platform;

impl PathPlatform for Platform {
    fn path_limit(&self) -> usize {
        4096
    }

    fn os_bucket(&self) -> &'static str {
        "linux"
    }
}
