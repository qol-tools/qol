use super::BuildPlatform;

pub(crate) struct Platform;

impl BuildPlatform for Platform {
    fn name(&self) -> &'static str {
        "linux"
    }

    fn tray_dev_features(&self) -> &'static str {
        "dev,linux_evdev"
    }

    fn executable_suffix(&self) -> &'static str {
        ""
    }
}
