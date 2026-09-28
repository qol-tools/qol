use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::{Handoff, Local, Operation, Peer, Remote, RemoteError};
use crate::bluetooth::DeviceInfo;
use crate::connect::{Attempt, Route};

const EARBUDS: &str = "AA:BB:CC:DD:EE:FF";

fn earbuds(connected: bool) -> DeviceInfo {
    DeviceInfo {
        address: EARBUDS.into(),
        alias: "Earbuds".into(),
        paired: true,
        trusted: true,
        connected,
        audio_connected: Some(connected),
        services_resolved: connected,
        icon: Some("audio-headset".into()),
        class: Some(0x240404),
        uuids: vec!["0000110b-0000-1000-8000-00805f9b34fb".into()],
        rssi: None,
    }
}

type Reply = (String, Operation, Result<Value, RemoteError>);

#[derive(Default)]
struct FakeRemote {
    peers: Vec<(&'static str, &'static str)>,
    replies: RefCell<VecDeque<Reply>>,
    calls: RefCell<Vec<(String, Operation)>>,
}

impl FakeRemote {
    fn reply(self, peer: &str, operation: Operation, reply: Result<Value, RemoteError>) -> Self {
        self.replies
            .borrow_mut()
            .push_back((peer.to_string(), operation, reply));
        self
    }
}

impl Remote for FakeRemote {
    fn peers(&self) -> Result<Vec<Peer>> {
        Ok(self
            .peers
            .iter()
            .map(|(id, name)| Peer {
                id: (*id).into(),
                name: (*name).into(),
            })
            .collect())
    }

    fn call(&self, peer: &Peer, operation: Operation, address: &str) -> Result<Value, RemoteError> {
        assert_eq!(address, EARBUDS);
        self.calls.borrow_mut().push((peer.id.clone(), operation));
        let mut replies = self.replies.borrow_mut();
        let index = replies
            .iter()
            .position(|(id, expected, _)| *id == peer.id && *expected == operation)
            .unwrap_or_else(|| panic!("unexpected {operation:?} on {}", peer.id));
        replies.remove(index).unwrap().2
    }
}

struct FakeLocal {
    device: DeviceInfo,
    connects: RefCell<VecDeque<Result<DeviceInfo>>>,
    attempts: RefCell<u32>,
}

impl FakeLocal {
    fn new(connects: Vec<Result<DeviceInfo>>) -> Self {
        Self {
            device: earbuds(false),
            connects: RefCell::new(connects.into()),
            attempts: RefCell::new(0),
        }
    }
}

impl Local for FakeLocal {
    fn device(&self, _: &str) -> Result<Option<DeviceInfo>> {
        Ok(Some(self.device.clone()))
    }

    fn connect(&self, _: &str) -> Result<DeviceInfo> {
        *self.attempts.borrow_mut() += 1;
        self.connects
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err(anyhow!("no more connects")))
    }

    fn pause(&self, _: Duration) {}
}

fn failure(attempt: Attempt) -> String {
    match attempt {
        Attempt::Failed(error) => format!("{error:#}"),
        _ => panic!("expected a failure"),
    }
}

#[test]
fn the_computer_holding_the_earbuds_releases_them_and_they_connect_here() {
    let remote = FakeRemote {
        peers: vec![("desk", "Desk"), ("laptop", "Laptop")],
        ..Default::default()
    }
    .reply(
        "desk",
        Operation::State,
        Ok(json!({"known": true, "connected": false})),
    )
    .reply(
        "laptop",
        Operation::State,
        Ok(json!({"known": true, "connected": true})),
    )
    .reply(
        "laptop",
        Operation::Release,
        Ok(json!({"released": true, "was_connected": true})),
    );
    let local = FakeLocal::new(vec![Err(anyhow!("slot still busy")), Ok(earbuds(true))]);
    let handoff = Handoff { remote, local };

    assert!(matches!(
        handoff.attempt("aa:bb:cc:dd:ee:ff"),
        Attempt::Connected(_)
    ));
    assert_eq!(*handoff.local.attempts.borrow(), 2);
    assert!(!handoff
        .remote
        .calls
        .borrow()
        .iter()
        .any(|(_, operation)| *operation == Operation::Resume));
}

#[test]
fn a_failed_local_connect_tells_the_other_computer_to_reconnect() {
    let remote = FakeRemote {
        peers: vec![("laptop", "Laptop")],
        ..Default::default()
    }
    .reply("laptop", Operation::State, Ok(json!({"connected": true})))
    .reply("laptop", Operation::Release, Ok(json!({"released": true})))
    .reply("laptop", Operation::Resume, Ok(json!({"resumed": true})));
    let local = FakeLocal::new(vec![
        Err(anyhow!("refused")),
        Ok(earbuds(false)),
        Err(anyhow!("refused")),
    ]);
    let handoff = Handoff { remote, local };

    let message = failure(handoff.attempt(EARBUDS));

    assert!(
        message.contains("Laptop can reconnect them again"),
        "{message}"
    );
    assert_eq!(*handoff.local.attempts.borrow(), 3);
    assert_eq!(
        handoff.remote.calls.borrow().last(),
        Some(&("laptop".to_string(), Operation::Resume))
    );
}

#[test]
fn an_unproven_release_stops_before_this_computer_touches_the_earbuds() {
    for reply in [
        Err(RemoteError::Unknown),
        Err(RemoteError::Refused("not allowed".into())),
        Ok(json!({"released": false})),
    ] {
        let remote = FakeRemote {
            peers: vec![("laptop", "Laptop")],
            ..Default::default()
        }
        .reply("laptop", Operation::State, Ok(json!({"connected": true})))
        .reply("laptop", Operation::Release, reply);
        let local = FakeLocal::new(vec![Ok(earbuds(true))]);
        let handoff = Handoff { remote, local };

        failure(handoff.attempt(EARBUDS));
        assert_eq!(*handoff.local.attempts.borrow(), 0);
    }
}

#[test]
fn without_a_holder_the_route_steps_aside_and_names_unreachable_computers() {
    let remote = FakeRemote {
        peers: vec![("laptop", "Laptop")],
        ..Default::default()
    }
    .reply(
        "laptop",
        Operation::State,
        Err(RemoteError::Refused("it is not connected".into())),
    );
    let handoff = Handoff {
        remote,
        local: FakeLocal::new(Vec::new()),
    };
    let Attempt::Skipped(Some(note)) = handoff.attempt(EARBUDS) else {
        panic!("expected the route to step aside with a note");
    };
    assert!(note.contains("Laptop (it is not connected)"), "{note}");

    let remote = FakeRemote {
        peers: vec![("laptop", "Laptop")],
        ..Default::default()
    }
    .reply("laptop", Operation::State, Ok(json!({"connected": false})));
    let handoff = Handoff {
        remote,
        local: FakeLocal::new(Vec::new()),
    };
    assert!(matches!(handoff.attempt(EARBUDS), Attempt::Skipped(None)));
    assert_eq!(*handoff.local.attempts.borrow(), 0);
}

#[test]
fn unpaired_devices_need_no_remote_call() {
    let mut local = FakeLocal::new(Vec::new());
    local.device.paired = false;
    let handoff = Handoff {
        remote: FakeRemote::default(),
        local,
    };
    assert!(matches!(handoff.attempt(EARBUDS), Attempt::Skipped(None)));
    assert!(handoff.remote.calls.borrow().is_empty());
}
