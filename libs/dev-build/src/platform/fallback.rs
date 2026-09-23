use super::BuildPlatform;

pub(crate) struct Platform;

impl BuildPlatform for Platform {
    fn name(&self) -> &'static str {
        std::env::consts::OS
    }

    fn tray_dev_features(&self) -> &'static str {
        "dev"
    }

    fn executable_suffix(&self) -> &'static str {
        std::env::consts::EXE_SUFFIX
    }
}
