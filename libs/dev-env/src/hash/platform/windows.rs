use serde::{Deserialize, Serialize};
use std::os::windows::fs::MetadataExt;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::hash) struct FileIdentity {
    creation: u64,
    last_write: u64,
    attributes: u32,
}

pub(in crate::hash) fn file_identity(metadata: &std::fs::Metadata) -> FileIdentity {
    FileIdentity {
        creation: metadata.creation_time(),
        last_write: metadata.last_write_time(),
        attributes: metadata.file_attributes(),
    }
}
