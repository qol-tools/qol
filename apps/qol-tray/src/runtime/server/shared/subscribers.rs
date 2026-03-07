use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

use super::lock_or_recover;

pub(super) struct SubscriberEntry {
    pub(super) interests: HashSet<RuntimeEventKind>,
    pub(super) tx: Sender<RuntimeEvent>,
}

pub(super) fn push(
    subscribers: &Mutex<Vec<SubscriberEntry>>,
    interests: HashSet<RuntimeEventKind>,
    tx: Sender<RuntimeEvent>,
) {
    lock_or_recover(subscribers).push(SubscriberEntry { interests, tx });
}

pub(super) fn has_subscribers(subscribers: &Mutex<Vec<SubscriberEntry>>) -> bool {
    !lock_or_recover(subscribers).is_empty()
}

pub(super) fn publish(subscribers: &Mutex<Vec<SubscriberEntry>>, events: &[RuntimeEvent]) {
    let mut subscribers = lock_or_recover(subscribers);
    subscribers.retain(|entry| publish_to_subscriber(entry, events));
}

fn publish_to_subscriber(entry: &SubscriberEntry, events: &[RuntimeEvent]) -> bool {
    for event in events {
        if !entry.interests.contains(&event_kind(event)) {
            continue;
        }
        if entry.tx.send(event.clone()).is_err() {
            return false;
        }
    }
    true
}

fn event_kind(event: &RuntimeEvent) -> RuntimeEventKind {
    match event {
        RuntimeEvent::ActiveMonitorChanged { .. } => RuntimeEventKind::ActiveMonitorChanged,
        RuntimeEvent::CursorMoved { .. } => RuntimeEventKind::CursorMoved,
        RuntimeEvent::FocusChanged { .. } => RuntimeEventKind::FocusChanged,
        RuntimeEvent::MonitorsChanged { .. } => RuntimeEventKind::MonitorsChanged,
    }
}
