use std::fs::Metadata;

use crate::props::Fact;

pub(crate) fn facts(_metadata: &Metadata) -> Vec<Fact> {
    Vec::new()
}

pub(crate) fn volume(_metadata: &Metadata) -> Option<u64> {
    None
}
