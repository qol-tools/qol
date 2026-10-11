use super::{BuildPlatform, FileIdentity};

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

    fn file_identity(&self, metadata: &std::fs::Metadata) -> Option<FileIdentity> {
        use std::os::unix::fs::MetadataExt;
        Some(FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            links: metadata.nlink(),
        })
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
