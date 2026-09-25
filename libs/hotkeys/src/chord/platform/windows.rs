use crate::chord::{Cap, ModifierToken};

pub(super) use super::text::JOINER;

pub(super) fn modifier_cap(modifier: ModifierToken) -> Cap {
    match modifier {
        ModifierToken::Platform => super::text::word("win"),
        other => super::text::word(super::text::shared_word(other)),
    }
}
