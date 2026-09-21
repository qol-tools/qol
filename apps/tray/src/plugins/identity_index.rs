use crate::plugins::manifest::{PluginId, PluginUid};
use std::collections::HashMap;

pub struct PluginDisplay {
    pub id: PluginId,
    pub name: String,
}

#[derive(Default)]
pub struct PluginIdentityIndex {
    by_uid: HashMap<PluginUid, PluginDisplay>,
}

impl PluginIdentityIndex {
    pub fn insert(&mut self, uid: PluginUid, id: PluginId, name: String) {
        self.by_uid.insert(uid, PluginDisplay { id, name });
    }

    pub fn display_for(&self, uid: &PluginUid) -> Option<&PluginDisplay> {
        self.by_uid.get(uid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_for_returns_entry_after_insert() {
        let mut index = PluginIdentityIndex::default();
        let uid_a = PluginUid::new("uid-aaa");
        let uid_b = PluginUid::new("uid-bbb");
        index.insert(
            uid_a.clone(),
            PluginId::new("plugin-a"),
            "Plugin A".to_string(),
        );
        index.insert(
            uid_b.clone(),
            PluginId::new("plugin-b"),
            "Plugin B".to_string(),
        );

        let display_a = index.display_for(&uid_a).expect("uid-aaa must be found");
        assert_eq!(display_a.id.as_str(), "plugin-a");
        assert_eq!(display_a.name, "Plugin A");

        let display_b = index.display_for(&uid_b).expect("uid-bbb must be found");
        assert_eq!(display_b.id.as_str(), "plugin-b");
        assert_eq!(display_b.name, "Plugin B");
    }

    #[test]
    fn display_for_returns_none_for_unknown_uid() {
        let index = PluginIdentityIndex::default();
        assert!(index.display_for(&PluginUid::new("nonexistent")).is_none());
    }
}
