use std::cell::RefCell;

use anyhow::anyhow;

use super::{connect, Attempt, Direct, Route};
use crate::bluetooth::DeviceInfo;

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";

fn device() -> DeviceInfo {
    DeviceInfo {
        address: ADDRESS.into(),
        alias: "Earbuds".into(),
        paired: true,
        trusted: true,
        connected: true,
        audio_connected: Some(true),
        services_resolved: true,
        icon: None,
        class: None,
        uuids: Vec::new(),
        rssi: None,
    }
}

struct Scripted {
    attempt: RefCell<Option<Attempt>>,
    tried: RefCell<bool>,
}

impl Scripted {
    fn new(attempt: Attempt) -> Self {
        Self {
            attempt: RefCell::new(Some(attempt)),
            tried: RefCell::new(false),
        }
    }
}

impl Route for Scripted {
    fn attempt(&self, _: &str) -> Attempt {
        *self.tried.borrow_mut() = true;
        self.attempt.borrow_mut().take().unwrap()
    }
}

#[test]
fn the_first_route_that_connects_wins_and_later_routes_are_not_tried() {
    let later = Scripted::new(Attempt::Failed(anyhow!("unused")));
    connect(ADDRESS, &[&Direct(|_: &str| Ok(device())), &later]).unwrap();
    assert!(!*later.tried.borrow());
}

#[test]
fn a_failed_route_falls_through_to_the_next_one() {
    let moved = Scripted::new(Attempt::Connected(device()));
    let direct = Direct(|_: &str| Err(anyhow!("busy")));
    connect(ADDRESS, &[&direct, &moved]).unwrap();
    assert!(*moved.tried.borrow());
}

#[test]
fn when_nothing_connects_the_last_failure_is_reported_with_the_skipped_notes() {
    let skipped = Scripted::new(Attempt::Skipped(Some("not asked: Laptop".into())));
    let direct = Direct(|_: &str| Err(anyhow!("busy")));
    let error = connect(ADDRESS, &[&direct, &skipped]).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("busy"), "{message}");
    assert!(message.contains("not asked: Laptop"), "{message}");
}
