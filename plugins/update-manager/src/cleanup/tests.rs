use std::collections::BTreeSet;

use super::fake::{self, found, ALONG, BY_HAND, LAST, LISTED, OLD, ORPHANS, SCRIPTED};
use super::*;

fn word(text: &str) -> bool {
    !text.trim().is_empty()
}

fn id(text: &str) -> bool {
    word(text) && text.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

#[test]
fn every_kind_fills_every_field_its_type_asks_for() {
    let mut ids = BTreeSet::new();
    for leftover in leftovers() {
        assert!(id(leftover.id), "kind id `{}`", leftover.id);
        assert!(ids.insert(leftover.id), "kind `{}` twice", leftover.id);
        for text in [
            leftover.name,
            leftover.what,
            leftover.noun.one,
            leftover.noun.many,
        ] {
            assert!(word(text), "kind `{}` has an empty word", leftover.id);
        }
        match leftover.measure {
            Measure::Files { dir, suffix } => {
                assert!(dir.starts_with('/'));
                assert!(word(suffix));
            }
            Measure::Command { argv, .. } => assert!(argv.iter().all(|arg| word(arg))),
            Measure::Unknown(reason) => assert!(word(reason)),
        }
        match leftover.remove {
            Remove::Command(argv) => assert!(!argv.is_empty()),
            Remove::Packages => {}
            Remove::Script(script) => assert!(word(script)),
        }
        if let WayBack::Last { to } = leftover.way_back {
            assert!(word(to));
        }
        for along in leftover.along {
            assert!(
                leftovers().iter().any(|known| known.id == along.id),
                "kind `{}` takes along `{}`, which is not in the table",
                leftover.id,
                along.id
            );
            assert!(along.along.is_empty(), "`{}` chains along", leftover.id);
        }
    }
}

#[test]
fn every_switch_fills_its_words_and_covers_kinds_of_its_own_table() {
    for switch in platform::switches() {
        assert!(id(switch.id), "switch id `{}`", switch.id);
        for text in [
            switch.owner.title,
            switch.owner.colophon,
            switch.name,
            switch.short,
            switch.doing,
            switch.when,
            switch.words.on,
            switch.words.idle,
            switch.words.off,
        ] {
            assert!(word(text), "switch `{}` has an empty word", switch.id);
        }
        assert!(!switch.covers.is_empty(), "switch `{}`", switch.id);
        assert!(
            crate::updates::AutoKey::INSTALLS
                .iter()
                .all(|key| key.id() != switch.id),
            "switch `{}` would decode as an install switch",
            switch.id
        );
        for kind in switch.covers {
            assert!(
                leftovers().iter().any(|known| known.id == kind.id),
                "switch `{}` covers `{}`, which is not in the table",
                switch.id,
                kind.id
            );
        }
    }
}

#[test]
fn the_total_counts_a_package_two_kinds_list_once() {
    let survey = fake::survey(vec![
        found(&OLD, 1, &[("linux-image-1", 10), ("linux-modules-1", 20)]),
        found(
            &ORPHANS,
            3,
            &[
                ("linux-image-1", 10),
                ("linux-modules-1", 20),
                ("libfoo", 5),
            ],
        ),
        Found {
            leftover: &LAST,
            amount: Some(Amount::unknown(Why::NeedsAdmin)),
        },
        Found {
            leftover: &BY_HAND,
            amount: None,
        },
    ]);
    assert_eq!(
        survey.total(),
        Total {
            bytes: 35,
            estimate: false,
            unknown: 1,
        }
    );
    let lump = fake::survey(vec![
        found(&OLD, 1, &[("linux-image-1", 10)]),
        Found {
            leftover: &ORPHANS,
            amount: Some(Amount {
                count: 2,
                present: 2,
                size: Size::Estimate {
                    bytes: 100,
                    why: "apt's count",
                },
                items: vec![
                    Item {
                        name: "linux-image-1".into(),
                        bytes: None,
                    },
                    Item {
                        name: "libfoo".into(),
                        bytes: None,
                    },
                ],
            }),
        },
    ]);
    assert_eq!(
        lump.total(),
        Total {
            bytes: 100,
            estimate: true,
            unknown: 0,
        }
    );
}

#[test]
fn the_last_way_back_and_kinds_installed_by_hand_ask_first() {
    assert!(!asks(&OLD));
    assert!(!asks(&ORPHANS));
    assert!(asks(&LAST));
    assert!(asks(&BY_HAND));
    assert!(asks(&ALONG), "taking an asking kind along asks too");
    assert_eq!(
        consent::why(&ALONG).as_deref(),
        Some("take last along, which are the last way back to the version before")
    );
    for kind in [&LAST, &BY_HAND, &ALONG] {
        assert_eq!(consent::allows(kind, Consent::Asked), Ok(()), "{}", kind.id);
        assert!(consent::allows(kind, Consent::All).is_err(), "{}", kind.id);
        let refused = remove(&found(kind, 1, &[("x", 1)]), Consent::All);
        assert!(
            matches!(refused, Err(Refusal::Failed(ref reason)) if reason.contains("only when asked for by name")),
            "{} went into remove all: {refused:?}",
            kind.id
        );
    }
    let absent = Found {
        leftover: &OLD,
        amount: None,
    };
    assert!(remove(&absent, Consent::Asked).is_err());
}

#[test]
fn a_kind_that_could_not_be_listed_is_not_reported_removed() {
    let failed = Found {
        leftover: &LISTED,
        amount: Some(Amount::unknown(Why::Failed("snap list failed".into()))),
    };
    let refused = remove(&failed, Consent::All);
    assert!(
        matches!(refused, Err(Refusal::Failed(ref reason)) if reason.contains("snap list failed")),
        "{refused:?}"
    );
    assert_eq!(remove(&found(&LISTED, 0, &[]), Consent::All), Ok(()));
    let scripted = Found {
        leftover: &SCRIPTED,
        amount: Some(Amount::unknown(Why::Failed("snap list failed".into()))),
    };
    assert!(
        matches!(remove(&scripted, Consent::All), Err(Refusal::Failed(ref reason)) if reason.contains("could not be listed")),
    );
    let whole = Found {
        leftover: &OLD,
        amount: Some(Amount::unknown(Why::Failed("apt-get -s failed".into()))),
    };
    assert!(
        matches!(remove(&whole, Consent::All), Err(Refusal::Failed(ref reason)) if reason.contains("apt-get -s failed")),
        "a kind that removes without a list still refuses when its listing failed"
    );
}

#[test]
fn a_kind_that_takes_a_list_refuses_when_its_list_is_unknown_for_any_reason() {
    for (kind, why) in [
        (
            &LISTED,
            Why::Cannot("apt would remove other packages with them"),
        ),
        (&LISTED, Why::NeedsAdmin),
        (&SCRIPTED, Why::Cannot("snap is not answering")),
    ] {
        let unknown = Found {
            leftover: kind,
            amount: Some(Amount::unknown(why.clone())),
        };
        assert_eq!(
            remove(&unknown, Consent::All),
            Err(Refusal::Failed(format!(
                "they could not be listed, so nothing was removed: {}",
                why.reason()
            ))),
            "{} with {why:?}",
            kind.id
        );
    }
}

#[test]
fn only_a_failed_measure_asks_for_the_quick_retry() {
    let unknown = |why: Why| Found {
        leftover: &OLD,
        amount: Some(Amount::unknown(why)),
    };
    assert!(fake::survey(vec![unknown(Why::Failed("apt is locked".into()))]).failed());
    assert!(!fake::survey(vec![unknown(Why::NeedsAdmin)]).failed());
    assert!(!fake::survey(vec![unknown(Why::Cannot("no dry run"))]).failed());
    assert!(!fake::survey(vec![found(&OLD, 1, &[("x", 1)])]).failed());
    let absent = Found {
        leftover: &OLD,
        amount: None,
    };
    assert!(!fake::survey(vec![absent]).failed());
}

fn on() -> Option<State> {
    Some(State::On { idle: false })
}

fn unread() -> Option<State> {
    None
}

fn refuse(_on: bool) -> Result<(), Refusal> {
    Err(Refusal::Denied)
}

fn accept(_on: bool) -> Result<(), Refusal> {
    Ok(())
}

const fn row(
    name: &'static str,
    requires: Requires,
    read: fn() -> Option<State>,
    set: fn(bool) -> Result<(), Refusal>,
) -> Switch {
    Switch {
        id: "kernels",
        owner: Owner {
            title: "owner",
            colophon: "owner",
        },
        name,
        short: name,
        doing: name,
        when: name,
        words: Words {
            on: name,
            idle: name,
            off: name,
        },
        covers: &[],
        requires,
        read,
        set,
    }
}

static ROWS: [Switch; 4] = [
    row(
        "missing",
        Requires::Path("/nowhere/qol-cleanup"),
        on,
        refuse,
    ),
    row("unread", Requires::Path("/"), unread, refuse),
    row("live", Requires::Path("/"), on, accept),
    row("second", Requires::Path("/"), on, refuse),
];

#[test]
fn the_first_present_row_that_reads_is_the_live_one_for_its_id() {
    let rows = live(&ROWS);
    let names: Vec<&str> = rows.iter().map(|live| live.switch.name).collect();
    assert_eq!(names, ["live"]);
    assert_eq!(
        named(&ROWS, "kernels").map(|switch| switch.name),
        Some("unread"),
        "a journal id names the first row present on this computer"
    );
    assert_eq!(
        named(&ROWS[..1], "kernels").map(|switch| switch.name),
        Some("missing")
    );
    assert!(named(&ROWS, "by_hand").is_none());
}

#[test]
fn a_switch_change_goes_to_the_row_live_on_this_computer() {
    let rows = live(&ROWS);
    assert_eq!(
        set_live(&rows, &ROWS[0], true),
        Ok(()),
        "the missing row would refuse; the live row accepts"
    );
    assert_eq!(
        set_live(&[], &ROWS[2], true),
        Err(Refusal::Failed("live is not on this computer".into()))
    );
}

#[test]
fn the_primary_switch_is_the_one_that_covers_kinds_the_system_installed() {
    let live = fake::mint();
    assert_eq!(
        primary(&live).map(|live| live.switch.name),
        Some("Old kernels and leftovers")
    );
    assert!(primary(&live[1..]).is_none());
    assert!(State::On { idle: false }.removing());
    assert!(!State::On { idle: true }.removing());
    assert!(State::On { idle: true }.on());
    assert!(!State::Unknown("split").on());
}
