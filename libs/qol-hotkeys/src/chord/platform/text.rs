use crate::chord::ModifierToken;

pub(super) fn shared_label(modifier: ModifierToken) -> &'static str {
    match modifier {
        ModifierToken::Ctrl | ModifierToken::Secondary => "Ctrl",
        ModifierToken::Alt => "Alt",
        ModifierToken::Shift => "Shift",
        ModifierToken::Platform => "Super",
    }
}

pub(super) fn join(mods: &[&str], key: &str) -> String {
    let mut parts = mods.iter().map(|part| part.to_string()).collect::<Vec<_>>();
    parts.push(key.to_string());
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use crate::chord::label_for;

    #[test]
    fn renders_text_modifiers_joined_with_plus() {
        assert_eq!(label_for("secondary+z").unwrap(), "Ctrl+Z");
        assert_eq!(label_for("alt+s").unwrap(), "Alt+S");
        assert_eq!(label_for("secondary+shift+z").unwrap(), "Shift+Ctrl+Z");
        assert_eq!(label_for("escape").unwrap(), "\u{238B}");
    }

    #[test]
    fn platform_and_secondary_are_different_keys_here() {
        assert_ne!(label_for("platform+w"), label_for("secondary+w"));
    }
}
