use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_ENCRYPTED,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
};

use crate::props::Fact;

const NAMED: [(u32, &str); 6] = [
    (FILE_ATTRIBUTE_READONLY, "Read-only"),
    (FILE_ATTRIBUTE_HIDDEN, "Hidden"),
    (FILE_ATTRIBUTE_SYSTEM, "System"),
    (FILE_ATTRIBUTE_ARCHIVE, "Archive"),
    (FILE_ATTRIBUTE_COMPRESSED, "Compressed"),
    (FILE_ATTRIBUTE_ENCRYPTED, "Encrypted"),
];

pub(crate) fn volume(_metadata: &Metadata) -> Option<u64> {
    None
}

pub(crate) fn facts(metadata: &Metadata) -> Vec<Fact> {
    vec![Fact::new(
        "Attributes",
        attributes(metadata.file_attributes()),
    )]
}

fn attributes(bits: u32) -> String {
    let named: Vec<&str> = NAMED
        .iter()
        .filter(|(mask, _)| bits & mask != 0)
        .map(|(_, label)| *label)
        .collect();
    if named.is_empty() {
        "None".to_string()
    } else {
        named.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::{attributes, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY};

    #[test]
    fn attributes_list_the_named_flags() {
        assert_eq!(attributes(0), "None");
        assert_eq!(attributes(FILE_ATTRIBUTE_READONLY), "Read-only");
        assert_eq!(
            attributes(FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN),
            "Read-only, Hidden"
        );
    }
}
