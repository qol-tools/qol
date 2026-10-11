use super::{BuildPlatform, FileIdentity};

pub(crate) struct Platform;

impl BuildPlatform for Platform {
    fn name(&self) -> &'static str {
        "macos"
    }

    fn walk_pace_sleep(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    fn tray_dev_features(&self) -> &'static str {
        "dev"
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
}
