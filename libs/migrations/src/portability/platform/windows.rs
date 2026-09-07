use super::PathPlatform;

pub(super) struct Platform;

impl PathPlatform for Platform {
    fn path_limit(&self) -> usize {
        260
    }

    fn os_bucket(&self) -> &'static str {
        "windows"
    }
}
