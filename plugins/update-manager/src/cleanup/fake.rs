use chrono::NaiveDateTime;

use super::{
    Amount, Found, Item, Leftover, Live, Measure, Noun, Origin, Owner, Remove, Requires, Size,
    State, Survey, Switch, WayBack, Words,
};
use crate::updates::Refusal;

const MINT: Owner = Owner {
    title: "mint's cleanup",
    colophon: "mint's own cleanup. it stays when qol stops.",
};
const KEPT: &str = "Old kernels stay until you remove them.";
const MINT_ON: &str =
    "Weekly and 3 hours after each startup. Also removes other packages nothing needs.";
const BY_HAND_ON: &str =
    "Each cleanup removes them too. The running kernel and the two newest stay.";

const fn kind(
    id: &'static str,
    noun: Noun,
    way_back: WayBack,
    origin: Origin,
    along: &'static [&'static Leftover],
) -> Leftover {
    Leftover {
        id,
        name: id,
        what: id,
        noun,
        requires: Requires::Path("/"),
        measure: Measure::Unknown("a fake kind"),
        remove: Remove::Command {
            argv: &["true"],
            items: false,
        },
        admin: false,
        way_back,
        origin,
        along,
    }
}

pub(crate) static OLD: Leftover = kind(
    "kernels",
    Noun {
        one: "old kernel",
        many: "old kernels",
    },
    WayBack::Keeps,
    Origin::System,
    &[],
);
pub(crate) static BY_HAND: Leftover = kind(
    "kernels_by_hand",
    Noun {
        one: "kernel installed by hand",
        many: "kernels installed by hand",
    },
    WayBack::Keeps,
    Origin::ByHand,
    &[],
);
pub(crate) static ORPHANS: Leftover = kind(
    "orphans",
    Noun {
        one: "package",
        many: "packages",
    },
    WayBack::None,
    Origin::System,
    &[&OLD],
);
pub(crate) static LAST: Leftover = kind(
    "last",
    Noun {
        one: "old version",
        many: "old versions",
    },
    WayBack::Last {
        to: "the version before",
    },
    Origin::System,
    &[],
);
pub(crate) static ALONG: Leftover = kind(
    "along",
    Noun {
        one: "thing",
        many: "things",
    },
    WayBack::None,
    Origin::System,
    &[&LAST],
);
pub(crate) static EXTRA: Leftover = kind(
    "extra",
    Noun {
        one: "extra",
        many: "extras",
    },
    WayBack::None,
    Origin::System,
    &[],
);

fn absent() -> Option<State> {
    None
}

fn accept(_on: bool) -> Result<(), Refusal> {
    Ok(())
}

pub(crate) static MINT_KERNELS: Switch = Switch {
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
    covers: &[&OLD, &ORPHANS],
    requires: Requires::Path("/"),
    read: absent,
    set: accept,
};

pub(crate) static MINT_BY_HAND: Switch = Switch {
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
    covers: &[&BY_HAND],
    requires: Requires::Path("/"),
    read: absent,
    set: accept,
};

pub(crate) static UNATTENDED_KERNELS: Switch = Switch {
    id: "kernels",
    owner: Owner {
        title: "automatic updates",
        colophon: "automatic updates, unattended-upgrades. it stays when qol stops.",
    },
    name: "Old kernels",
    short: "old kernel removal",
    doing: "Removing old kernels",
    when: "removed with updates",
    words: Words {
        on: "When automatic updates run. The running kernel and the two newest stay.",
        idle: "Automatic updates are off here, so this never runs.",
        off: KEPT,
    },
    covers: &[&OLD],
    requires: Requires::Path("/"),
    read: absent,
    set: accept,
};

pub(crate) fn mint() -> Vec<Live> {
    vec![
        Live {
            switch: &MINT_KERNELS,
            state: State::On { idle: false },
        },
        Live {
            switch: &MINT_BY_HAND,
            state: State::Off,
        },
    ]
}

pub(crate) fn found(leftover: &'static Leftover, count: usize, items: &[(&str, u64)]) -> Found {
    let items: Vec<Item> = items
        .iter()
        .map(|(name, bytes)| Item {
            name: (*name).into(),
            bytes: Some(*bytes),
        })
        .collect();
    Found {
        leftover,
        amount: Some(Amount {
            count,
            size: Size::Exact(items.iter().filter_map(|item| item.bytes).sum()),
            items,
        }),
    }
}

pub(crate) fn survey(found: Vec<Found>) -> Survey {
    Survey {
        at: NaiveDateTime::default(),
        found,
    }
}
