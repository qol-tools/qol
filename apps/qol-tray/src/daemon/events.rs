use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::broadcast;

use super::DaemonEvent;

const CHANNEL_CAPACITY: usize = 64;

pub struct EventBus {
    tx: broadcast::Sender<DaemonEvent>,
    plugins_revision: AtomicU64,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx,
            plugins_revision: AtomicU64::new(0),
        }
    }

    pub fn send(&self, event: DaemonEvent) {
        let _ = self.tx.send(event);
    }

    pub fn plugins_revision(&self) -> u64 {
        self.plugins_revision.load(Ordering::SeqCst)
    }

    pub fn send_plugins_changed(&self) -> u64 {
        let revision = self.plugins_revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.send(DaemonEvent::PluginsChanged { revision });
        revision
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_subscriber_receives_event() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.send_plugins_changed();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, DaemonEvent::PluginsChanged { revision: 1 }));
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let mut rx3 = bus.subscribe();

        bus.send_plugins_changed();

        for rx in [&mut rx1, &mut rx2, &mut rx3] {
            let event = rx.recv().await.unwrap();
            assert!(matches!(event, DaemonEvent::PluginsChanged { revision: 1 }));
        }
    }

    #[tokio::test]
    async fn late_subscriber_misses_earlier_events() {
        let bus = EventBus::new();

        bus.send_plugins_changed();

        let mut rx = bus.subscribe();
        bus.send_plugins_changed();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, DaemonEvent::PluginsChanged { revision: 2 }));
    }

    #[test]
    fn send_without_subscribers_does_not_panic() {
        let bus = EventBus::new();
        bus.send_plugins_changed();
    }

    #[test]
    fn subscriber_count_tracks_live_receivers() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        let rx = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        let rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        drop(rx);
        assert_eq!(bus.subscriber_count(), 1);
        drop(rx2);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn plugins_changed_increments_revision() {
        let bus = EventBus::new();
        assert_eq!(bus.plugins_revision(), 0);
        assert_eq!(bus.send_plugins_changed(), 1);
        assert_eq!(bus.send_plugins_changed(), 2);
        assert_eq!(bus.plugins_revision(), 2);
    }
}

#[cfg(all(test, feature = "dev"))]
mod dev_tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_events_in_order() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.send(DaemonEvent::DiscoveryStarted);
        bus.send(DaemonEvent::DiscoveryComplete { plugins: vec![] });
        bus.send_plugins_changed();

        assert!(matches!(
            rx.recv().await.unwrap(),
            DaemonEvent::DiscoveryStarted
        ));
        assert!(matches!(
            rx.recv().await.unwrap(),
            DaemonEvent::DiscoveryComplete { .. }
        ));
        assert!(matches!(
            rx.recv().await.unwrap(),
            DaemonEvent::PluginsChanged { revision: 1 }
        ));
    }
}
