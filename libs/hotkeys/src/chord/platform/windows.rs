use crate::chord::ModifierToken;

pub(super) struct Platform;

impl super::ChordStyle for Platform {
    fn modifier_label(&self, modifier: ModifierToken) -> &'static str {
        match modifier {
            ModifierToken::Platform => "Win",
            other => super::text::shared_label(other),
        }
    }

    fn join(&self, mods: &[&str], key: &str) -> String {
        super::text::join(mods, key)
    }
}
