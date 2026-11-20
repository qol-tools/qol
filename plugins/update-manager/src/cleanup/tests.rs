use std::collections::BTreeSet;

use super::fake::{self, found, ALONG, BY_HAND, LAST, LISTED, OLD, ORPHANS};
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
            Measure::Files { dirs, suffix } => {
                assert!(!dirs.is_empty() && dirs.iter().all(|dir| dir.starts_with('/')));
                assert!(word(suffix));
            }
            Measure::Command { argv, .. } => assert!(argv.iter().all(|arg| word(arg))),
            Measure::Unknown(reason) => assert!(word(reason)),
        }
        match leftover.remove {
            Remove::Command { argv, .. } => assert!(!argv.is_empty()),
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
        for kind in switch.covers {
            assert!(
                leftover(kind.id).is_some(),
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
        survey.total(|_| true),
        Total {
            bytes: 35,
            estimate: false,
            unknown: 1,
        }
    );
    assert_eq!(survey.total(|kind| kind.id == "kernels").bytes, 30);
    let lump = fake::survey(vec![
        found(&OLD, 1, &[("linux-image-1", 10)]),
        Found {
            leftover: &ORPHANS,
            amount: Some(Amount {
                count: 2,
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
        lump.total(|_| true),
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
        consent::why(&ALONG),
        "take last along, which are the last way back to the version before"
    );
    for kind in [&LAST, &BY_HAND, &ALONG] {
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
}

#[test]
fn the_first_switch_present_for_an_id_is_the_live_one() {
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
