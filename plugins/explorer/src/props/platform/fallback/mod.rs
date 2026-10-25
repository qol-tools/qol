use std::fs::Metadata;

use crate::props::Fact;

pub(crate) fn facts(_metadata: &Metadata) -> Vec<Fact> {
    Vec::new()
}
