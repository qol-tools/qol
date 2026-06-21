use tokio::sync::broadcast;

use super::ConfigKind;

const CHANNEL_CAPACITY: usize = 64;

pub struct ConfigBus {
    tx: broadcast::Sender<ConfigKind>,
}

impl ConfigBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    pub fn config_changed(&self, kind: ConfigKind) {
        let _ = self.tx.send(kind);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConfigKind> {
        self.tx.subscribe()
    }
}

impl Default for ConfigBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscriber_receives_config_kind() {
        let bus = ConfigBus::new();
        let mut rx = bus.subscribe();
        bus.config_changed(ConfigKind::Hotkeys);
        assert_eq!(rx.recv().await.unwrap(), ConfigKind::Hotkeys);
    }

    #[tokio::test]
    async fn late_subscriber_misses_earlier_signals() {
        let bus = ConfigBus::new();
        bus.config_changed(ConfigKind::Plugins);
        let mut rx = bus.subscribe();
        bus.config_changed(ConfigKind::Profile);
        assert_eq!(rx.recv().await.unwrap(), ConfigKind::Profile);
    }

    #[test]
    fn config_changed_without_subscribers_does_not_panic() {
        let bus = ConfigBus::new();
        bus.config_changed(ConfigKind::Shortcuts);
    }
}
