use crate::chord::{Cap, ModifierToken};

pub(super) const JOINER: Option<&str> = Some("+");

pub(super) fn shared_word(modifier: ModifierToken) -> &'static str {
    match modifier {
        ModifierToken::Ctrl | ModifierToken::Secondary => "ctrl",
        ModifierToken::Alt => "alt",
        ModifierToken::Shift => "shift",
        ModifierToken::Platform => "super",
    }
}

pub(super) fn word(word: &str) -> Cap {
    Cap::Text(word.to_owned())
}

#[cfg(test)]
mod tests {
    use crate::chord::{caps_for, Cap, Glyph};

    fn text(value: &str) -> Cap {
        Cap::Text(value.to_owned())
    }

    #[test]
    fn renders_lowercase_modifiers_joined_with_plus() {
        assert_eq!(caps_for("secondary+z").unwrap(), [text("ctrl+z")]);
        assert_eq!(caps_for("alt+s").unwrap(), [text("alt+s")]);
        assert_eq!(
            caps_for("secondary+shift+z").unwrap(),
            [text("ctrl+shift+z")]
        );
        assert_eq!(
            caps_for("shift+enter").unwrap(),
            [text("shift+"), Cap::Glyph(Glyph::Enter)]
        );
    }

    #[test]
    fn platform_and_secondary_are_different_keys_here() {
        assert_ne!(caps_for("platform+w"), caps_for("secondary+w"));
    }
}
