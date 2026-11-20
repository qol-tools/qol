use super::{KERNELS, KERNELS_BY_HAND, ORPHANS};
use crate::cleanup::{Owner, Requires, Switch, Words};

mod mint;
mod unattended;

const MINT: Owner = Owner {
    title: "mint's cleanup",
    colophon: "mint's own cleanup. it stays when qol stops.",
};
const UNATTENDED: Owner = Owner {
    title: "automatic updates",
    colophon: "automatic updates, unattended-upgrades. it stays when qol stops.",
};
const MINT_AUTOMATION: Requires = Requires::Path("/usr/bin/mintupdate-automation");
const KEPT: &str = "Old kernels stay until you remove them.";
const MINT_ON: &str =
    "Weekly and 3 hours after each startup. Also removes other packages nothing needs.";
const BY_HAND_ON: &str =
    "Each cleanup removes them too. The running kernel and the two newest stay.";

static SWITCHES: [Switch; 3] = [
    Switch {
        id: "kernels",
        owner: MINT,
        name: "Old kernels and leftovers",
        short: "old kernel removal",
        doing: "Removing old kernels",
        when: "removed weekly",
        words: Words {
            on: MINT_ON,
            idle: MINT_ON,
            off: KEPT,
        },
        covers: &[&KERNELS, &ORPHANS],
        requires: MINT_AUTOMATION,
        read: mint::removals,
        set: mint::set_removals,
    },
    Switch {
        id: "by_hand",
        owner: MINT,
        name: "Include kernels installed by hand",
        short: "hand-installed kernel removal",
        doing: "Removing kernels installed by hand",
        when: "removed weekly",
        words: Words {
            on: BY_HAND_ON,
            idle: BY_HAND_ON,
            off: "A kernel installed from Mint's Update Manager window is never removed.",
        },
        covers: &[&KERNELS_BY_HAND],
        requires: MINT_AUTOMATION,
        read: mint::by_hand,
        set: mint::set_by_hand,
    },
    Switch {
        id: "kernels",
        owner: UNATTENDED,
        name: "Old kernels",
        short: "old kernel removal",
        doing: "Removing old kernels",
        when: "removed with updates",
        words: Words {
            on: "When automatic updates run. The running kernel and the two newest stay.",
            idle: "Automatic updates are off here, so this never runs.",
            off: KEPT,
        },
        covers: &[&KERNELS],
        requires: Requires::Path("/usr/bin/unattended-upgrade"),
        read: unattended::read,
        set: unattended::set,
    },
];

pub(crate) fn switches() -> &'static [Switch] {
    &SWITCHES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::AutoKey;

    #[test]
    fn journal_ids_from_before_the_table_still_decode() {
        for id in ["kernels", "by_hand"] {
            let key: AutoKey =
                serde_json::from_str(&format!("\"{id}\"")).expect("a known switch id");
            assert_eq!(key.id(), id);
            assert!(key.removes());
            assert_eq!(
                serde_json::to_string(&key).expect("encode"),
                format!("\"{id}\"")
            );
        }
        assert!(serde_json::from_str::<AutoKey>("\"nothing\"").is_err());
    }

    #[test]
    fn switches_sharing_an_id_are_alternatives_for_the_same_kinds() {
        let mint = &SWITCHES[0];
        let unattended = &SWITCHES[2];
        assert_eq!(mint.id, unattended.id);
        assert!(unattended
            .covers
            .iter()
            .all(|kind| mint.covers.iter().any(|known| known.id == kind.id)));
    }
}
