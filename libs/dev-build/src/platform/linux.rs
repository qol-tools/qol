use super::BuildPlatform;

pub(crate) struct Platform;

impl BuildPlatform for Platform {
    fn tray_dev_features(&self) -> &'static str {
        "dev,linux_evdev"
    }

    fn executable_suffix(&self) -> &'static str {
        ""
    }

    fn read_preserving_access_time(&self, path: &std::path::Path) -> Option<String> {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOATIME)
            .open(path)
        else {
            return super::read_restoring_access_time(path);
        };
        let mut content = String::new();
        file.take(64).read_to_string(&mut content).ok()?;
        Some(content)
    }
}
