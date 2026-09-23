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
