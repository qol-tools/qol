use crate::chord::ModifierToken;

pub(super) struct Platform;

impl super::ChordStyle for Platform {
    fn modifier_label(&self, modifier: ModifierToken) -> &'static str {
        match modifier {
            ModifierToken::Ctrl => "\u{2303}",
            ModifierToken::Alt => "\u{2325}",
            ModifierToken::Shift => "\u{21E7}",
            ModifierToken::Platform | ModifierToken::Secondary => "\u{2318}",
        }
    }

    fn join(&self, mods: &[&str], key: &str) -> String {
        format!("{}{key}", mods.concat())
    }
}

#[cfg(test)]
mod tests {
    use crate::chord::label_for;

    #[test]
    fn renders_glyphs_without_separators() {
        assert_eq!(label_for("platform+w").unwrap(), "\u{2318}W");
        assert_eq!(label_for("secondary+z").unwrap(), "\u{2318}Z");
        assert_eq!(label_for("alt+s").unwrap(), "\u{2325}S");
        assert_eq!(label_for("secondary+shift+z").unwrap(), "\u{21E7}\u{2318}Z");
        assert_eq!(label_for("platform+backspace").unwrap(), "\u{2318}\u{232B}");
        assert_eq!(label_for("escape").unwrap(), "\u{238B}");
    }

    #[test]
    fn platform_and_secondary_are_the_same_key_here() {
        assert_eq!(label_for("platform+w"), label_for("secondary+w"));
    }
}
