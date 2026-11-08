use chrono::TimeZone;
use qol_config::pages::{Control, Enter};

use super::*;

fn now() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
}

fn phone(id: &str, name: &str, at: DateTime<Local>) -> Phone {
    Phone {
        id: id.into(),
        name: name.into(),
        paired_at_ms: at.timestamp_millis() as u64,
    }
}

fn pixel() -> Phone {
    phone(
        "pixel",
        "Pixel 8",
        Local.with_ymd_and_hms(2026, 11, 3, 9, 0, 0).unwrap(),
    )
}

fn snapshot() -> Snapshot {
    Snapshot {
        serving: Serving::Running,
        phones: vec![pixel()],
        listed: true,
        pairing: None,
        hostname: "kmrh-desk".into(),
        ip: Some("192.168.1.24".into()),
    }
}

fn row<'a>(pages: &'a Pages, page: &str, row: &str) -> &'a Row {
    pages.page(page).unwrap().row(row).unwrap()
}

fn open(pages: &[&str]) -> Vec<String> {
    pages.iter().map(|page| page.to_string()).collect()
}

#[test]
fn the_root_holds_one_feature_row_that_opens_the_pointzerver_page() {
    let pages = build(&snapshot(), &Memory::new(), now());
    assert!(pages.problems().is_empty(), "{:?}", pages.problems());
    assert!(pages.dead_ends().is_empty(), "{:?}", pages.dead_ends());
    let server = row(&pages, ROOT, SERVER);
    assert_eq!(
        server.control,
        Some(Control::Feature {
            on: true,
            count: "1 phone".into()
        })
    );
    assert_eq!(server.enter, Some(Enter::Open(LIST.into())));
    assert_eq!(
        server.detail.as_deref(),
        Some("Running · phones find kmrh-desk at 192.168.1.24")
    );
    assert_eq!(pages.page(ROOT).unwrap().focus.as_deref(), Some(SERVER));
    assert_eq!(pages.page(ROOT).unwrap().rows().count(), 1);
}

#[test]
fn the_feature_row_says_why_phones_cannot_reach_this_computer() {
    let cases = [
        (
            Serving::Off,
            false,
            false,
            "Off · phones cannot reach this computer",
        ),
        (
            Serving::PortBusy(45455),
            true,
            true,
            "Not listening · port 45455 is in use",
        ),
        (
            Serving::Failed(45454),
            true,
            true,
            "Not listening · port 45454 would not open",
        ),
        (
            Serving::Unknown,
            false,
            true,
            "Not listening · qol did not answer",
        ),
    ];
    for (serving, on, attention, detail) in cases {
        let pages = build(
            &Snapshot {
                serving,
                phones: Vec::new(),
                ..snapshot()
            },
            &Memory::new(),
            now(),
        );
        let server = row(&pages, ROOT, SERVER);
        assert_eq!(
            server.control,
            Some(Control::Feature {
                on,
                count: "0 phones".into()
            }),
            "{serving:?}"
        );
        assert_eq!(server.attention, attention, "{serving:?}");
        assert_eq!(server.detail.as_deref(), Some(detail), "{serving:?}");
    }
}

#[test]
fn the_pointzerver_page_lists_phones_between_pairing_and_the_app() {
    let joined = phone("galaxy", "Galaxy S24", now() - TimeDelta::minutes(1));
    let pages = build(
        &Snapshot {
            phones: vec![joined, pixel()],
            ..snapshot()
        },
        &Memory::new(),
        now(),
    );
    let page = pages.page(LIST).unwrap();
    let ids: Vec<&str> = page.groups[0]
        .rows
        .iter()
        .map(|row| row.id.as_str())
        .collect();
    assert_eq!(ids, ["pair", "phone:galaxy", "phone:pixel", "app"]);
    assert_eq!(
        page.groups[0].rows[1].detail.as_deref(),
        Some("Paired just now")
    );
    assert_eq!(
        page.groups[0].rows[2].detail.as_deref(),
        Some("Paired 3 November")
    );
    assert!(page.groups[0].rows[0].opens);
    assert_eq!(page.groups[0].rows[0].enter, Some(Enter::Press));
    let shown: Vec<(&str, Option<&str>)> = page.groups[1]
        .rows
        .iter()
        .map(|row| (row.label.as_str(), row.copies()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Hostname", Some("kmrh-desk")),
            ("IP address", Some("192.168.1.24")),
            ("Control port", Some("45455")),
            ("Discovery port", Some("45454")),
        ]
    );
}

#[test]
fn the_code_shows_only_on_the_pair_card_while_pairing_is_open() {
    let closed = build(&snapshot(), &Memory::new(), now());
    assert!(closed.page(PAIR).is_none());

    let pages = build(
        &Snapshot {
            pairing: Some(Pairing {
                code: "482913".into(),
                seconds: 58,
            }),
            ..snapshot()
        },
        &Memory::new(),
        now(),
    );
    let card = pages.page(PAIR).unwrap();
    let code = card.row("code").unwrap();
    assert_eq!(code.value.as_deref(), Some("482 913"));
    assert_eq!(code.detail.as_deref(), Some("Runs out in 58 s"));
    assert_eq!(code.copies(), Some("482 913"));
    assert_eq!(card.focus.as_deref(), Some(APP));
    let leave = card.leave.as_ref().unwrap();
    assert_eq!(
        (leave.row.as_str(), leave.label.as_str()),
        (STOP, "stop pairing")
    );
    for (id, page) in &pages.pages {
        if id != PAIR {
            assert!(
                page.rows()
                    .all(|row| row.value.as_deref() != Some("482 913")),
                "the code leaked onto {id}"
            );
        }
    }
}

#[test]
fn the_app_card_is_one_qr_row_and_a_phone_page_asks_before_it_removes() {
    let pages = build(&snapshot(), &Memory::new(), now());
    let qr = row(&pages, APP, "qr");
    assert_eq!(
        qr.control,
        Some(Control::Qr(ServerConfig::APP_DOWNLOAD_URL.into()))
    );
    assert_eq!(
        qr.detail.as_deref(),
        Some("Scan to download the PointZ app.")
    );

    let page = pages.page("phone:pixel").unwrap();
    assert_eq!(page.title, "pixel 8");
    assert_eq!(
        page.groups[0].colophon.as_deref(),
        Some("paired 3 november.")
    );
    assert_eq!(
        page.row(REMOVE).unwrap().control,
        Some(Control::Destroy {
            verb: "remove".into(),
            ask: "remove Pixel 8".into()
        })
    );
}

#[test]
fn presses_name_what_the_row_does() {
    assert_eq!(act(ROOT, SERVER, Some(false)), Act::Serve(false));
    assert_eq!(act(ROOT, SERVER, None), Act::Nothing);
    assert_eq!(act(LIST, PAIR, None), Act::Pair);
    assert_eq!(act(PAIR, STOP, None), Act::Stop);
    assert_eq!(
        act("phone:pixel", REMOVE, None),
        Act::Remove("pixel".into())
    );
    assert_eq!(act(LIST, REMOVE, None), Act::Nothing);
    assert_eq!(act(LIST, "app", None), Act::Nothing);
}

#[test]
fn a_phone_that_pairs_closes_the_card_and_takes_the_focus() {
    let mut memory = Memory::new();
    memory.began(&snapshot());
    let pairing = Snapshot {
        pairing: Some(Pairing {
            code: "482913".into(),
            seconds: 40,
        }),
        ..snapshot()
    };
    assert!(!memory.observe(&pairing, &open(&[LIST, PAIR])));

    let joined = Snapshot {
        phones: vec![pixel(), phone("galaxy", "Galaxy S24", now())],
        ..pairing
    };
    assert!(memory.observe(&joined, &open(&[LIST, PAIR])));
    let pages = build(&joined, &memory, now());
    assert_eq!(
        pages.page(LIST).unwrap().focus.as_deref(),
        Some("phone:galaxy")
    );
    assert!(!memory.observe(&joined, &open(&[LIST])));

    assert!(!memory.observe(&joined, &open(&[])));
    assert_eq!(
        build(&joined, &memory, now()).page(LIST).unwrap().focus,
        None
    );
}

#[test]
fn pairing_that_runs_out_or_stops_is_forgotten() {
    let mut memory = Memory::new();
    memory.began(&snapshot());
    assert!(!memory.observe(&snapshot(), &open(&[LIST])));
    let joined = Snapshot {
        phones: vec![pixel(), phone("galaxy", "Galaxy S24", now())],
        ..snapshot()
    };
    assert!(!memory.observe(&joined, &open(&[LIST])));

    memory.began(&snapshot());
    memory.stopped();
    assert_eq!(memory, Memory::new());
}

#[test]
fn an_incomplete_phone_list_never_reads_as_a_phone_that_joined() {
    let mut memory = Memory::new();
    let partial = Snapshot {
        phones: Vec::new(),
        listed: false,
        pairing: Some(Pairing {
            code: "482913".into(),
            seconds: 40,
        }),
        ..snapshot()
    };
    memory.began(&partial);
    assert!(!memory.observe(&snapshot(), &open(&[LIST, PAIR])));
    assert_eq!(
        build(&snapshot(), &memory, now()).page(LIST).unwrap().focus,
        None
    );

    memory.began(&Snapshot {
        listed: true,
        ..partial.clone()
    });
    let unread = Snapshot {
        phones: vec![pixel()],
        ..partial
    };
    assert!(!memory.observe(&unread, &open(&[LIST, PAIR])));
    assert!(memory.observe(
        &Snapshot {
            listed: true,
            ..unread
        },
        &open(&[LIST, PAIR])
    ));
}

#[test]
fn removing_a_phone_moves_the_focus_to_its_neighbour() {
    let phones = [
        phone("a", "A", now()),
        phone("b", "B", now()),
        phone("c", "C", now()),
    ];
    let cases = [
        (&phones[..], "b", "phone:c"),
        (&phones[..], "c", "phone:b"),
        (&phones[..1], "a", PAIR),
        (&phones[..], "gone", PAIR),
    ];
    for (phones, removed, focus) in cases {
        let mut memory = Memory::new();
        memory.removed(phones, removed);
        assert_eq!(memory.focus.as_deref(), Some(focus), "{removed}");
    }
    assert_eq!(name_of(&phones, "b"), Some("B"));
    assert_eq!(name_of(&phones, "x"), None);
}
