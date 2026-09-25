use crate::chord::{Cap, ModifierToken};

pub(super) use super::text::JOINER;

pub(super) fn modifier_cap(modifier: ModifierToken) -> Cap {
    super::text::word(super::text::shared_word(modifier))
}

#[cfg(test)]
mod tests {
    use crate::chord::{caps_for, Cap, Glyph};

    #[test]
    fn the_meta_key_reads_as_super() {
        assert_eq!(
            caps_for("platform+w").unwrap(),
            [Cap::Text("super+w".into())]
        );
        assert_eq!(
            caps_for("platform+backspace").unwrap(),
            [Cap::Text("super+".into()), Cap::Glyph(Glyph::Backspace)]
        );
    }
}
