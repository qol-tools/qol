use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::hash) struct FileIdentity;

pub(in crate::hash) fn file_identity(_metadata: &std::fs::Metadata) -> FileIdentity {
    FileIdentity
}
