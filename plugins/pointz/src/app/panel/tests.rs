use std::cell::RefCell;

use chrono::TimeZone;
use qol_config::pages::{Pages, Row};

use super::*;

struct Fake {
    snapshot: RefCell<Snapshot>,
    calls: RefCell<Vec<String>>,
    works: bool,
}

impl Fake {
    fn new(serving: Serving) -> Self {
        Self {
            snapshot: RefCell::new(Snapshot {
                serving,
                phones: vec![phone("a", "Pixel 8"), phone("b", "Galaxy S24")],
                pairing: None,
                hostname: "desk".into(),
                ip: Some("10.0.0.2".into()),
            }),
            calls: RefCell::new(Vec::new()),
            works: true,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Tray for Fake {
    fn snapshot(&self) -> Snapshot {
        self.snapshot.borrow().clone()
    }

    fn serve(&self, on: bool) -> bool {
        self.calls.borrow_mut().push(format!("serve {on}"));
        if self.works {
            self.snapshot.borrow_mut().serving = if on { Serving::Running } else { Serving::Off };
        }
        self.works
    }

    fn begin(&self) -> bool {
        self.calls.borrow_mut().push("begin".into());
        if self.works {
            self.snapshot.borrow_mut().pairing = Some(Pairing {
                code: "482913".into(),
                seconds: 60,
            });
        }
        self.works
    }

    fn cancel(&self) {
        self.calls.borrow_mut().push("cancel".into());
        self.snapshot.borrow_mut().pairing = None;
    }

    fn remove(&self, device: &str) -> bool {
        self.calls.borrow_mut().push(format!("remove {device}"));
        if self.works {
            self.snapshot
                .borrow_mut()
                .phones
                .retain(|phone| phone.id != device);
        }
        self.works
    }
}

fn phone(id: &str, name: &str) -> Phone {
    Phone {
        id: id.into(),
        name: name.into(),
        paired_at_ms: 0,
    }
}

fn now() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap()
}

fn press(
    fake: &Fake,
    memory: &mut Memory,
    page: &str,
    row: &str,
    open: &[&str],
    set: Option<bool>,
) -> PressReply {
    let input = serde_json::json!({ "page": page, "row": row, "open": open, "set": set });
    let ReadResult::<()>::HandledWithData(data) = answer_press(fake, memory, &input, now()) else {
        panic!("a press reply");
    };
    serde_json::from_value(data).unwrap()
}

fn pages(fake: &Fake, memory: &mut Memory, open: &[&str]) -> Pages {
    let input = serde_json::json!({ "open": open });
    let ReadResult::<()>::HandledWithData(data) = answer_pages(fake, memory, &input, now()) else {
        panic!("pages");
    };
    serde_json::from_value(data).unwrap()
}

fn focus(pages: &Pages) -> Option<&str> {
    pages.page("pointzerver")?.focus.as_deref()
}

fn row<'a>(pages: &'a Pages, page: &str, row: &str) -> Option<&'a Row> {
    pages.page(page)?.row(row)
}

#[test]
fn pages_answer_the_root_and_the_open_pages_only() {
    let fake = Fake::new(Serving::Running);
    let root = pages(&fake, &mut Memory::new(), &[]);
    assert_eq!(root.pages.keys().collect::<Vec<_>>(), ["main"]);
    let open = pages(&fake, &mut Memory::new(), &["pointzerver", "phone:a"]);
    assert_eq!(
        open.pages.keys().collect::<Vec<_>>(),
        ["main", "pointzerver", "phone:a"]
    );
    let ReadResult::<()>::Error(_) =
        answer_pages(&fake, &mut Memory::new(), &serde_json::json!(7), now())
    else {
        panic!("a bad stack is refused");
    };
}

#[test]
fn the_switch_turns_pointzerver_off_and_on() {
    let fake = Fake::new(Serving::Running);
    let mut memory = Memory::new();
    let reply = press(&fake, &mut memory, "main", "server", &[], Some(false));
    assert_eq!(fake.calls(), ["serve false"]);
    assert_eq!(reply.go, None);
    assert_eq!(reply.notice, None);
    let server = row(reply.pages.as_ref().unwrap(), "main", "server").unwrap();
    assert_eq!(
        server.detail.as_deref(),
        Some("Off · phones cannot reach this computer")
    );

    let broken = Fake {
        works: false,
        ..Fake::new(Serving::Off)
    };
    let reply = press(&broken, &mut memory, "main", "server", &[], Some(true));
    assert_eq!(
        reply.notice,
        Some(Notice::Failure("PointZerver could not start".into()))
    );
}

#[test]
fn pairing_opens_the_card_and_a_new_phone_closes_it_with_the_focus() {
    let fake = Fake::new(Serving::Off);
    let mut memory = Memory::new();
    let reply = press(
        &fake,
        &mut memory,
        "pointzerver",
        "pair",
        &["pointzerver"],
        None,
    );
    assert_eq!(fake.calls(), ["serve true", "begin"]);
    assert_eq!(reply.go, Some(Go::Open("pair".into())));
    let card = reply.pages.as_ref().unwrap();
    assert_eq!(
        row(card, "pair", "code").unwrap().value.as_deref(),
        Some("482 913")
    );

    let open = ["pointzerver", "pair"];
    assert!(pages(&fake, &mut memory, &open).page("pair").is_some());
    fake.snapshot.borrow_mut().phones.push(phone("c", "Moto G"));
    let joined = pages(&fake, &mut memory, &open);
    assert!(joined.page("pair").is_none());
    assert_eq!(focus(&joined), Some("phone:c"));
    assert_eq!(fake.calls(), ["serve true", "begin", "cancel"]);
}

#[test]
fn leaving_the_card_stops_pairing_and_a_refused_pairing_says_so() {
    let fake = Fake::new(Serving::Running);
    let mut memory = Memory::new();
    press(
        &fake,
        &mut memory,
        "pointzerver",
        "pair",
        &["pointzerver"],
        None,
    );
    let reply = press(&fake, &mut memory, "pair", "stop", &["pointzerver"], None);
    assert_eq!(fake.calls(), ["begin", "cancel"]);
    assert!(reply.pages.unwrap().page("pair").is_none());
    assert_eq!(memory, Memory::new());

    let broken = Fake {
        works: false,
        ..Fake::new(Serving::Running)
    };
    let reply = press(
        &broken,
        &mut Memory::new(),
        "pointzerver",
        "pair",
        &["pointzerver"],
        None,
    );
    assert_eq!(reply.go, None);
    assert_eq!(
        reply.notice,
        Some(Notice::Failure(
            "PointZerver could not start pairing".into()
        ))
    );
}

#[test]
fn removing_a_phone_goes_back_with_the_focus_on_its_neighbour() {
    let fake = Fake::new(Serving::Running);
    let mut memory = Memory::new();
    let open = ["pointzerver", "phone:a"];
    let reply = press(&fake, &mut memory, "phone:a", "remove", &open, None);
    assert_eq!(fake.calls(), ["remove a"]);
    assert_eq!(reply.go, Some(Go::Back));
    let pages = reply.pages.unwrap();
    assert!(pages.page("phone:a").is_none());
    assert_eq!(focus(&pages), Some("phone:b"));

    let broken = Fake {
        works: false,
        ..Fake::new(Serving::Running)
    };
    let reply = press(
        &broken,
        &mut Memory::new(),
        "phone:b",
        "remove",
        &open,
        None,
    );
    assert_eq!(reply.go, None);
    assert_eq!(
        reply.notice,
        Some(Notice::Failure(
            "PointZerver could not remove Galaxy S24".into()
        ))
    );
}

#[test]
fn transport_states_map_to_what_the_switch_says() {
    let status = |transport| PointzStatus {
        plugin: qol_peers::pointz::PointzPlugin::Compatible,
        authority: None,
        migration: None,
        server_id: None,
        device_count: 0,
        pairing: qol_peers::pointz::PointzPairing::CLOSED,
        transport,
    };
    let cases = [
        (PointzTransport::Running { dropped: 0 }, Serving::Running),
        (PointzTransport::Stopped {}, Serving::Off),
        (
            PointzTransport::PortBusy {
                socket: PointzSocket::Command,
            },
            Serving::PortBusy(45455),
        ),
        (
            PointzTransport::Failed {
                socket: PointzSocket::Discovery,
            },
            Serving::Failed(45454),
        ),
    ];
    for (transport, expected) in cases {
        assert_eq!(serving(&status(transport)), expected);
    }
    assert!(phones(&status(PointzTransport::Stopped {})).is_empty());
}
