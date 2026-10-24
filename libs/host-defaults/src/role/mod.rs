#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    FolderOpen,
    FolderReveal,
    SlotLauncher,
    SlotWindowSwitcher,
    SlotScreenshot,
    SlotClipboardHistory,
    SlotDictation,
}

impl Role {
    pub const ALL: [Role; 7] = [
        Role::FolderOpen,
        Role::FolderReveal,
        Role::SlotLauncher,
        Role::SlotWindowSwitcher,
        Role::SlotScreenshot,
        Role::SlotClipboardHistory,
        Role::SlotDictation,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Role::FolderOpen => "folder.open",
            Role::FolderReveal => "folder.reveal",
            Role::SlotLauncher => "slot.launcher",
            Role::SlotWindowSwitcher => "slot.window_switcher",
            Role::SlotScreenshot => "slot.screenshot",
            Role::SlotClipboardHistory => "slot.clipboard_history",
            Role::SlotDictation => "slot.dictation",
        }
    }

    pub fn parse(id: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.id() == id)
    }

    pub fn kind_noun(self) -> &'static str {
        match self {
            Role::FolderOpen | Role::FolderReveal => "file manager",
            Role::SlotLauncher => "app launcher",
            Role::SlotWindowSwitcher => "window switcher",
            Role::SlotScreenshot => "screenshot tool",
            Role::SlotClipboardHistory => "clipboard history",
            Role::SlotDictation => "dictation",
        }
    }

    pub fn is_slot(self) -> bool {
        match self {
            Role::FolderOpen | Role::FolderReveal => false,
            Role::SlotLauncher
            | Role::SlotWindowSwitcher
            | Role::SlotScreenshot
            | Role::SlotClipboardHistory
            | Role::SlotDictation => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_nouns_match_the_catalog() {
        let cases = [
            (Role::FolderOpen, "folder.open", "file manager", false),
            (Role::FolderReveal, "folder.reveal", "file manager", false),
            (Role::SlotLauncher, "slot.launcher", "app launcher", true),
            (
                Role::SlotWindowSwitcher,
                "slot.window_switcher",
                "window switcher",
                true,
            ),
            (
                Role::SlotScreenshot,
                "slot.screenshot",
                "screenshot tool",
                true,
            ),
            (
                Role::SlotClipboardHistory,
                "slot.clipboard_history",
                "clipboard history",
                true,
            ),
            (Role::SlotDictation, "slot.dictation", "dictation", true),
        ];
        assert_eq!(cases.len(), Role::ALL.len());
        for (role, id, noun, slot) in cases {
            assert_eq!(role.id(), id, "id of {role:?}");
            assert_eq!(role.kind_noun(), noun, "noun of {role:?}");
            assert_eq!(role.is_slot(), slot, "slot flag of {role:?}");
        }
    }

    #[test]
    fn every_role_round_trips_through_its_id() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.id()), Some(role));
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids = Role::ALL.map(Role::id).to_vec();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Role::ALL.len());
    }

    #[test]
    fn unknown_ids_do_not_parse() {
        for id in [
            "",
            "folder",
            "Folder.Open",
            "slot.window_snap",
            " folder.open",
        ] {
            assert_eq!(Role::parse(id), None, "id: {id:?}");
        }
    }
}
