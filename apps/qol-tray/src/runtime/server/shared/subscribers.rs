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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    fn cursor_moved(x: f32, y: f32) -> RuntimeEvent {
        RuntimeEvent::CursorMoved { x, y }
    }

    fn active_monitor_changed() -> RuntimeEvent {
        RuntimeEvent::ActiveMonitorChanged {
            monitor_idx: Some(0),
            monitor: None,
        }
    }

    fn focus_changed() -> RuntimeEvent {
        RuntimeEvent::FocusChanged {
            monitor_idx: Some(1),
            monitor: None,
        }
    }

    fn monitors_changed() -> RuntimeEvent {
        RuntimeEvent::MonitorsChanged {
            monitors: Vec::new(),
        }
    }

    fn drain<T>(rx: &std_mpsc::Receiver<T>) -> Vec<T> {
        let mut out = Vec::new();
        while let Ok(event) = rx.recv_timeout(Duration::from_millis(1)) {
            out.push(event);
        }
        out
    }

    fn interests(kinds: &[RuntimeEventKind]) -> HashSet<RuntimeEventKind> {
        kinds.iter().copied().collect()
    }

    #[test]
    fn event_kind_maps_every_variant_exhaustively() {
        let cases = [
            (
                active_monitor_changed(),
                RuntimeEventKind::ActiveMonitorChanged,
            ),
            (cursor_moved(1.0, 2.0), RuntimeEventKind::CursorMoved),
            (focus_changed(), RuntimeEventKind::FocusChanged),
            (monitors_changed(), RuntimeEventKind::MonitorsChanged),
        ];
        for (event, expected) in cases {
            assert_eq!(event_kind(&event), expected, "event: {event:?}");
        }
    }

    #[test]
    fn has_subscribers_reflects_push_state() {
        let subs = Mutex::new(Vec::new());
        assert!(!has_subscribers(&subs));
        let (tx, _rx) = std_mpsc::channel();
        push(&subs, interests(&[RuntimeEventKind::CursorMoved]), tx);
        assert!(has_subscribers(&subs));
    }

    #[test]
    fn publish_delivers_only_subscribed_event_kinds() {
        let subs = Mutex::new(Vec::new());
        let (tx, rx) = std_mpsc::channel();
        push(&subs, interests(&[RuntimeEventKind::CursorMoved]), tx);

        publish(
            &subs,
            &[
                cursor_moved(10.0, 20.0),
                focus_changed(),
                cursor_moved(30.0, 40.0),
            ],
        );

        let received = drain(&rx);
        assert_eq!(
            received.len(),
            2,
            "two cursor events delivered: {received:?}"
        );
        for event in received {
            assert!(matches!(event, RuntimeEvent::CursorMoved { .. }));
        }
    }

    #[test]
    fn publish_drops_subscribers_whose_receiver_is_gone() {
        let subs = Mutex::new(Vec::new());
        let (tx, rx) = std_mpsc::channel();
        push(&subs, interests(&[RuntimeEventKind::CursorMoved]), tx);
        drop(rx);

        publish(&subs, &[cursor_moved(1.0, 1.0)]);

        assert!(
            !has_subscribers(&subs),
            "disconnected receiver must be evicted on send failure",
        );
    }

    #[test]
    fn publish_keeps_subscribers_when_no_relevant_events_fire() {
        let subs = Mutex::new(Vec::new());
        let (tx, _rx) = std_mpsc::channel();
        push(&subs, interests(&[RuntimeEventKind::FocusChanged]), tx);

        publish(&subs, &[cursor_moved(1.0, 2.0), monitors_changed()]);

        assert!(
            has_subscribers(&subs),
            "subscriber must NOT be evicted just because no relevant event fired",
        );
    }

    #[test]
    fn publish_fans_out_to_multiple_subscribers_with_distinct_interests() {
        let subs = Mutex::new(Vec::new());
        let (tx_cursor, rx_cursor) = std_mpsc::channel();
        let (tx_focus, rx_focus) = std_mpsc::channel();
        push(
            &subs,
            interests(&[RuntimeEventKind::CursorMoved]),
            tx_cursor,
        );
        push(
            &subs,
            interests(&[RuntimeEventKind::FocusChanged]),
            tx_focus,
        );

        publish(
            &subs,
            &[
                cursor_moved(1.0, 1.0),
                focus_changed(),
                cursor_moved(2.0, 2.0),
            ],
        );

        let cursor_events = drain(&rx_cursor);
        let focus_events = drain(&rx_focus);
        assert_eq!(cursor_events.len(), 2, "cursor sub gets 2 cursor events");
        assert_eq!(focus_events.len(), 1, "focus sub gets 1 focus event");
    }

    #[test]
    fn publish_to_subscriber_returns_false_when_send_fails_mid_batch() {
        let (tx, rx) = std_mpsc::channel();
        let entry = SubscriberEntry {
            interests: interests(&[
                RuntimeEventKind::CursorMoved,
                RuntimeEventKind::FocusChanged,
            ]),
            tx,
        };
        drop(rx);
        assert!(!publish_to_subscriber(
            &entry,
            &[cursor_moved(1.0, 1.0), focus_changed()],
        ));
    }
}
