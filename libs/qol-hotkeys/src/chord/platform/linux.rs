use crate::chord::ModifierToken;

pub(super) struct Platform;

impl super::ChordStyle for Platform {
    fn modifier_label(&self, modifier: ModifierToken) -> &'static str {
        super::text::shared_label(modifier)
    }

    fn join(&self, mods: &[&str], key: &str) -> String {
        super::text::join(mods, key)
    }
}

#[cfg(test)]
mod tests {
    use crate::chord::label_for;

    #[test]
    fn the_meta_key_reads_as_super() {
        assert_eq!(label_for("platform+w").unwrap(), "Super+W");
        assert_eq!(label_for("platform+backspace").unwrap(), "Super+\u{232B}");
    }
}
