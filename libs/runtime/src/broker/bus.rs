//! In-memory publish/subscribe core for the broker event bus.
//!
//! `EventBus` is the broker's state machine: a topic-keyed registry of
//! subscribers, each with a bounded queue. `publish` gates the
//! publisher against its declared topic set, then fans the payload out
//! to the queues of the current subscribers (fire-and-forget: a full
//! queue drops the event, the publisher never blocks, and no history
//! is retained). Who drains the queues and writes `BrokerEvent` lines
//! back to each peer belongs to the socket layer, not here.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, SyncSender};

use crate::broker::protocol::BrokerEvent;
use crate::broker::topic::{check_topic_access, TopicCheckError};

/// Per-subscriber queue capacity. A subscriber that drains slower than
/// the publisher loses events (bounded fan-out, no retention).
pub const BUS_QUEUE_CAPACITY: usize = 64;

/// Publish/subscribe registry for the broker event bus.
#[derive(Default)]
pub struct EventBus {
    topics: HashMap<String, Vec<Subscriber>>,
    next_id: u64,
}

struct Subscriber {
    id: u64,
    tx: SyncSender<BrokerEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a subscriber for `topic`, gated on `declared`.
    ///
    /// The caller supplies the plugin's manifest-declared topic set
    /// (resolved from peer identity by the socket layer); when `topic`
    /// is not in it, the subscription is refused and nothing is
    /// registered. Returns the subscriber id and the bounded event
    /// queue the connection owner drains.
    pub fn subscribe(
        &mut self,
        declared: &[String],
        topic: &str,
    ) -> Result<(u64, Receiver<BrokerEvent>), TopicCheckError> {
        check_topic_access(declared, topic)?;
        let (tx, rx) = mpsc::sync_channel(BUS_QUEUE_CAPACITY);
        let id = self.next_id;
        self.next_id += 1;
        self.topics
            .entry(topic.to_string())
            .or_default()
            .push(Subscriber { id, tx });
        Ok((id, rx))
    }

    /// Remove the subscriber `id` from `topic` (connection closed).
    pub fn unsubscribe(&mut self, id: u64, topic: &str) {
        if let Some(subscribers) = self.topics.get_mut(topic) {
            subscribers.retain(|subscriber| subscriber.id != id);
        }
    }

    /// Gate the publisher, then deliver `payload` to every current
    /// subscriber of `topic`.
    ///
    /// Returns the number of subscriber queues that accepted the
    /// event; a full or disconnected queue drops it (fire-and-forget).
    pub fn publish(
        &mut self,
        declared: &[String],
        topic: &str,
        payload: &serde_json::Value,
    ) -> Result<usize, TopicCheckError> {
        check_topic_access(declared, topic)?;
        let event = BrokerEvent {
            topic: topic.to_string(),
            payload: payload.clone(),
        };
        let Some(subscribers) = self.topics.get(topic) else {
            return Ok(0);
        };
        let mut delivered = 0;
        for subscriber in subscribers {
            if let Ok(()) = subscriber.tx.try_send(event.clone()) {
                delivered += 1;
            }
        }
        Ok(delivered)
    }
}
