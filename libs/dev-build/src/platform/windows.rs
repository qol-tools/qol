use super::BuildPlatform;

pub(crate) struct Platform;

impl BuildPlatform for Platform {
    fn tray_dev_features(&self) -> &'static str {
        "dev"
    }

    fn executable_suffix(&self) -> &'static str {
        ".exe"
    }
}
