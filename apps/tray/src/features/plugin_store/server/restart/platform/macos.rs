use super::RestartPlatformOps;

pub(super) struct Platform;

impl RestartPlatformOps for Platform {
    fn binary_name() -> &'static str {
        "qol-tray"
    }
}
