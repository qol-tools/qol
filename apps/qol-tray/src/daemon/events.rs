use tokio::sync::broadcast;

use super::DaemonEvent;

const CHANNEL_CAPACITY: usize = 64;

pub struct EventBus {
    tx: broadcast::Sender<DaemonEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    pub fn send(&self, event: DaemonEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
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

        bus.send(DaemonEvent::PluginsChanged);

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, DaemonEvent::PluginsChanged));
    }

    #[tokio::test]
    async fn multiple_subscribers_receive_same_event() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let mut rx3 = bus.subscribe();

        bus.send(DaemonEvent::PluginsChanged);

        for rx in [&mut rx1, &mut rx2, &mut rx3] {
            let event = rx.recv().await.unwrap();
            assert!(matches!(event, DaemonEvent::PluginsChanged));
        }
    }

    #[tokio::test]
    async fn late_subscriber_misses_earlier_events() {
        let bus = EventBus::new();

        bus.send(DaemonEvent::PluginsChanged);

        let mut rx = bus.subscribe();
        bus.send(DaemonEvent::PluginsChanged);

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, DaemonEvent::PluginsChanged));
    }

    #[test]
    fn send_without_subscribers_does_not_panic() {
        let bus = EventBus::new();
        bus.send(DaemonEvent::PluginsChanged);
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
        bus.send(DaemonEvent::PluginsChanged);

        assert!(matches!(rx.recv().await.unwrap(), DaemonEvent::DiscoveryStarted));
        assert!(matches!(rx.recv().await.unwrap(), DaemonEvent::DiscoveryComplete { .. }));
        assert!(matches!(rx.recv().await.unwrap(), DaemonEvent::PluginsChanged));
    }
}
