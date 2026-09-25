use crate::chord::{Cap, Glyph, ModifierToken};

pub(super) const JOINER: Option<&str> = None;

pub(super) fn modifier_cap(modifier: ModifierToken) -> Cap {
    Cap::Glyph(match modifier {
        ModifierToken::Ctrl => Glyph::Control,
        ModifierToken::Alt => Glyph::Option,
        ModifierToken::Shift => Glyph::Shift,
        ModifierToken::Platform | ModifierToken::Secondary => Glyph::Command,
    })
}

#[cfg(test)]
mod tests {
    use crate::chord::{caps_for, Cap, Glyph};

    #[test]
    fn modifiers_are_drawn_without_separators() {
        assert_eq!(
            caps_for("platform+w").unwrap(),
            [Cap::Glyph(Glyph::Command), Cap::Text("w".into())]
        );
        assert_eq!(
            caps_for("secondary+shift+z").unwrap(),
            [
                Cap::Glyph(Glyph::Command),
                Cap::Glyph(Glyph::Shift),
                Cap::Text("z".into())
            ]
        );
        assert_eq!(caps_for("escape").unwrap(), [Cap::Text("esc".into())]);
    }

    #[test]
    fn platform_and_secondary_are_the_same_key_here() {
        assert_eq!(caps_for("platform+w"), caps_for("secondary+w"));
    }
}
