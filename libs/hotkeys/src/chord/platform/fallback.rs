use crate::chord::{Cap, ModifierToken};

pub(super) use super::text::JOINER;

pub(super) fn modifier_cap(modifier: ModifierToken) -> Cap {
    super::text::word(super::text::shared_word(modifier))
}
