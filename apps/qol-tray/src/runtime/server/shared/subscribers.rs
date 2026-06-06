use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

use super::lock_or_recover;

pub(super) struct SubscriberEntry {
    pub(super) plugin_id: String,
    pub(super) interests: HashSet<RuntimeEventKind>,
    pub(super) tx: Sender<RuntimeEvent>,
}

pub(super) fn push(
    subscribers: &Mutex<Vec<SubscriberEntry>>,
    plugin_id: String,
    interests: HashSet<RuntimeEventKind>,
    tx: Sender<RuntimeEvent>,
) {
    lock_or_recover(subscribers).push(SubscriberEntry {
        plugin_id,
        interests,
        tx,
    });
}

pub(super) fn has_subscribers(subscribers: &Mutex<Vec<SubscriberEntry>>) -> bool {
    !lock_or_recover(subscribers).is_empty()
}

pub(super) fn publish(
    subscribers: &Mutex<Vec<SubscriberEntry>>,
    events: &[RuntimeEvent],
    armed_lifelines: &[String],
    monitors: &[qol_runtime::MonitorBounds],
) {
    let mut subscribers = lock_or_recover(subscribers);
    let results = fan_out(&subscribers, events);

    let amc_interested = amc_interested_ids(&subscribers);
    super::super::trace::publish_summary(
        events,
        &results,
        &amc_interested,
        armed_lifelines,
        monitors,
    );

    retain_succeeded(&mut subscribers, &results);
}

fn fan_out(subscribers: &[SubscriberEntry], events: &[RuntimeEvent]) -> Vec<(String, bool, bool)> {
    subscribers
        .iter()
        .map(|entry| {
            let mut success = true;
            let mut amc_delivered = false;
            for event in events {
                if !entry.interests.contains(&event_kind(event)) {
                    continue;
                }
                if entry.tx.send(event.clone()).is_err() {
                    success = false;
                    break;
                }
                if matches!(event, RuntimeEvent::ActiveMonitorChanged { .. }) {
                    amc_delivered = true;
                }
            }
            (entry.plugin_id.clone(), success, amc_delivered)
        })
        .collect()
}

fn retain_succeeded(subscribers: &mut Vec<SubscriberEntry>, results: &[(String, bool, bool)]) {
    let mut i = 0;
    subscribers.retain(|_| {
        let success = results[i].1;
        i += 1;
        success
    });
}

fn amc_interested_ids(subscribers: &[SubscriberEntry]) -> Vec<String> {
    subscribers
        .iter()
        .filter(|entry| {
            entry
                .interests
                .contains(&RuntimeEventKind::ActiveMonitorChanged)
        })
        .map(|entry| entry.plugin_id.clone())
        .collect()
}

fn event_kind(event: &RuntimeEvent) -> RuntimeEventKind {
    match event {
        RuntimeEvent::ActiveMonitorChanged { .. } => RuntimeEventKind::ActiveMonitorChanged,
        RuntimeEvent::CursorMoved { .. } => RuntimeEventKind::CursorMoved,
        RuntimeEvent::FocusChanged { .. } => RuntimeEventKind::FocusChanged,
        RuntimeEvent::LauncherAppsSynced { .. } => RuntimeEventKind::LauncherAppsSynced,
        RuntimeEvent::MonitorsChanged { .. } => RuntimeEventKind::MonitorsChanged,
        RuntimeEvent::WindowListChanged => RuntimeEventKind::WindowListChanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
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

    fn launcher_apps_synced() -> RuntimeEvent {
        RuntimeEvent::LauncherAppsSynced {
            dir: std::path::PathBuf::from("/a/b/Applications/QoL"),
        }
    }

    fn window_list_changed() -> RuntimeEvent {
        RuntimeEvent::WindowListChanged
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
            (launcher_apps_synced(), RuntimeEventKind::LauncherAppsSynced),
            (monitors_changed(), RuntimeEventKind::MonitorsChanged),
            (window_list_changed(), RuntimeEventKind::WindowListChanged),
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
        push(
            &subs,
            "test".to_string(),
            interests(&[RuntimeEventKind::CursorMoved]),
            tx,
        );
        assert!(has_subscribers(&subs));
    }

    #[test]
    fn publish_delivers_only_subscribed_event_kinds() {
        let subs = Mutex::new(Vec::new());
        let (tx, rx) = std_mpsc::channel();
        push(
            &subs,
            "test".to_string(),
            interests(&[RuntimeEventKind::CursorMoved]),
            tx,
        );

        publish(
            &subs,
            &[
                cursor_moved(10.0, 20.0),
                focus_changed(),
                cursor_moved(30.0, 40.0),
            ],
            &[],
            &[],
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
        push(
            &subs,
            "test".to_string(),
            interests(&[RuntimeEventKind::CursorMoved]),
            tx,
        );
        drop(rx);

        publish(&subs, &[cursor_moved(1.0, 1.0)], &[], &[]);

        assert!(
            !has_subscribers(&subs),
            "disconnected receiver must be evicted on send failure",
        );
    }

    #[test]
    fn publish_keeps_subscribers_when_no_relevant_events_fire() {
        let subs = Mutex::new(Vec::new());
        let (tx, _rx) = std_mpsc::channel();
        push(
            &subs,
            "test".to_string(),
            interests(&[RuntimeEventKind::FocusChanged]),
            tx,
        );

        publish(
            &subs,
            &[cursor_moved(1.0, 2.0), monitors_changed()],
            &[],
            &[],
        );

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
            "test-cursor".to_string(),
            interests(&[RuntimeEventKind::CursorMoved]),
            tx_cursor,
        );
        push(
            &subs,
            "test-focus".to_string(),
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
            &[],
            &[],
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
            plugin_id: "test".to_string(),
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

    fn event_strategy() -> impl Strategy<Value = RuntimeEvent> {
        prop_oneof![
            (any::<f32>(), any::<f32>()).prop_map(|(x, y)| cursor_moved(x, y)),
            Just(active_monitor_changed()),
            Just(focus_changed()),
            Just(launcher_apps_synced()),
            Just(monitors_changed()),
            Just(window_list_changed()),
        ]
    }

    fn interest_set() -> impl Strategy<Value = HashSet<RuntimeEventKind>> {
        proptest::collection::hash_set(
            prop_oneof![
                Just(RuntimeEventKind::CursorMoved),
                Just(RuntimeEventKind::FocusChanged),
                Just(RuntimeEventKind::ActiveMonitorChanged),
                Just(RuntimeEventKind::LauncherAppsSynced),
                Just(RuntimeEventKind::MonitorsChanged),
                Just(RuntimeEventKind::WindowListChanged),
            ],
            0..=6,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_publish_delivers_exactly_subscribed_kinds(
            interests_a in interest_set(),
            interests_b in interest_set(),
            events in proptest::collection::vec(event_strategy(), 0..32),
        ) {
            let subs = Mutex::new(Vec::new());
            let (tx_a, rx_a) = std_mpsc::channel();
            let (tx_b, rx_b) = std_mpsc::channel();
            push(&subs, "test-a".to_string(), interests_a.clone(), tx_a);
            push(&subs, "test-b".to_string(), interests_b.clone(), tx_b);

            publish(&subs, &events, &[], &[]);

            let received_a = drain(&rx_a);
            let received_b = drain(&rx_b);
            let expected_a: Vec<_> = events.iter()
                .filter(|event| interests_a.contains(&event_kind(event)))
                .cloned()
                .collect();
            let expected_b: Vec<_> = events.iter()
                .filter(|event| interests_b.contains(&event_kind(event)))
                .cloned()
                .collect();
            prop_assert_eq!(received_a.len(), expected_a.len());
            prop_assert_eq!(received_b.len(), expected_b.len());
            for (got, exp) in received_a.iter().zip(expected_a.iter()) {
                prop_assert_eq!(event_kind(got), event_kind(exp));
            }
            for (got, exp) in received_b.iter().zip(expected_b.iter()) {
                prop_assert_eq!(event_kind(got), event_kind(exp));
            }
        }

        #[test]
        fn prop_publish_evicts_disconnected_subscribers(
            interests in interest_set(),
            events in proptest::collection::vec(event_strategy(), 1..16),
        ) {
            // Only meaningful when at least one event matches an interest, otherwise
            // send is never attempted and eviction is impossible.
            let any_match = events.iter().any(|event| interests.contains(&event_kind(event)));
            prop_assume!(any_match);

            let subs = Mutex::new(Vec::new());
            let (tx, rx) = std_mpsc::channel();
            push(&subs, "test".to_string(), interests, tx);
            drop(rx);

            publish(&subs, &events, &[], &[]);

            prop_assert!(!has_subscribers(&subs), "disconnected sub must be evicted");
        }
    }
}
