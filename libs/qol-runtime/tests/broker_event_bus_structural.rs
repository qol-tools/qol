//! Round-trip, gating, and fidelity invariants for the broker event
//! bus core.
//!
//! `EventBus` is the broker's state machine. Each fake peer is a
//! (declared topic set, event queue) pair: the test hands the declared
//! set to the gate, exactly as the socket layer will hand the resolved
//! manifest declaration of a connected plugin, and drains the queue,
//! exactly as a connection writer task will.
//!
//! Closes: general inter-plugin event bus, V1 scope.

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use qol_runtime::broker::{EventBus, TopicCheckError, BUS_QUEUE_CAPACITY};

const TOPIC: &str = "kitty.session_opened";
const OTHER_TOPIC: &str = "kitty.session_closed";
const RECV_TIMEOUT: Duration = Duration::from_secs(1);
const QUIET_TIMEOUT: Duration = Duration::from_millis(50);

fn declared(topics: &[&str]) -> Vec<String> {
    topics.iter().map(|topic| topic.to_string()).collect()
}

#[test]
fn publish_reaches_every_current_subscriber_with_payload_intact() {
    // Two fake peers subscribe with the topic declared; a third peer
    // (also declared) publishes. Both subscribers must receive the
    // event with the payload byte-for-byte identical.
    let mut bus = EventBus::new();
    let (_, peer_a) = bus
        .subscribe(&declared(&[TOPIC]), TOPIC)
        .expect("peer A subscribes");
    let (_, peer_b) = bus
        .subscribe(&declared(&[TOPIC]), TOPIC)
        .expect("peer B subscribes");

    let payload = serde_json::json!({
        "pane_id": "p1",
        "cwd": "/home/u/proj",
        "tags": ["claude", "agent"],
        "score": 1.5,
        "meta": { "n": 3, "ok": true },
    });
    let delivered = bus
        .publish(&declared(&[TOPIC]), TOPIC, &payload)
        .expect("publisher declared the topic");

    assert_eq!(
        delivered, 2,
        "expected both current subscribers to accept the event"
    );
    for (index, queue) in [peer_a, peer_b].into_iter().enumerate() {
        let event = queue
            .recv_timeout(RECV_TIMEOUT)
            .unwrap_or_else(|error| panic!("peer {index} missed the event: {error}"));
        assert_eq!(event.topic, TOPIC, "peer {index} received the wrong topic");
        assert_eq!(
            event.payload, payload,
            "peer {index} received a mutated payload; round-trip fidelity is broken"
        );
    }
}

#[test]
fn gating_denies_peers_that_lack_the_topics_capability() {
    // A peer whose declared set does not include the topic must be
    // refused on both directions, with the topic echoed in the error.
    let mut bus = EventBus::new();

    match bus.subscribe(&declared(&[]), TOPIC) {
        Err(TopicCheckError::NotDeclared { topic }) => assert_eq!(topic, TOPIC),
        other => {
            panic!("subscribe with an empty declared set returned {other:?}; expected NotDeclared")
        }
    }
    match bus.publish(&declared(&[OTHER_TOPIC]), TOPIC, &serde_json::json!(1)) {
        Err(TopicCheckError::NotDeclared { topic }) => assert_eq!(topic, TOPIC),
        other => panic!(
            "publish with an unrelated declared set returned {other:?}; expected NotDeclared"
        ),
    }

    // The refused subscribe registered nothing: a declared publisher
    // sees zero subscribers instead of a silent no-op delivery path.
    let delivered = bus
        .publish(&declared(&[TOPIC]), TOPIC, &serde_json::json!(1))
        .expect("declared publisher");
    assert_eq!(delivered, 0, "refused subscription was registered");
}

#[test]
fn full_queue_drops_events_without_blocking_the_publisher() {
    // A subscriber that never drains has a bounded queue; once it is
    // full, further events are dropped (fire-and-forget) and the
    // publisher is not blocked. The bounded count is the contract.
    let mut bus = EventBus::new();
    let (_, queue) = bus
        .subscribe(&declared(&[TOPIC]), TOPIC)
        .expect("subscriber");

    let mut delivered = 0;
    for _ in 0..BUS_QUEUE_CAPACITY + 10 {
        delivered += bus
            .publish(&declared(&[TOPIC]), TOPIC, &serde_json::json!(0))
            .expect("publisher declared the topic");
    }
    assert_eq!(
        delivered, BUS_QUEUE_CAPACITY,
        "the queue must hold exactly {BUS_QUEUE_CAPACITY} events, then drop"
    );

    // The buffered events are intact and in order up to the cap.
    for index in 0..BUS_QUEUE_CAPACITY {
        let event = queue
            .recv_timeout(RECV_TIMEOUT)
            .unwrap_or_else(|error| panic!("buffered event {index} lost: {error}"));
        assert_eq!(event.payload, serde_json::json!(0));
    }
    assert!(matches!(
        queue.recv_timeout(QUIET_TIMEOUT),
        Err(RecvTimeoutError::Timeout)
    ));
}

#[test]
fn unsubscribe_stops_delivery() {
    let mut bus = EventBus::new();
    let (id, queue) = bus
        .subscribe(&declared(&[TOPIC]), TOPIC)
        .expect("subscribe");

    bus.unsubscribe(id, TOPIC);

    let delivered = bus
        .publish(&declared(&[TOPIC]), TOPIC, &serde_json::json!(1))
        .expect("publisher declared the topic");
    assert_eq!(delivered, 0, "unsubscribed peer still received events");
    // Unsubscribe drops the bus's only sender for this subscription,
    // so the queue reads as closed: `Disconnected`, not a quiet
    // `Timeout`. Closure is the deterministic end-of-stream signal -
    // delivery can never resume - whereas a timeout would only mean
    // "nothing right now".
    assert!(matches!(
        queue.recv_timeout(QUIET_TIMEOUT),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn topics_are_isolated() {
    // Events on one topic must never reach subscribers of another.
    let mut bus = EventBus::new();
    let (_, queue) = bus
        .subscribe(&declared(&[TOPIC, OTHER_TOPIC]), TOPIC)
        .expect("subscribe to TOPIC");

    bus.publish(
        &declared(&[OTHER_TOPIC]),
        OTHER_TOPIC,
        &serde_json::json!(1),
    )
    .expect("publisher declared OTHER_TOPIC");

    assert!(matches!(
        queue.recv_timeout(QUIET_TIMEOUT),
        Err(RecvTimeoutError::Timeout)
    ));
}
