use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::hash) struct FileIdentity {
    device: u64,
    inode: u64,
    change_seconds: i64,
    change_nanoseconds: i64,
    mode: u32,
}

pub(in crate::hash) fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        change_seconds: metadata.ctime(),
        change_nanoseconds: metadata.ctime_nsec(),
        mode: metadata.mode(),
    }
}
